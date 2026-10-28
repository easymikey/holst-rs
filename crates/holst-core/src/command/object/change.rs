//! `object edit` and `object rm`: changes to objects already on a board.

use itertools::Itertools as _;
use serde::Serialize;

use crate::{
    command::{
        account,
        board::show::{Loading, account_cache, board_id},
        board_load::{self, CacheUse, RemoteBoard},
        board_write::{WriteRequest, write_cached},
        object::{EditArgs, EditItem, RemoveArgs},
        table::{Confirmation, not_value},
    },
    domain::{
        board::{LoadedBoard, arrow_look::ARROW_WIDTHS},
        change::{
            receipt::ChangeReport,
            value::{
                ArrowEdit,
                Backdrop,
                BoardChange,
                ObjectEdit,
                ObjectPatch,
                ShapeEdit,
                TextLook,
            },
        },
        id,
        rich_text::{Mark, TextFormat},
    },
    effect::{
        board_write::WriteInterpreter,
        board_write_plan::WrittenChange,
        cache::CachePort,
        confirmation::{
            ALWAYS,
            NEVER,
            ReleaseCheck,
            UNLESS_FRAME_CONTENT,
            unless_confirmed,
        },
        settings::CacheSettings,
    },
    error::{
        Error,
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
    },
    remote::{client::Client, socket::write::interpreter},
    storage::cache::Cache,
};

const EDIT: &str = "object edit";
const REMOVE: &str = "object rm";
/// BUNDLE: the outline slider goes to 10 (main.a33651a4.js 183120-183140).
const MAX_STROKE_WIDTH: f64 = 10.0;
/// BUNDLE: the opacity slider starts at 0.1 (main.a33651a4.js 182790).
const MIN_OPACITY: f64 = 0.1;

/// A removal: what it took, or would take, and what it waits for.
#[derive(Debug, Serialize)]
pub struct Removed {
    #[serde(flatten)]
    pub report: ChangeReport,
    /// The risk of a removal held back for want of its confirmation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// The refusal of that removal, to word for a surface.
    #[serde(skip)]
    pub refusal: Option<Unconfirmed>,
}

/// The objects a call names: the one its board link names, then each
/// `--object`, once each.
pub(super) fn named(
    board: &str,
    objects: &[String],
    command: &str,
) -> Result<Vec<String>> {
    let named: Vec<String> = id::object_in(board)
        .map(|linked| linked.as_str().to_owned())
        .into_iter()
        .chain(objects.iter().cloned())
        .unique()
        .collect();
    if named.is_empty() {
        Err(Error::from(InputError::NoArgument {
            command: command.to_owned(),
            name: "object",
        }))
    } else {
        Ok(named)
    }
}

/// What a write sends to a board and how it is let go.
pub(super) struct Sending<'a> {
    pub(super) board: &'a str,
    pub(super) change: &'a BoardChange,
    pub(super) release: ReleaseCheck,
}

/// Writes the change as the logged-in account.
pub(super) async fn send<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    sending: Sending<'_>,
) -> Result<WrittenChange> {
    let user = account::account_id(client).await?;
    let changed = write_cached(
        client,
        interpreter,
        WriteRequest {
            board: sending.board,
            change: sending.change,
            send_if: sending.release,
            account_id: &user,
            kept: None,
        },
        cache,
    )
    .await
    .map_err(Error::from)?;
    Ok(changed.change.written)
}

/// The board loaded as `loading` says.
pub(super) async fn load_board(
    client: &Client,
    loading: &Loading<'_>,
    board: &str,
) -> Result<LoadedBoard> {
    let (loaded, _) = board_load::load(
        RemoteBoard {
            client,
            connector: loading.connector,
            board,
        },
        loading.cache,
        loading.cache_use,
    )
    .await?;
    Ok(loaded)
}

/// What the write did to the board's objects, each with its link.
pub(super) fn reported(
    client: &Client,
    board: &str,
    written: &WrittenChange,
) -> ChangeReport {
    let base = client.base_url();
    written.report(board, |object| base.object_link(board, object))
}

/// Changes text, corner, size, color, rotation, order or tags of objects, in
/// one write.
///
/// # Errors
/// An item that is not valid, an object that is not found or has no such
/// field, nothing to change, a board that is not found or not editable, and
/// errors of the requests and of the write.
pub async fn edit(
    client: &Client,
    cache: &CacheSettings,
    args: &EditArgs,
) -> Result<ChangeReport> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    edited(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn edited<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &EditArgs,
) -> Result<ChangeReport> {
    let change = BoardChange::EditAll(read_edits(args)?);
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        CacheUse::Use,
    )
    .await?;
    let written = send(
        client,
        cache,
        interpreter,
        Sending {
            board: &board,
            change: &change,
            release: ALWAYS,
        },
    )
    .await?;
    Ok(reported(client, &board, &written))
}

