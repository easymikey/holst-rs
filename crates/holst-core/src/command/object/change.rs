//! `object edit` and `object rm`: changes to objects already on a board.

use std::num::NonZeroU32;

use itertools::Itertools as _;
use serde::{Serialize, Serializer};

use crate::{
    command::{
        board::show::{Loading, board_id, through_cache},
        board_load::{self, CacheUse, RemoteBoard},
        board_write::{BoardWrite, Guard, change_board, refuse_withheld},
        change::{ChangeReport as Outcome, gate},
        object::{EditArgs, EditFields, EditItem, MarkChoice, RemoveArgs},
        table::{ArgumentGroup as _, Confirmation, not_value},
    },
    domain::{
        board::{LoadedBoard, arrow_look::ARROW_WIDTHS},
        change::{
            kind_edit::{KindEdit, MOST_KINDS, StickerEdit, TextSize},
            receipt::{ChangeReport, Delivery},
            value::{
                ArrowEdit,
                Backdrop,
                BoardChange,
                FrameEdit,
                LinkTo,
                ObjectEdit,
                ObjectPatch,
                Reparent,
                ShapeEdit,
                TextLook,
                TextWidth,
                Tick,
            },
        },
        id,
        rich_text::{Mark, TextFormat},
    },
    effect::{
        confirmation::{
            ALWAYS,
            NEVER,
            UNLESS_FRAME_CONTENT,
            frame_content_unconfirmed,
        },
        settings::CacheSettings,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
    },
    remote::client::Client,
};

const EDIT: &str = "object edit";
const REMOVE: &str = "object rm";
/// BUNDLE: the outline slider goes to 10 (main.a33651a4.js 183120-183140).
const MAX_STROKE_WIDTH: f64 = 10.0;
/// BUNDLE: the opacity slider starts at 0.1 (main.a33651a4.js 182790).
const MIN_OPACITY: f64 = 0.1;

/// A removal: what it took, or would take, and what it waits for.
#[derive(Debug)]
pub struct Removed {
    pub report: ChangeReport,
    /// Whether the removal went out, was a dry run, or is held.
    pub outcome: Outcome<()>,
}

/// Flat JSON: the report, its `status` the outcome's, and `needs` when held.
impl Serialize for Removed {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.report.told(&self.outcome).serialize(serializer)
    }
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

/// Changes text, corner, size, color, rotation, link, order or tags of
/// objects, in one write.
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
    through_cache(client, cache, CacheUse::Use, async |loading| {
        edited(client, loading, args).await
    })
    .await
}

async fn edited(
    client: &Client,
    loading: Loading<'_>,
    args: &EditArgs,
) -> Result<ChangeReport> {
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        CacheUse::Use,
    )
    .await?;
    let edits = read_edits(args, &client.base_url().share_link(&board))?;
    let hides = edits.iter().any(|edit| edit.patch.frame.hidden.is_some());
    change_board(
        client,
        loading,
        BoardWrite {
            board: &board,
            change: BoardChange::EditAll(edits),
            release: ALWAYS,
            guard: hides.then_some(HIDING),
            mode: args.dry_run,
        },
    )
    .await
}

