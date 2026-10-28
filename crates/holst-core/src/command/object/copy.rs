//! `object cp`: copies of objects on their board or on another.

use std::collections::BTreeSet;

use itertools::Itertools as _;

use crate::{
    command::{
        board::show::{Loading, account_cache, board_id},
        board_load::CacheUse,
        object::{
            CopyArgs,
            change::{
                Guard,
                Sending,
                load_board,
                named,
                refuse_withheld,
                reported,
                send,
            },
        },
        table::not_value,
    },
    domain::{
        area::scene::Scene,
        board::{Object, ObjectKind, detail::TagNames, objects as board_objects},
        change::{
            receipt::ChangeReport,
            value::{BoardChange, Pasted},
        },
        geometry::{Point, Rectangle, Size},
        lookup::rules::find_id,
        placement::free_spot_among,
    },
    effect::{
        board_write::WriteInterpreter,
        confirmation::ALWAYS,
        settings::CacheSettings,
    },
    error::{Result, board_change::BoardChangeError},
    remote::{
        client::Client,
        socket::write::{connector, interpreter},
    },
    storage::cache::Cache,
};

const COMMAND: &str = "object cp";

/// Copies the objects with what a frame holds and the arrows between them,
/// onto their board or the one `--to` names, as one write.
///
/// # Errors
/// No object, an object that is not found or cannot be copied, an arrow
/// stuck to an object left out, an image or a file for another board, a
/// corner that is not valid, a board that is not found or not editable, a
/// source board that keeps copying out of it to a higher right than the
/// caller holds, and errors of the requests and of the write.
pub async fn cp(
    client: &Client,
    cache: &CacheSettings,
    args: &CopyArgs,
) -> Result<ChangeReport> {
    let cache_use = CacheUse::Use;
    let account = account_cache(client, cache, cache_use).await?;
    copied(
        client,
        Loading {
            connector: connector(),
            cache: account.as_ref().map(Cache::as_port),
            cache_use,
        },
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn copied<I: WriteInterpreter + ?Sized>(
    client: &Client,
    loading: Loading<'_>,
    interpreter: &I,
    args: &CopyArgs,
) -> Result<ChangeReport> {
    let references = named(&args.board, &args.object, COMMAND)?;
    let workspace = args.workspace.as_deref();
    let source = board_id(client, &args.board, workspace, loading.cache_use).await?;
    let target = match args.to.as_deref() {
        Some(to) => board_id(client, to, workspace, loading.cache_use).await?,
        None => source.clone(),
    };
    let loaded = load_board(client, &loading, &source).await?;
    let objects = board_objects(&loaded.document);
    let chosen = select(&objects, &references)?;
    let elsewhere = target != source;
    refuse_files(&chosen, elsewhere)?;
    if elsewhere {
        // As the web client does before it lets objects go to another board.
        let copying: Guard = ("who_can_copy_content", |needs, holds| {
            BoardChangeError::CopyWithheld { needs, holds }
        });
        refuse_withheld(client, &source, copying).await?;
    }
    let beside = if elsewhere {
        board_objects(&load_board(client, &loading, &target).await?.document)
    } else {
        objects
    };
    let change = BoardChange::Paste(pasted(
        chosen,
        &TagNames::of(&loaded.document),
        &beside,
        args.at.as_deref(),
    )?);
    let written = send(
        client,
        loading.cache,
        interpreter,
        Sending {
            board: &target,
            change: &change,
            release: ALWAYS,
        },
    )
    .await?;
    Ok(reported(client, &target, &written))
}

/// The copies of the chosen objects, moved to the corner `at` gives, else to a
/// free one among the objects `beside` on the board they go to.
fn pasted(
    chosen: Chosen,
    tags: &TagNames,
    beside: &[Object],
    at: Option<&str>,
) -> Result<Pasted> {
    let corner = corner(at, beside, &chosen.area)?;
    Ok(Pasted {
        tags: tags.titles_in(&chosen.objects),
        shift: Point::new(corner.x() - chosen.area.x, corner.y() - chosen.area.y)?,
        objects: chosen.objects,
    })
}

/// What a copy takes: its objects in put-down order, and their box.
struct Chosen {
    objects: Vec<Object>,
    area: Rectangle,
}

/// The objects named, what the frames among them hold and the arrows between
/// them; frames go first, the topmost first, then the rest from the bottom.
fn select(objects: &[Object], references: &[String]) -> Result<Chosen> {
    let asked = references
        .iter()
        .map(|reference| {
            find_id(objects.iter().map(|object| object.id.as_str()), reference)
        })
        .collect::<Result<Vec<String>>>()?;
    let held = held_with(objects, &asked);
    let inside = |arrow: &Object| ends(arrow).iter().all(|end| held.contains(*end));
    objects
        .iter()
        .find(|object| {
            object.kind == ObjectKind::Arrow && asked.contains(&object.id) && !inside(object)
        })
        .map_or(Ok(()), |arrow| {
            Err(not_value(
                COMMAND,
                "object",
                &format!(
                    "an arrow with its ends: {} is stuck to an object left out of the copy",
                    arrow.id
                ),
            ))
        })?;
    let chosen: Vec<&Object> = objects
        .iter()
        .filter(|object| {
            held.contains(&object.id)
                || (object.kind == ObjectKind::Arrow
                    && !ends(object).is_empty()
                    && inside(object))
        })
        .collect();
    chosen
        .iter()
        .find(|object| !copyable(&object.kind))
        .map_or(Ok(()), |object| {
            Err(not_value(
                COMMAND,
                "object",
                &format!(
                    "an object that can be copied: {} is {}",
                    object.id,
                    object.kind.short_name()
                ),
            ))
        })?;
    let scene = Scene::new(objects);
    let area = Rectangle::bounding(
        chosen
            .iter()
            .filter(|object| object.kind != ObjectKind::Arrow)
            .filter_map(|object| scene.rectangle_of(object)),
    )
    .unwrap_or(Rectangle::new(0.0, 0.0, 0.0, 0.0));
    let (frames, others): (Vec<&Object>, Vec<&Object>) = chosen
        .into_iter()
        .partition(|object| object.kind == ObjectKind::Frame);
    let in_order = frames
        .into_iter()
        .sorted_by(|a, b| b.z_index.total_cmp(&a.z_index))
        .chain(
            others
                .into_iter()
                .sorted_by(|a, b| a.z_index.total_cmp(&b.z_index)),
        )
        .cloned()
        .collect();
    Ok(Chosen {
        objects: in_order,
        area,
    })
}

/// The ids of the objects an arrow is stuck to.
fn ends(arrow: &Object) -> Vec<&str> {
    arrow
        .ends
        .iter()
        .flat_map(|ends| [ends.start.as_deref(), ends.end.as_deref()])
        .flatten()
        .collect()
}

/// `asked` and everything inside them, down the parents.
fn held_with(objects: &[Object], asked: &[String]) -> BTreeSet<String> {
    std::iter::successors(
        Some(asked.iter().cloned().collect::<BTreeSet<_>>()),
        |held| {
            let grown: BTreeSet<String> = objects
                .iter()
                .filter(|object| {
                    object.parent.as_ref().is_some_and(|p| held.contains(p))
                })
                .map(|object| object.id.clone())
                .chain(held.iter().cloned())
                .collect();
            (grown.len() > held.len()).then_some(grown)
        },
    )
    .last()
    .unwrap_or_default()
}

/// Kinds whose whole content is in their own fields and text.
const fn copyable(kind: &ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Sticker
            | ObjectKind::Text
            | ObjectKind::Shape
            | ObjectKind::Document
            | ObjectKind::Frame
            | ObjectKind::Arrow
            | ObjectKind::Image
            | ObjectKind::File
            | ObjectKind::Stamp
            | ObjectKind::Group
            | ObjectKind::Drawing
            | ObjectKind::Link
    )
}

/// An image or a file is not copied to another board: its file would have to
/// be uploaded again.
fn refuse_files(chosen: &Chosen, elsewhere: bool) -> Result<()> {
    chosen
        .objects
        .iter()
        .find(|object| {
            elsewhere && matches!(object.kind, ObjectKind::Image | ObjectKind::File)
        })
        .map_or(Ok(()), |object| {
            Err(not_value(
                COMMAND,
                "to",
                &format!(
                    "the board of the objects while {} {} is among them",
                    object.kind.short_name(),
                    object.id
                ),
            ))
        })
}

/// The corner the copies' box goes to: the one given, else a free one.
fn corner(at: Option<&str>, on_board: &[Object], area: &Rectangle) -> Result<Point> {
    let given = at.map(str::parse::<Point>).transpose()?;
    given.map_or_else(
        || {
            let size = Size::new(area.width, area.height).unwrap_or(Size::STICKER);
            free_spot_among(&Scene::new(on_board), None, size, &[])
        },
        Ok,
    )
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;

    use crate::{
        Result,
        command::{
            board::show::Loading,
            board_load::CacheUse,
            fixture::{
                ARROW,
                FRAME,
                IMAGE,
                OTHER,
                STICKER,
                TEXT,
                arguments,
                board_of,
                listed,
                objects,
                signed_in_hud,
            },
            object::{
                CopyArgs,
                copy::{copied, pasted, select},
            },
        },
        domain::{
            board::{ObjectKind, detail::TagNames, object_of},
            change::value::ObjectChangeKind,
        },
        remote::socket::{fake::BOARD, write::WriteOver},
    };

    fn chosen_ids(references: &[&str]) -> Result<Vec<String>> {
        let references: Vec<String> =
            references.iter().map(|id| (*id).to_owned()).collect();
        Ok(select(&listed(), &references)?
            .objects
            .into_iter()
            .map(|object| object.id)
            .collect())
    }

    fn refusal(references: &[&str]) -> Option<String> {
        let references: Vec<String> =
            references.iter().map(|id| (*id).to_owned()).collect();
        select(&listed(), &references)
            .err()
            .map(|error| error.to_string())
    }

    #[test]
    fn a_frame_goes_with_what_it_holds() -> Result<()> {
        let chosen = select(&listed(), &[FRAME.to_owned()])?;
        assert_eq!(
            (
                chosen
                    .objects
                    .into_iter()
                    .map(|object| object.id)
                    .collect::<Vec<_>>(),
                (
                    chosen.area.x,
                    chosen.area.y,
                    chosen.area.width,
                    chosen.area.height
                )
            ),
            (
                vec![FRAME.to_owned(), STICKER.to_owned()],
                (0.0, 0.0, 800.0, 600.0)
            )
        );
        Ok(())
    }

    #[test]
    fn arrows_between_the_copied_objects_come_along() -> Result<()> {
        assert_eq!(
            chosen_ids(&[TEXT, STICKER])?,
            [STICKER, TEXT, ARROW].map(str::to_owned)
        );
        Ok(())
    }

    #[test]
    fn frames_go_first_and_the_topmost_of_them_first() -> Result<()> {
        assert_eq!(
            chosen_ids(&[IMAGE, FRAME])?,
            [FRAME, STICKER, IMAGE].map(str::to_owned)
        );
        Ok(())
    }

    #[test]
    fn an_arrow_stuck_outside_the_copy_is_refused() {
        assert_eq!(
            refusal(&[ARROW]).as_deref(),
            Some(
                "object of `object cp` must be an arrow with its ends: a1 is stuck to an object left out of the copy"
            )
        );
    }

    #[test]
    fn a_kind_with_side_stores_is_refused() {
        let table = object_of(json!({
            "id": "x1", "type": "table", "position": {"x": 0, "y": 0}, "zIndex": 1
        }));
        let refused =
            select(&table.into_iter().collect::<Vec<_>>(), &["x1".to_owned()])
                .err()
                .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some(
                "object of `object cp` must be an object that can be copied: x1 is table"
            )
        );
    }

    async fn copies(
        given: serde_json::Value,
    ) -> Result<(String, Vec<(ObjectKind, ObjectChangeKind)>)> {
        let (_, client) = signed_in_hud()?;
        let args: CopyArgs = arguments(given)?;
        let report = copied(
            &client,
            Loading {
                connector: &board_of(&objects()),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &WriteOver::reporter(&board_of(&objects())),
            &args,
        )
        .await?;
        Ok((
            report.board,
            report
                .objects
                .into_iter()
                .map(|object| (object.kind, object.change))
                .collect(),
        ))
    }

    #[tokio::test]
    async fn a_copy_goes_to_a_free_corner_of_the_same_board() -> Result<()> {
        let done = copies(json!({"object": [STICKER]})).await?;
        assert_eq!(
            done,
            (
                BOARD.to_owned(),
                vec![(ObjectKind::Sticker, ObjectChangeKind::Created)]
            )
        );
        Ok(())
    }

    #[test]
    fn a_copy_for_another_board_moves_to_the_corner_given() -> Result<()> {
        let beside = objects()
            .into_iter()
            .nth(2)
            .into_iter()
            .filter_map(object_of)
            .collect::<Vec<_>>();
        let made = pasted(
            select(&listed(), &[STICKER.to_owned(), TEXT.to_owned()])?,
            &TagNames::default(),
            &beside,
            Some("100,200"),
        )?;
        assert_eq!(
            (
                made.objects
                    .into_iter()
                    .map(|object| object.id)
                    .collect::<Vec<_>>(),
                (made.shift.x(), made.shift.y())
            ),
            (
                [STICKER, TEXT, ARROW].map(str::to_owned).to_vec(),
                (76.0, 200.0)
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_image_stays_off_another_board() -> Result<()> {
        let refused = copies(json!({"object": [IMAGE], "to": OTHER}))
            .await
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some(
                "to of `object cp` must be the board of the objects while image i1 is among them"
            )
        );
        Ok(())
    }
}