/// The edits: those of `--items`, or the one the flags describe for each
/// object named.
fn read_edits(args: &EditArgs) -> Result<Vec<ObjectEdit>> {
    let linked = id::object_in(&args.board).map(|linked| linked.as_str().to_owned());
    if args.items.0.is_empty() {
        let patch = patch_of(
            &EditItem {
                object: None,
                text: args.text.clone(),
                at: args.at.clone(),
                size: args.size.clone(),
                color: args.color.clone(),
                shape: args.shape,
                stroke: args.stroke.clone(),
                stroke_width: args.stroke_width,
                stroke_style: args.stroke_style,
                radius: args.radius,
                opacity: args.opacity,
                mark: args.mark.clone(),
                text_color: args.text_color.clone(),
                text_bg: args.text_bg.clone(),
                line: args.line,
                heads: args.heads.clone(),
                bend: args.bend.clone(),
                from: args.from.clone(),
                to: args.to.clone(),
                label: args.label.clone(),
                rotate: args.rotate,
                order: None,
                tag: Vec::new(),
            },
            args.text_format.unwrap_or_default(),
        )?;
        named(&args.board, &args.object, EDIT)?
            .into_iter()
            .map(|object| {
                Ok(ObjectEdit {
                    object,
                    patch: patch.clone(),
                    stacking: args.order,
                    tags: tags_of(&args.tag),
                })
            })
            .collect()
    } else {
        refuse_with_items(args)?;
        args.items
            .0
            .iter()
            .enumerate()
            .map(|(slot, item)| {
                item_edit(
                    item,
                    linked.as_deref(),
                    slot,
                    args.text_format.unwrap_or_default(),
                )
            })
            .collect()
    }
}

/// What the flags of one call cannot say beside `--items`.
fn refuse_with_items(args: &EditArgs) -> Result<()> {
    [
        ("object", !args.object.is_empty()),
        ("text", args.text.is_some()),
        ("at", args.at.is_some()),
        ("size", args.size.is_some()),
        ("color", args.color.is_some()),
        ("shape", args.shape.is_some()),
        ("stroke", args.stroke.is_some()),
        ("stroke-width", args.stroke_width.is_some()),
        ("stroke-style", args.stroke_style.is_some()),
        ("radius", args.radius.is_some()),
        ("opacity", args.opacity.is_some()),
        ("mark", args.mark.is_some()),
        ("text_color", args.text_color.is_some()),
        ("text_bg", args.text_bg.is_some()),
        ("line", args.line.is_some()),
        ("heads", args.heads.is_some()),
        ("bend", args.bend.is_some()),
        ("from", args.from.is_some()),
        ("to", args.to.is_some()),
        ("label", !args.label.is_empty()),
        ("rotate", args.rotate.is_some()),
        ("order", args.order.is_some()),
        ("tag", !args.tag.is_empty()),
    ]
    .into_iter()
    .find_map(|(name, given)| given.then_some(name))
    .map_or(Ok(()), |name| {
        Err(not_value(EDIT, name, "left out with --items"))
    })
}

/// The edit one item asks for; an item with no object takes the linked one.
fn item_edit(
    item: &EditItem,
    linked: Option<&str>,
    slot: usize,
    format: TextFormat,
) -> Result<ObjectEdit> {
    let object = item.object.as_deref().or(linked).ok_or_else(|| {
        not_value(EDIT, "object", &format!("given, in item {}", slot + 1))
    })?;
    Ok(ObjectEdit {
        object: object.to_owned(),
        patch: patch_of(item, format)?,
        stacking: item.order,
        tags: tags_of(&item.tag),
    })
}