/// The edits: those of `--items`, or the one the flags describe for each
/// object named.
fn read_edits(args: &EditArgs, share: &str) -> Result<Vec<ObjectEdit>> {
    let linked = id::object_in(&args.board).map(|linked| linked.as_str().to_owned());
    if args.items.0.is_empty() {
        let patch = patch_of(&args.edit, args.text_format.unwrap_or_default(), share)?;
        named(&args.board, &args.object, EDIT)?
            .into_iter()
            .map(|object| {
                Ok(ObjectEdit {
                    object,
                    patch: patch.clone(),
                    stacking: args.edit.order,
                    tags: tags_of(&args.edit.tag),
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
                    (args.text_format.unwrap_or_default(), share),
                )
            })
            .collect()
    }
}

/// What the flags of one call cannot say beside `--items`.
fn refuse_with_items(args: &EditArgs) -> Result<()> {
    (!args.object.is_empty())
        .then_some("object")
        .or_else(|| args.edit.first_given())
        .map_or(Ok(()), |name| {
            Err(not_value(EDIT, name, "left out with --items"))
        })
}

/// The edit one item asks for; an item with no object takes the linked one.
fn item_edit(
    item: &EditItem,
    linked: Option<&str>,
    slot: usize,
    (format, share): (TextFormat, &str),
) -> Result<ObjectEdit> {
    let object = item.object.as_deref().or(linked).ok_or_else(|| {
        not_value(EDIT, "object", &format!("given, in item {}", slot + 1))
    })?;
    Ok(ObjectEdit {
        object: object.to_owned(),
        patch: patch_of(&item.edit, format, share)?,
        stacking: item.edit.order,
        tags: tags_of(&item.edit.tag),
    })
}

fn patch_of(
    item: &EditFields,
    format: TextFormat,
    address: &str,
) -> Result<ObjectPatch> {
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
        scale: scale_of(item)?,
        kind_edits: kind_edits_of(item)?,
        link_to: item
            .link_to
            .as_deref()
            .map(|raw| link_of(raw, address))
            .transpose()?,
        width: width_of(item)?,
        tick: tick_of(item)?,
        shape: ShapeEdit {
            kind: item.shape,
            stroke: item.stroke_color.as_deref().map(str::parse).transpose()?,
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
        frame: FrameEdit {
            ratio: item.ratio,
            background: item.background,
            hidden: hidden_of(EDIT, Some(item.hide_content), Some(item.show_content))?,
        },
        reparent: item.frame.as_deref().map(reparent_of).transpose()?,
    })
}

/// What `--scale` says: a factor above 0, not beside a size that says the same.
fn scale_of(item: &EditFields) -> Result<Option<f64>> {
    item.scale
        .map(|scale| match (item.size.is_some(), scale) {
            (true, _) => Err(not_value(EDIT, "scale", "left out with size")),
            (false, scale) if scale.is_finite() && scale > 0.0 => Ok(scale),
            (false, _) => Err(not_value(EDIT, "scale", "a number above 0")),
        })
        .transpose()
}

/// What `--frame` says: `none` for the board, else a frame.
fn reparent_of(raw: &str) -> Result<Reparent> {
    let raw = raw.trim();
    match raw {
        "" => Err(not_value(EDIT, "frame", "a frame or none")),
        "none" => Ok(Reparent::Board),
        _ => Ok(Reparent::Frame(raw.to_owned())),
    }
}

/// Whether the flags hide the content of a frame, show it, or leave it.
pub(super) fn hidden_of(
    command: &str,
    hide: Option<bool>,
    show: Option<bool>,
) -> Result<Option<bool>> {
    match (hide == Some(true), show == Some(true)) {
        (true, true) => Err(not_value(
            command,
            "show_content",
            "left out with hide_content",
        )),
        (true, false) => Ok(Some(true)),
        (false, true) => Ok(Some(false)),
        (false, false) => Ok(None),
    }
}

/// Refuses hiding or showing frame content on `board` when its
/// `who_can_manage_frame_hide_content` stands above the caller's role.
pub(super) async fn refuse_hiding(client: &Client, board: &str) -> Result<()> {
    refuse_withheld(client, board, HIDING).await
}

/// Hiding or showing frame content needs `who_can_manage_frame_hide_content`.
const HIDING: Guard = ("who_can_manage_frame_hide_content", |needs, holds| {
    BoardChangeError::HideWithheld { needs, holds }
});

/// What `--width` says: `auto`, or a width above 0. The width is read as
/// drawn, so a size given beside it would change what it means.
fn width_of(item: &EditFields) -> Result<Option<TextWidth>> {
    item.width
        .as_deref()
        .map(|raw| match (item.size.is_some(), raw.trim()) {
            (true, _) => Err(not_value(EDIT, "width", "left out with size")),
            (false, "auto") => Ok(TextWidth::Auto),
            (false, number) => number
                .parse::<f64>()
                .ok()
                .filter(|width| width.is_finite() && *width > 0.0)
                .map(TextWidth::Fixed)
                .ok_or_else(|| not_value(EDIT, "width", "auto or a number above 0")),
        })
        .transpose()
}

/// What `--text-size`, `--show-author`, `--hide-author` and `--reaction` say.
fn kind_edits_of(item: &EditFields) -> Result<Vec<KindEdit>> {
    let text_size = item.text_size.as_deref().map(text_size_of).transpose()?;
    let author = match (item.hide_author, item.show_author) {
        (true, true) => {
            Err(not_value(EDIT, "show_author", "left out with hide_author"))
        }
        (true, false) => Ok(Some(false)),
        (false, true) => Ok(Some(true)),
        (false, false) => Ok(None),
    }?;
    Ok((text_size.is_some() || author.is_some())
        .then_some(KindEdit::Sticker(StickerEdit { text_size, author }))
        .into_iter()
        .chain(reactions_of(&item.reaction)?)
        .collect())
}

/// What `--reaction` says: the ids to leave, or `none` alone for no reaction.
fn reactions_of(given: &[String]) -> Result<Option<KindEdit>> {
    let ids: Vec<String> = given
        .iter()
        .map(|id| id.trim().to_owned())
        .unique()
        .collect();
    match ids.as_slice() {
        [] => Ok(None),
        [none] if none == "none" => Ok(Some(KindEdit::Reaction(Vec::new()))),
        many if many.iter().any(|id| id.is_empty() || id == "none") => {
            Err(not_value(EDIT, "reaction", "reaction ids, or none alone"))
        }
        many if many.len() > MOST_KINDS => {
            Err(not_value(EDIT, "reaction", "at most 8 different reactions"))
        }
        _ => Ok(Some(KindEdit::Reaction(ids))),
    }
}

/// What `--text-size` says: `auto`, or a whole number 4 to 64.
fn text_size_of(raw: &str) -> Result<TextSize> {
    match raw.trim() {
        "auto" => Ok(TextSize::Auto),
        number => number
            .parse()
            .ok()
            .filter(|points| TextSize::RANGE.contains(points))
            .map(TextSize::Points)
            .ok_or_else(|| {
                not_value(EDIT, "text-size", "auto or a whole number 4 to 64")
            }),
    }
}

/// What `--check` and `--uncheck` say: one checklist item, from 1.
fn tick_of(item: &EditFields) -> Result<Option<Tick>> {
    let asked = match (item.check, item.uncheck) {
        (Some(_), Some(_)) => Err(not_value(EDIT, "check", "left out with uncheck")),
        (Some(number), None) => Ok(Some((number, true, "check"))),
        (None, Some(number)) => Ok(Some((number, false, "uncheck"))),
        (None, None) => Ok(None),
    }?;
    asked
        .map(|(number, done, name)| {
            NonZeroU32::new(number)
                .map(|nth| Tick { item: nth, done })
                .ok_or_else(|| not_value(EDIT, name, "an item number from 1"))
        })
        .transpose()
}

/// What `--link-to` says: `none`, an address, or an object of the board.
fn link_of(raw: &str, share: &str) -> Result<LinkTo> {
    let raw = raw.trim();
    let address = raw.contains(['.', ':', '/']);
    match raw {
        "" => Err(not_value(EDIT, "link-to", "an address, an object or none")),
        "none" => Ok(LinkTo::Clear),
        _ if address => Ok(LinkTo::Url(with_scheme(raw))),
        _ => Ok(LinkTo::Object {
            reference: raw.to_owned(),
            share: share.to_owned(),
        }),
    }
}