fn patch_of(item: &EditItem, format: TextFormat) -> Result<ObjectPatch> {
    // The kind decides later: a shape takes 0 to 10, an arrow its nine widths.
    let stroke_width = item
        .stroke_width
        .map(|width| {
            let shape = width.fract() == 0.0 && (0.0..=MAX_STROKE_WIDTH).contains(&width);
            (shape || ARROW_WIDTHS.contains(&width)).then_some(width).ok_or_else(|| {
                not_value(
                    EDIT,
                    "stroke-width",
                    "a whole number 0 to 10, or for an arrow 1, 1.5, 2, 4, 5, 6, 9, 13 or 17",
                )
            })
        })
        .transpose()?;
    let opacity = item
        .opacity
        .map(|opacity| {
            (MIN_OPACITY..=1.0)
                .contains(&opacity)
                .then_some(opacity)
                .ok_or_else(|| not_value(EDIT, "opacity", "a number 0.1 to 1"))
        })
        .transpose()?;
    Ok(ObjectPatch {
        text: item.text.clone(),
        format,
        look: look_of(item)?,
        at: item.at.as_deref().map(str::parse).transpose()?,
        size: item.size.as_deref().map(str::parse).transpose()?,
        color: item.color.as_deref().map(str::parse).transpose()?,
        rotation: item
            .rotate
            .map(|degrees| {
                degrees
                    .is_finite()
                    .then_some(degrees)
                    .ok_or_else(|| not_value(EDIT, "rotate", "a number of degrees"))
            })
            .transpose()?,
        shape: ShapeEdit {
            kind: item.shape,
            stroke: item.stroke.as_deref().map(str::parse).transpose()?,
            stroke_width,
            stroke_style: item.stroke_style,
            radius: item.radius,
            opacity,
        },
        arrow: ArrowEdit {
            line: item.line,
            heads: item
                .heads
                .as_deref()
                .map(str::parse)
                .transpose()?
                .unwrap_or_default(),
            bend: item.bend.as_deref().map(str::parse).transpose()?,
            from: item.from.as_deref().map(str::parse).transpose()?,
            to: item.to.as_deref().map(str::parse).transpose()?,
            labels: item
                .label
                .iter()
                .map(|raw| raw.parse())
                .collect::<Result<_>>()?,
        },
    })
}

/// The marks and colors of the whole text the flags ask for.
fn look_of(item: &EditItem) -> Result<TextLook> {
    Ok(TextLook {
        marks: item.mark.as_deref().map(marks_of).transpose()?,
        color: item.text_color.as_deref().map(str::parse).transpose()?,
        background: item
            .text_bg
            .as_deref()
            .map(|raw| match raw.trim() {
                "none" => Ok(Backdrop::Clear),
                token => token.parse().map(Backdrop::Color),
            })
            .transpose()?,
    })
}

/// The marks of a comma-apart list; `none` is no marks.
fn marks_of(raw: &str) -> Result<Vec<Mark>> {
    let listed = raw.split(',').map(str::trim).filter(|name| *name != "none");
    listed
        .filter(|name| !name.is_empty())
        .map(|name| {
            Mark::of(name).ok_or_else(|| {
                not_value(
                    EDIT,
                    "mark",
                    "bold, italic, underline, strikethrough, code or none",
                )
            })
        })
        .collect()
}

/// The tags asked for by title; a blank one asks for none.
fn tags_of(tags: &[String]) -> Option<Vec<String>> {
    (!tags.is_empty()).then(|| {
        tags.iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect()
    })
}

/// Removes objects with what a frame holds and the arrows on them. Several
/// objects, or a frame with its content, stay unremoved without the
/// confirmation.
///
/// # Errors
/// No object, an object that is not found, a board that is not found or not
/// editable, and errors of the requests and of the write.
pub async fn rm(
    client: &Client,
    cache: &CacheSettings,
    args: &RemoveArgs,
) -> Result<Removed> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    removed(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn removed<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &RemoveArgs,
) -> Result<Removed> {
    let objects = named(&args.board, &args.object, REMOVE)?;
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        CacheUse::Use,
    )
    .await?;
    let confirmed = args.confirm == Confirmation::Given;
    let count = objects.len();
    let several = count > 1;
    let release = match (confirmed, several) {
        (true, _) => ALWAYS,
        (false, true) => NEVER,
        (false, false) => UNLESS_FRAME_CONTENT,
    };
    let change = BoardChange::RemoveAll(objects);
    let written = send(
        client,
        cache,
        interpreter,
        Sending {
            board: &board,
            change: &change,
            release,
        },
    )
    .await?;
    let unconfirmed = if several {
        Some(Unconfirmed::new(
            format!("removing {count} objects"),
            ChangeRisk::Irreversible,
        ))
    } else {
        written.held_unconfirmed()
    };
    let refusal = unless_confirmed(unconfirmed, confirmed, args.dry_run).err();
    let held = refusal
        .as_ref()
        .map(|needs| format!("{} {}", needs.change, needs.risk));
    Ok(Removed {
        report: reported(client, &board, &written),
        held,
        refusal,
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;

    use crate::{
        Result,
        command::{
            fixture::{ARROW, FRAME, STICKER, TEXT, arguments, board_of, objects},
            object::{
                EditArgs,
                RemoveArgs,
                change::{edited, removed},
            },
        },
        domain::change::receipt::Delivery,
        remote::socket::{
            fake::{BOARD, signed_in},
            write::WriteOver,
        },
    };

    fn link(object: &str) -> String {
        format!("https://app.holst.so/board/{BOARD}?objectId={object}")
    }

    async fn edit_fields(
        given: serde_json::Value,
    ) -> Result<Vec<(String, Vec<&'static str>)>> {
        let (_, client) = signed_in([])?;
        let args: EditArgs = arguments(given)?;
        let report = edited(
            &client,
            None,
            &WriteOver::reporter(&board_of(&objects())),
            &args,
        )
        .await?;
        Ok(report
            .objects
            .into_iter()
            .map(|object| (object.id, object.fields))
            .collect())
    }

    #[tokio::test]
    async fn an_edit_changes_the_linked_object_and_each_object_flag() -> Result<()> {
        let changed = edit_fields(
            json!({"board": link(STICKER), "object": [TEXT], "text": "new"}),
        )
        .await?;
        assert_eq!(
            changed,
            vec![
                (STICKER.to_owned(), vec!["text"]),
                (TEXT.to_owned(), vec!["text"])
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_item_without_an_object_edits_the_linked_one() -> Result<()> {
        let changed = edit_fields(
            json!({"board": link(STICKER), "items": [{"color": "blue3", "order": "front"}]}),
        )
        .await?;
        assert_eq!(changed, vec![(STICKER.to_owned(), vec!["color", "order"])]);
        Ok(())
    }

    #[tokio::test]
    async fn a_tag_and_an_order_are_edits_of_a_sticker() -> Result<()> {
        let changed = edit_fields(
            json!({"object": [STICKER], "tag": ["urgent"], "order": "back"}),
        )
        .await?;
        assert_eq!(changed, vec![(STICKER.to_owned(), vec!["order", "tags"])]);
        Ok(())
    }

    #[tokio::test]
    async fn the_flags_of_one_call_are_refused_beside_items() -> Result<()> {
        let refused = edit_fields(
            json!({"text": "x", "items": [{"object": STICKER, "color": "blue3"}]}),
        )
        .await
        .err()
        .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("text of `object edit` must be left out with --items")
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_edit_that_asks_for_nothing_is_refused() -> Result<()> {
        let refused = edit_fields(json!({"object": [STICKER]}))
            .await
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some(
                "nothing to change: give a text, a position, a size, a color, an order or tags"
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_edit_without_an_object_is_refused() -> Result<()> {
        let refused = edit_fields(json!({"text": "x"}))
            .await
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("object is required by `object edit`")
        );
        Ok(())
    }

    async fn remove(
        given: serde_json::Value,
    ) -> Result<(Vec<String>, Delivery, Option<String>)> {
        let (_, client) = signed_in([])?;
        let args: RemoveArgs = arguments(given)?;
        let socket = board_of(&objects());
        let done = removed(&client, None, &WriteOver::sender(&socket), &args).await?;
        Ok((
            done.report
                .objects
                .into_iter()
                .map(|object| object.id)
                .collect(),
            done.report.receipt.delivery,
            done.held,
        ))
    }

    #[tokio::test]
    async fn one_object_goes_with_its_arrows_without_a_confirmation() -> Result<()> {
        let done = remove(json!({"object": [STICKER]})).await?;
        assert_eq!(
            done,
            (
                vec![STICKER.to_owned(), ARROW.to_owned()],
                Delivery::Accepted,
                None
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn several_objects_wait_for_the_confirmation() -> Result<()> {
        let done = remove(json!({"object": [STICKER, TEXT]})).await?;
        assert_eq!(
            done,
            (
                vec![STICKER.to_owned(), ARROW.to_owned(), TEXT.to_owned()],
                Delivery::Built,
                Some("removing 2 objects cannot be undone".to_owned())
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn several_confirmed_objects_go() -> Result<()> {
        let done = remove(json!({"object": [STICKER, TEXT], "confirm": true})).await?;
        assert_eq!(done.1, Delivery::Accepted);
        assert_eq!(done.2, None);
        Ok(())
    }

    #[tokio::test]
    async fn a_frame_with_content_waits_for_the_confirmation() -> Result<()> {
        let done = remove(json!({"object": [FRAME]})).await?;
        assert_eq!(
            done,
            (
                vec![FRAME.to_owned(), ARROW.to_owned(), STICKER.to_owned()],
                Delivery::Built,
                Some(
                    "removing the frame and 2 objects with it cannot be undone"
                        .to_owned()
                )
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_dry_run_asks_for_no_confirmation() -> Result<()> {
        let done = remove(json!({"object": [STICKER, TEXT], "dry_run": true})).await?;
        assert_eq!((done.1, done.2), (Delivery::Built, None));
        Ok(())
    }
}