/// BUNDLE: `http://` goes before an address with no http or https scheme
/// (main.a33651a4.js 220331).
fn with_scheme(address: &str) -> String {
    if address.starts_with("http://") || address.starts_with("https://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    }
}

/// The marks and colors of the whole text the flags ask for.
fn look_of(item: &EditFields) -> Result<TextLook> {
    Ok(TextLook {
        marks: marks_of(&item.mark),
        color: item.text_color.as_deref().map(str::parse).transpose()?,
        background: item
            .text_background
            .as_deref()
            .map(|raw| match raw.trim() {
                "none" => Ok(Backdrop::Clear),
                token => token.parse().map(Backdrop::Color),
            })
            .transpose()?,
    })
}

/// The marks chosen; `none` is no marks, nothing chosen leaves them unchanged.
fn marks_of(choices: &[MarkChoice]) -> Option<Vec<Mark>> {
    (!choices.is_empty()).then(|| {
        choices
            .iter()
            .filter_map(|choice| match choice {
                MarkChoice::Bold => Some(Mark::Bold),
                MarkChoice::Italic => Some(Mark::Italic),
                MarkChoice::Underline => Some(Mark::Underline),
                MarkChoice::Strikethrough => Some(Mark::Strikethrough),
                MarkChoice::Code => Some(Mark::Code),
                MarkChoice::None => None,
            })
            .collect()
    })
}

/// The tags asked for by title; a blank one or `none` asks for none.
fn tags_of(tags: &[String]) -> Option<Vec<String>> {
    (!tags.is_empty()).then(|| {
        tags.iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !matches!(tag.as_str(), "" | "none"))
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
    through_cache(client, cache, CacheUse::Use, async |loading| {
        removed(client, loading, args).await
    })
    .await
}

async fn removed(
    client: &Client,
    loading: Loading<'_>,
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
    let report = change_board(
        client,
        loading,
        BoardWrite {
            board: &board,
            change: BoardChange::RemoveAll(objects),
            release,
            guard: None,
            mode: args.dry_run,
        },
    )
    .await?;
    let unconfirmed = match (several, report.receipt.delivery) {
        (true, _) => Some(Unconfirmed::new(
            format!("removing {count} objects"),
            ChangeRisk::Irreversible,
        )),
        (false, Delivery::Accepted) => None,
        (false, Delivery::Built) => frame_content_unconfirmed(&report.objects),
    };
    let outcome = gate(unconfirmed, (args.dry_run, args.confirm), || ())
        .err()
        .unwrap_or(Outcome::Sent);
    Ok(Removed { report, outcome })
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
            fixture::{ARROW, FRAME, STICKER, TEXT, arguments, board_of, objects},
            object::{
                EditArgs,
                RemoveArgs,
                change::{edited, removed},
            },
        },
        domain::change::receipt::Delivery,
        effect::board_write::WriteMode,
        remote::socket::fake::{BOARD, Scripted, signed_in},
    };

    fn link(object: &str) -> String {
        format!("https://app.holst.so/board/{BOARD}?objectId={object}")
    }

    const fn over(socket: &Scripted) -> Loading<'_> {
        Loading {
            connector: socket,
            cache: None,
            cache_use: CacheUse::Use,
        }
    }

    async fn edit_fields(
        given: serde_json::Value,
    ) -> Result<Vec<(String, Vec<&'static str>)>> {
        let (_, client) = signed_in([])?;
        let args = EditArgs {
            dry_run: WriteMode::DryRun,
            ..arguments(given)?
        };
        let socket = board_of(&objects());
        let report = edited(&client, over(&socket), &args).await?;
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

    #[rstest::rstest]
    #[case::empty("")]
    #[case::none("none")]
    #[case::none_padded(" none ")]
    fn a_blank_or_none_tag_clears_the_tags(#[case] tag: &str) {
        assert_eq!(super::tags_of(&[tag.to_owned()]), Some(Vec::new()));
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
    async fn an_object_flag_and_a_switch_are_refused_beside_items() -> Result<()> {
        let items = json!([{"object": STICKER, "color": "blue3"}]);
        let refused = [
            edit_fields(json!({"object": [TEXT], "items": items})).await,
            edit_fields(json!({"hide_content": true, "items": items})).await,
        ]
        .map(|result| result.err().map(|error| error.to_string()));
        assert_eq!(
            refused,
            [
                Some(
                    "object of `object edit` must be left out with --items".to_owned()
                ),
                Some(
                    "hide_content of `object edit` must be left out with --items"
                        .to_owned()
                )
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn uncheck_zero_is_refused_under_its_own_name() -> Result<()> {
        let refused = edit_fields(json!({"object": [STICKER], "uncheck": 0}))
            .await
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("uncheck of `object edit` must be an item number from 1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_options_of_a_sticker_are_one_edit() -> Result<()> {
        let changed = edit_fields(
            json!({"object": [STICKER], "text_size": "auto", "hide_author": true}),
        )
        .await?;
        assert_eq!(
            changed,
            vec![(STICKER.to_owned(), vec!["text size and author"])]
        );
        Ok(())
    }

    #[tokio::test]
    async fn both_author_flags_and_a_text_size_out_of_range_are_refused() -> Result<()>
    {
        let refused = [
            json!({"object": [STICKER], "show_author": true, "hide_author": true}),
            json!({"object": [STICKER], "text_size": "3"}),
            json!({"object": [STICKER], "text_size": "65"}),
            json!({"object": [STICKER], "text_size": "big"}),
        ]
        .map(|given| async move {
            edit_fields(given)
                .await
                .err()
                .map(|error| error.to_string())
        });
        let mut shown = Vec::new();
        // imperative: each call awaits the fake socket
        for call in refused {
            shown.push(call.await);
        }
        assert_eq!(
            shown,
            vec![
                Some(
                    "show_author of `object edit` must be left out with hide_author"
                        .to_owned()
                ),
                Some(
                    "text-size of `object edit` must be auto or a whole number 4 to 64"
                        .to_owned()
                ),
                Some(
                    "text-size of `object edit` must be auto or a whole number 4 to 64"
                        .to_owned()
                ),
                Some(
                    "text-size of `object edit` must be auto or a whole number 4 to 64"
                        .to_owned()
                ),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn reactions_and_none_are_an_edit_and_a_mixed_or_long_list_is_refused()
    -> Result<()> {
        let accepted = [
            json!({"object": [STICKER], "reaction": ["like", "fire"]}),
            json!({"object": [STICKER], "reaction": ["none"]}),
        ]
        .map(|given| async move { edit_fields(given).await });
        let mut changed = Vec::new();
        // imperative: each call awaits the fake socket
        for call in accepted {
            changed.push(call.await?);
        }
        let refused = [
            json!({"object": [STICKER], "reaction": ["like", "none"]}),
            json!({"object": [STICKER], "reaction": ["a", "b", "c", "d", "e", "f", "g", "h", "i"]}),
        ]
        .map(|given| async move {
            edit_fields(given)
                .await
                .err()
                .map(|error| error.to_string())
        });
        let mut shown = Vec::new();
        // imperative: each call awaits the fake socket
        for call in refused {
            shown.push(call.await);
        }
        let one = vec![(STICKER.to_owned(), vec!["reactions"])];
        assert_eq!(
            (changed, shown),
            (
                vec![one.clone(), one],
                vec![
                    Some(
                        "reaction of `object edit` must be reaction ids, or none alone"
                            .to_owned()
                    ),
                    Some(
                        "reaction of `object edit` must be at most 8 different reactions"
                            .to_owned()
                    ),
                ]
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_stroke_width_beside_items_is_named_as_the_tool_argument() -> Result<()> {
        let refused = edit_fields(
            json!({"stroke_width": 4, "items": [{"object": STICKER, "color": "blue3"}]}),
        )
        .await
        .err()
        .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("stroke_width of `object edit` must be left out with --items")
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
    ) -> Result<(Vec<String>, Delivery, serde_json::Value)> {
        let (_, client) = signed_in([])?;
        let args: RemoveArgs = arguments(given)?;
        let socket = board_of(&objects());
        let done = removed(&client, over(&socket), &args).await?;
        Ok((
            done.report
                .objects
                .into_iter()
                .map(|object| object.id)
                .collect(),
            done.report.receipt.delivery,
            serde_json::to_value(&done.outcome)?,
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
                json!({"status": "sent"})
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_held_removal_is_one_flat_result() -> Result<()> {
        let (_, client) = signed_in([])?;
        let args: RemoveArgs = arguments(json!({"object": [STICKER, TEXT]}))?;
        let socket = board_of(&objects());
        let done = removed(&client, over(&socket), &args).await?;
        let flat = serde_json::to_value(&done)?;
        assert_eq!(
            (
                flat.pointer("/status"),
                flat.pointer("/needs"),
                flat.pointer("/outcome"),
                flat.pointer("/update_bytes").map(serde_json::Value::is_u64)
            ),
            (
                Some(&json!("held")),
                Some(&json!("removing 2 objects cannot be undone")),
                None,
                Some(true)
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
                json!({"status": "held", "needs": "removing 2 objects cannot be undone"})
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn several_confirmed_objects_go() -> Result<()> {
        let done = remove(json!({"object": [STICKER, TEXT], "confirm": true})).await?;
        assert_eq!(done.1, Delivery::Accepted);
        assert_eq!(done.2, json!({"status": "sent"}));
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
                json!({
                    "status": "held",
                    "needs": "removing the frame and 2 objects with it cannot be undone"
                })
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_dry_run_asks_for_no_confirmation() -> Result<()> {
        let done = remove(json!({"object": [STICKER, TEXT], "dry_run": true})).await?;
        assert_eq!(
            (done.1, done.2),
            (Delivery::Built, json!({"status": "dry_run"}))
        );
        Ok(())
    }
}
