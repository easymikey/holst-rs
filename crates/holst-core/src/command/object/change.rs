//! `object edit` and `object rm`: changes to objects already on a board.

use std::{collections::HashMap, num::NonZeroU32};

use futures_util::future::try_join_all;
use itertools::Itertools as _;
use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::{
    command::{
        account::account_id,
        board::show::{Loading, board_id, through_cache},
        board_load::{self, CacheUse, RemoteBoard},
        board_write::{
            BoardWrite,
            Guard,
            change_board,
            change_board_with,
            refuse_in,
            refuse_withheld,
        },
        change::{ChangeReport as Outcome, gate},
        lookup,
        node::missing_field,
        object::{
            EditArgs,
            EditFields,
            EditItem,
            MarkChoice,
            RemoveArgs,
            add::{Local, local_picture, uploaded},
            extract::extracted,
        },
        table::{ArgumentGroup as _, Confirmation, not_value},
        workspace::{Member, find_member, members_of},
    },
    domain::{
        board::{EditRight, LoadedBoard, arrow_look::ARROW_WIDTHS, objects},
        change::{
            kind_edit::{
                CardMove,
                KindEdit,
                MOST_KINDS,
                Setting,
                Span,
                StickerEdit,
                TableEdit,
                TaskEdit,
                TextSize,
                Track,
            },
            media::{FileDisplay, MediaEdit, uncounted},
            receipt::{ChangeReport, Delivery},
            value::{
                ArrowEdit,
                Backdrop,
                BoardChange,
                FrameEdit,
                ImageUpload,
                LinkTo,
                ObjectEdit,
                ObjectPatch,
                Reparent,
                ShapeEdit,
                TextLook,
                TextWidth,
                Tick,
                Upload,
            },
        },
        geometry::Size,
        id,
        kinds::{
            dice,
            task::{self, Day},
        },
        lookup::rules::find_id,
        node::NodeBrief,
        rich_text::{Mark, TextFormat},
    },
    effect::{
        board_write::WriteMode,
        confirmation::{
            ALWAYS,
            NEVER,
            UNLESS_CELL_TEXT,
            UNLESS_FRAME_CONTENT,
            cell_text_unconfirmed,
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
    remote::{
        api::{self, files::page_count},
        client::Client,
    },
};

const EDIT: &str = "object edit";
const REMOVE: &str = "object rm";
/// BUNDLE: the pen goes to 30 (main.a33651a4.js 109609), the outline to 10.
const MAX_STROKE_WIDTH: f64 = 30.0;
/// BUNDLE: the opacity slider starts at 0.1 (main.a33651a4.js 182790).
const MIN_OPACITY: f64 = 0.1;
/// BUNDLE: the menu refuses to roll more than 50 dice at once (`S2#4`).
const MOST_ROLLED: usize = 50;

/// A removal or an edit that may wait: what it took, and what it waits for.
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
    Ok(load_board_read(client, loading, board).await?.0)
}

/// As [`load_board`], with the `channel/{id}` reply the load read, if any.
pub(super) async fn load_board_read(
    client: &Client,
    loading: &Loading<'_>,
    board: &str,
) -> Result<(LoadedBoard, Option<Value>)> {
    let (loaded, _, channel) = board_load::load_with_channel(
        RemoteBoard {
            client,
            connector: loading.connector,
            board,
        },
        loading.cache,
        loading.cache_use,
    )
    .await?;
    Ok((loaded, channel))
}

/// Changes text, corner, size, color, rotation, link, order or tags of
/// objects, in one write. Deleting a table row or column whose cells hold
/// text stays unsent without the confirmation.
///
/// # Errors
/// An item that is not valid, an object that is not found or has no such
/// field, nothing to change, a picture that cannot be read or is not one, a
/// board that is not found or not editable, and errors of the requests (the
/// upload among them) and of the write.
pub async fn edit(
    client: &Client,
    cache: &CacheSettings,
    args: &EditArgs,
) -> Result<Removed> {
    match args.edit.extract.as_deref() {
        Some(pages) => {
            through_cache(client, cache, CacheUse::Use, async |loading| {
                extracted(client, loading, args, pages).await
            })
            .await
        }
        None => edit_fields(client, cache, args).await,
    }
}

async fn edit_fields(
    client: &Client,
    cache: &CacheSettings,
    args: &EditArgs,
) -> Result<Removed> {
    // The picture is read before the first request.
    let picture = picture_of(args)?;
    through_cache(client, cache, CacheUse::Use, async |loading| {
        edited(client, loading, args, picture.as_ref()).await
    })
    .await
}

/// What `--path` cannot stand beside: `--items`, a mask, a name, or more than
/// one object, since a call uploads once.
fn refuse_path_beside(args: &EditArgs) -> Result<()> {
    args.path.as_ref().map_or(Ok(()), |_| {
        match (
            args.items.0.is_empty(),
            args.edit.crop.is_some(),
            args.edit.name.is_some(),
        ) {
            (false, _, _) => Err(not_value(EDIT, "path", "left out with --items")),
            (true, true, _) => Err(not_value(EDIT, "crop", "left out with --path")),
            (true, false, true) => Err(not_value(EDIT, "name", "left out with --path")),
            (true, false, false) => match named(&args.board, &args.object, EDIT)?.len()
            {
                1 => Ok(()),
                _ => Err(not_value(EDIT, "path", "left out with several objects")),
            },
        }
    })
}

/// The picture of `--path` and its pixels.
fn picture_of(args: &EditArgs) -> Result<Option<(Local, Size)>> {
    refuse_path_beside(args)?;
    args.path
        .as_deref()
        .map(|path| local_picture(path, |expected| not_value(EDIT, "path", expected)))
        .transpose()
}

async fn edited(
    client: &Client,
    loading: Loading<'_>,
    args: &EditArgs,
    picture: Option<&(Local, Size)>,
) -> Result<Removed> {
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        CacheUse::Use,
    )
    .await?;
    let edits = read_edits(args, &client.base_url().share_link(&board))?;
    refuse_many_rolls(&edits)?;
    let edits = match assigning(&edits) {
        Some(()) => assigned(client, &board, edits).await?,
        None => edits,
    };
    let (edits, asked) = counted(client, &loading, &board, edits, args.dry_run).await?;
    let hides = edits.iter().any(|edit| edit.patch.frame.hidden.is_some());
    let release = match args.confirm {
        Confirmation::Given => ALWAYS,
        Confirmation::Missing => UNLESS_CELL_TEXT,
    };
    let write = async |changes: Vec<ObjectEdit>, mode: WriteMode, read| {
        change_board_with(
            client,
            loading,
            BoardWrite {
                board: &board,
                change: BoardChange::EditAll(changes),
                release,
                guard: hides.then_some(HIDING),
                mode,
            },
            read,
        )
        .await
    };
    let report = match picture {
        None => write(edits, args.dry_run, None).await?,
        Some((file, pixels)) => {
            let taking = |upload: &Upload| replaced(edits.clone(), upload, *pixels);
            // Before any upload: a board that cannot be changed takes no file.
            let (loaded, read) = load_board_read(client, &loading, &board).await?;
            match loaded.right {
                EditRight::Editable => Ok(()),
                EditRight::ReadOnly => {
                    Err(Error::BoardChange(BoardChangeError::ReadOnly))
                }
            }?;
            // Before any upload: the edit is built, to see the object takes a picture.
            let built = write(
                taking(&file.upload(file.name.clone())),
                WriteMode::DryRun,
                read.clone(),
            )
            .await?;
            match args.dry_run {
                WriteMode::DryRun => built,
                WriteMode::Send => {
                    let upload = uploaded(client, &board, file).await?;
                    write(taking(&upload), WriteMode::Send, read).await?
                }
            }
        }
    };
    let report = ChangeReport {
        warnings: report.warnings.into_iter().chain(asked).collect(),
        ..report
    };
    Ok(settled(report, args))
}

/// The report with what it waits for: a confirmation, a dry run, or nothing.
pub(super) fn settled(report: ChangeReport, args: &EditArgs) -> Removed {
    let unconfirmed = match report.receipt.delivery {
        Delivery::Accepted => None,
        Delivery::Built => cell_text_unconfirmed(&report.objects),
    };
    let outcome = gate(unconfirmed, (args.dry_run, args.confirm), || ())
        .err()
        .unwrap_or(Outcome::Sent);
    Removed { report, outcome }
}

/// `edits` with the page count of each page file they show as content and that
/// has none yet, asked of the page service; a dry run asks nothing and says it
/// would.
async fn counted(
    client: &Client,
    loading: &Loading<'_>,
    board: &str,
    edits: Vec<ObjectEdit>,
    mode: WriteMode,
) -> Result<(Vec<ObjectEdit>, Vec<String>)> {
    let content =
        |edit: &ObjectEdit| edit.patch.media.display == Some(FileDisplay::Content);
    // A board that cannot be changed gets no request: the write says why.
    let states: HashMap<String, Value> = match edits.iter().any(content) {
        wanted if wanted => {
            let loaded = load_board(client, loading, board).await?;
            objects(&loaded.document)
                .into_iter()
                .filter(|_| matches!(loaded.right, EditRight::Editable))
                .map(|object| (object.id, object.fields))
                .collect()
        }
        _ => HashMap::new(),
    };
    let names: Vec<Option<String>> = edits
        .iter()
        .map(|edit| {
            find_id(states.keys().map(String::as_str), &edit.object)
                .ok()
                .and_then(|id| states.get(&id))
                .filter(|_| content(edit))
                .and_then(|fields| uncounted(fields).map(str::to_owned))
        })
        .collect();
    match mode {
        WriteMode::DryRun => Ok((
            edits,
            names
                .into_iter()
                .flatten()
                .map(|name| {
                    format!("would ask the server for the page count of {name}")
                })
                .collect(),
        )),
        WriteMode::Send => Ok((
            try_join_all(
                edits
                    .into_iter()
                    .zip(names)
                    .map(|(edit, name)| with_pages(client, edit, name)),
            )
            .await?,
            Vec::new(),
        )),
    }
}

/// `edit` with the page count of the file `name`, when there is a name.
async fn with_pages(
    client: &Client,
    edit: ObjectEdit,
    name: Option<String>,
) -> Result<ObjectEdit> {
    match name {
        None => Ok(edit),
        Some(name) => {
            let pages = page_count(client, &name).await?;
            Ok(ObjectEdit {
                patch: ObjectPatch {
                    media: MediaEdit {
                        pages: Some(pages),
                        ..edit.patch.media
                    },
                    ..edit.patch
                },
                ..edit
            })
        }
    }
}

/// `edits` with the file `upload` as the picture each takes.
fn replaced(edits: Vec<ObjectEdit>, file: &Upload, pixels: Size) -> Vec<ObjectEdit> {
    edits
        .into_iter()
        .map(|edit| ObjectEdit {
            patch: ObjectPatch {
                media: MediaEdit {
                    replace: Some(ImageUpload {
                        file: file.clone(),
                        pixels,
                    }),
                    ..edit.patch.media
                },
                ..edit.patch
            },
            ..edit
        })
        .collect()
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

/// `Some` when an edit names assignees.
fn assigning(edits: &[ObjectEdit]) -> Option<()> {
    edits
        .iter()
        .flat_map(|edit| &edit.patch.kind_edits)
        .any(|kind| {
            matches!(
                kind,
                KindEdit::Task(TaskEdit {
                    assignees: Some(_),
                    ..
                })
            )
        })
        .then_some(())
}

/// `edits` with the assignees they name turned into ids of members of the
/// board's workspace.
async fn assigned(
    client: &Client,
    board: &str,
    edits: Vec<ObjectEdit>,
) -> Result<Vec<ObjectEdit>> {
    let members = assignable(client, board).await?;
    edits
        .into_iter()
        .map(|edit| {
            let kind_edits = edit
                .patch
                .kind_edits
                .into_iter()
                .map(|kind| match kind {
                    KindEdit::Task(task) => {
                        member_ids(task, &members).map(KindEdit::Task)
                    }
                    KindEdit::Sticker(_)
                    | KindEdit::Reaction(_)
                    | KindEdit::Roll
                    | KindEdit::Faces(_)
                    | KindEdit::Children(_)
                    | KindEdit::Flip
                    | KindEdit::Back(..)
                    | KindEdit::Code { .. }
                    | KindEdit::Table(_)
                    | KindEdit::Card(_) => Ok(kind),
                })
                .collect::<Result<_>>()?;
            Ok(ObjectEdit {
                patch: ObjectPatch {
                    kind_edits,
                    ..edit.patch
                },
                ..edit
            })
        })
        .collect()
}

/// The members of the workspace of `board`, when the caller may assign among
/// them: a member, admin or owner, as the web client's `canEditAssignees`.
async fn assignable(client: &Client, board: &str) -> Result<Vec<Member>> {
    let workspace = match lookup::cached_node(client, board) {
        Some(kept) => kept.workspace.id,
        None => NodeBrief::of(&api::channel::get(client, board).await?)?
            .workspace_id()
            .ok_or_else(|| missing_field("workspace_id"))?
            .to_owned(),
    };
    let members = members_of(api::workspace::get(client, &workspace).await?)?;
    let me = account_id(client).await?;
    match members
        .iter()
        .find(|member| member.id == me)
        .and_then(|member| member.role.as_deref())
    {
        Some("member" | "admin" | "owner") => Ok(members),
        _ => Err(not_value(
            EDIT,
            "assignee",
            "left out: only a member of the workspace assigns people",
        )),
    }
}

/// `task` with the references it names as the ids of `members`.
fn member_ids(task: TaskEdit, members: &[Member]) -> Result<TaskEdit> {
    let assignees = task
        .assignees
        .map(|named| {
            named
                .iter()
                .map(|each| {
                    find_member(members, each, EDIT).map(|member| member.id.clone())
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    Ok(TaskEdit { assignees, ..task })
}

/// Refuses a call that rolls more than [`MOST_ROLLED`] objects.
fn refuse_many_rolls(edits: &[ObjectEdit]) -> Result<()> {
    let rolled = edits
        .iter()
        .filter(|edit| edit.patch.kind_edits.contains(&KindEdit::Roll))
        .count();
    (rolled <= MOST_ROLLED)
        .then_some(())
        .ok_or_else(|| not_value(EDIT, "roll", "left out for more than 50 objects"))
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
    item.edit.extract.as_ref().map_or(Ok(()), |_| {
        Err(not_value(EDIT, "extract", "left out in --items"))
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
                    "a whole number 0 to 10 (a drawing 1 to 30), or for an arrow 1, 1.5, 2, 4, 5, 6, 9, 13 or 17",
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
        text: item.text.clone().filter(|_| item.cell.is_none()),
        format,
        look: look_of(item)?,
        at: item.at.as_deref().map(str::parse).transpose()?,
        size: item.size.as_deref().map(str::parse).transpose()?,
        color: item
            .color
            .as_deref()
            .filter(|_| item.cell.is_none())
            .filter(|raw| task::tint(raw).is_none())
            .map(str::parse)
            .transpose()?,
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
        kind_edits: kind_edits_of(item, format)?,
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
        media: media_of(item)?,
    })
}

/// What `--name`, `--crop`, `--flip-image`, `--display`, `--start-page`,
/// `--hide-details` and `--show-details` say; a mask sets the corner and
/// size itself.
fn media_of(item: &EditFields) -> Result<MediaEdit> {
    item.crop
        .and_then(|_| {
            [
                ("at", item.at.is_some()),
                ("size", item.size.is_some()),
                ("rotate", item.rotate.is_some()),
            ]
            .into_iter()
            .find_map(|(name, given)| given.then_some(name))
        })
        .map_or(Ok(()), |name| {
            Err(not_value(EDIT, name, "left out with --crop"))
        })?;
    Ok(MediaEdit {
        name: item.name.clone(),
        crop: item.crop,
        flip: item.flip_image,
        replace: None,
        display: item.display,
        start_page: item.start_page,
        hide_details: match (item.hide_details, item.show_details) {
            (true, true) => Err(not_value(
                EDIT,
                "show_details",
                "left out with hide_details",
            )),
            (hide, show) => Ok(hide.then_some(true).or(show.then_some(false))),
        }?,
        pages: None,
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
/// `who_can_manage_frame_hide_content` stands above the caller's role; `read`
/// is the board's `channel/{id}` reply if the caller has it, else it is asked.
pub(super) async fn refuse_hiding(
    client: &Client,
    board: &str,
    read: Option<&Value>,
) -> Result<()> {
    match read {
        Some(reply) => refuse_in(reply, HIDING),
        None => refuse_withheld(client, board, HIDING).await,
    }
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

/// What `--text-size`, `--show-author`, `--hide-author`, `--reaction`,
/// `--roll`, `--faces`, `--hide-children`, `--show-children`, `--flip` and
/// `--back`, `--language` and `--theme`, `--cell` and the insert and delete
/// flags of rows and columns, `--status`, `--start`, `--due` and
/// `--description`, `--assignee`, `--kanban`, `--column` and `--swimlane` say.
fn kind_edits_of(item: &EditFields, format: TextFormat) -> Result<Vec<KindEdit>> {
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
        .chain(item.roll.then_some(KindEdit::Roll))
        .chain(faces_of(item)?)
        .chain(
            hidden_of(EDIT, Some(item.hide_children), Some(item.show_children))?
                .map(KindEdit::Children),
        )
        .chain(placed_beside(item, "flip", item.flip)?.then_some(KindEdit::Flip))
        .chain(item.back.clone().map(|text| KindEdit::Back(text, format)))
        .chain((item.language.is_some() || item.theme.is_some()).then_some(
            KindEdit::Code {
                language: item.language,
                theme: item.theme,
            },
        ))
        .chain(table_edit_of(item, format)?)
        .chain(task_edit_of(item, format)?)
        .chain(card_move_of(item)?)
        .collect())
}

/// What `--cell` with `--text` and `--insert-row`, `--insert-column`,
/// `--delete-row` and `--delete-column` say; one of them to a call.
fn table_edit_of(item: &EditFields, format: TextFormat) -> Result<Option<KindEdit>> {
    let spanned =
        |name: &'static str, raw: Option<&str>, make: fn(Span) -> TableEdit| {
            raw.map(|raw| span_of(name, raw).map(|span| (name, make(span))))
                .transpose()
        };
    let structural: Vec<(&'static str, TableEdit)> = [
        item.insert_row
            .map(|at| ("insert_row", TableEdit::Insert(Track::Row, at))),
        item.insert_column
            .map(|at| ("insert_column", TableEdit::Insert(Track::Column, at))),
        item.delete_row
            .map(|at| ("delete_row", TableEdit::Delete(Track::Row, at))),
        item.delete_column
            .map(|at| ("delete_column", TableEdit::Delete(Track::Column, at))),
        spanned("merge", item.merge.as_deref(), TableEdit::Merge)?,
        item.unmerge
            .as_deref()
            .map(|raw| {
                pair_of("unmerge", raw).map(|(row, column)| {
                    ("unmerge", TableEdit::Unmerge { row, column })
                })
            })
            .transpose()?,
        spanned("clear", item.clear.as_deref(), TableEdit::Clear)?,
    ]
    .into_iter()
    .flatten()
    .collect();
    let cell = item
        .cell
        .as_deref()
        .map(|raw| cell_of(raw, (item.text.as_deref(), item.color.as_deref()), format))
        .transpose()?;
    match (cell, structural.as_slice()) {
        (None, []) => Ok(None),
        (Some(edit), []) => Ok(Some(KindEdit::Table(edit))),
        (None, [(_, edit)]) => Ok(Some(KindEdit::Table(edit.clone()))),
        (Some(_), [(name, _), ..]) => Err(not_value(EDIT, name, "left out with cell")),
        (None, [_, (name, _), ..]) => Err(not_value(
            EDIT,
            name,
            "left out with another of insert_row, insert_column, delete_row, delete_column, merge, unmerge and clear",
        )),
    }
}

/// What `--status`, `--start`, `--due`, `--description` and `--assignee` say;
/// `none` takes a word away. The assignees stay references until `assigned`.
fn task_edit_of(item: &EditFields, format: TextFormat) -> Result<Option<KindEdit>> {
    let day = |name: &'static str, given: Option<&str>| {
        setting_of(given, |raw| {
            raw.trim()
                .parse::<Day>()
                .map_err(|expected| not_value(EDIT, name, &expected.to_string()))
        })
    };
    let (start, due) = (
        day("start", item.start.as_deref())?,
        day("due", item.due.as_deref())?,
    );
    match (&start, &due) {
        (Some(Setting::To(start)), Some(Setting::To(due))) if start > due => {
            Err(not_value(EDIT, "start", "on or before the due day"))
        }
        _ => Ok(()),
    }?;
    let status = setting_of(item.status.as_deref(), |raw| {
        Some(raw.trim())
            .filter(|status| !status.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| not_value(EDIT, "status", "a status or none"))
    })?;
    let description =
        setting_of(item.description.as_deref(), |raw| Ok(raw.to_owned()))?;
    let assignees = tags_of(&item.assignee);
    let color = item
        .color
        .as_deref()
        .filter(|_| item.cell.is_none())
        .and_then(task::tint);
    Ok((status.is_some()
        || start.is_some()
        || due.is_some()
        || description.is_some()
        || assignees.is_some()
        || color.is_some())
    .then_some(KindEdit::Task(TaskEdit {
        status,
        start,
        due,
        description,
        format,
        assignees,
        color,
    })))
}

/// What `--kanban`, `--column` and `--swimlane` say: `none` takes the card out
/// of its kanban, else it goes to the cell the titles name.
fn card_move_of(item: &EditFields) -> Result<Option<KindEdit>> {
    let title = |name: &'static str, given: &[String]| match given {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        _ => Err(not_value(EDIT, name, "one title")),
    };
    let (column, swimlane) = (
        title("column", &item.column)?,
        title("swimlane", &item.swimlane)?,
    );
    let moved = match (item.kanban.as_deref().map(str::trim), column, swimlane) {
        (None, None, None) => Ok(None),
        (Some(""), ..) => Err(not_value(EDIT, "kanban", "a kanban or none")),
        (Some(_), ..) if item.frame.is_some() => {
            Err(not_value(EDIT, "frame", "left out with kanban"))
        }
        (Some("none"), None, None) => Ok(Some(KindEdit::Card(CardMove::Out))),
        (Some("none"), Some(_), _) => {
            Err(not_value(EDIT, "column", "left out with kanban none"))
        }
        (Some("none"), None, Some(_)) => {
            Err(not_value(EDIT, "swimlane", "left out with kanban none"))
        }
        (kanban, column, swimlane) => Ok(Some(KindEdit::Card(CardMove::Into {
            kanban: kanban.map(str::to_owned),
            column,
            swimlane,
        }))),
    }?;
    placed_beside(item, "kanban", moved.is_some())?;
    Ok(moved)
}

/// `given`, unless `--at`, `--rotate`, `--size` or `--scale` come beside it:
/// the flip and the card move write the place, turn and size themselves.
fn placed_beside(item: &EditFields, other: &str, given: bool) -> Result<bool> {
    [
        ("at", item.at.is_some()),
        ("rotate", item.rotate.is_some()),
        ("size", item.size.is_some()),
        ("scale", item.scale.is_some()),
    ]
    .into_iter()
    .filter(|(_, placed)| *placed && given)
    .try_for_each(|(name, _)| {
        Err(not_value(EDIT, name, &format!("left out with {other}")))
    })
    .map(|()| given)
}

/// `none` is [`Setting::Clear`], anything else is parsed.
fn setting_of<T>(
    given: Option<&str>,
    parse: impl FnOnce(&str) -> Result<T>,
) -> Result<Option<Setting<T>>> {
    given
        .map(|raw| match raw.trim() {
            "none" => Ok(Setting::Clear),
            _ => parse(raw).map(Setting::To),
        })
        .transpose()
}

/// A row and column from 1, e.g. `2,3`.
fn pair_of(name: &'static str, raw: &str) -> Result<(u32, u32)> {
    raw.split_once(',')
        .and_then(|(row, column)| {
            Some((row.trim().parse().ok()?, column.trim().parse().ok()?))
        })
        .ok_or_else(|| not_value(EDIT, name, "row,column, e.g. 2,3"))
}

/// A block of cells `r1,c1,r2,c2` from 1, the first not after the last.
fn span_of(name: &'static str, raw: &str) -> Result<Span> {
    let numbers: Vec<u32> = raw
        .split(',')
        .map(|part| part.trim().parse())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| span_hint(name))?;
    match numbers.as_slice() {
        &[row, column, to_row, to_column] if row <= to_row && column <= to_column => {
            Ok(Span {
                first: (row, column),
                last: (to_row, to_column),
            })
        }
        _ => Err(span_hint(name)),
    }
}

fn span_hint(name: &'static str) -> Error {
    not_value(
        EDIT,
        name,
        "r1,c1,r2,c2 from 1, the first row and column not after the last, e.g. 1,1,2,2",
    )
}

/// What `--cell` says: a cell or a block of cells, to take the text (one cell)
/// or the colour.
fn cell_of(
    raw: &str,
    (text, color): (Option<&str>, Option<&str>),
    format: TextFormat,
) -> Result<TableEdit> {
    let span = match raw.matches(',').count() {
        3 => span_of("cell", raw),
        _ => pair_of("cell", raw).map(|place| Span {
            first: place,
            last: place,
        }),
    }?;
    match (text, color) {
        (Some(text), None) if span.first == span.last => Ok(TableEdit::Cell {
            row: span.first.0,
            column: span.first.1,
            text: text.to_owned(),
            format,
        }),
        (Some(_), None) => {
            Err(not_value(EDIT, "cell", "row,column, e.g. 2,3, with text"))
        }
        (None, Some(color)) => Ok(TableEdit::Fill(span, color.parse()?)),
        (None, None) => Err(not_value(EDIT, "cell", "given with text or color")),
        (Some(_), Some(_)) => {
            Err(not_value(EDIT, "color", "left out with text beside cell"))
        }
    }
}

/// What `--faces` says: one of the face counts of the web client, not beside
/// a roll, which changes the value the faces keep in range.
fn faces_of(item: &EditFields) -> Result<Option<KindEdit>> {
    item.faces
        .map(|faces| match (item.roll, dice::FACES.contains(&faces)) {
            (true, _) => Err(not_value(EDIT, "faces", "left out with roll")),
            (false, true) => Ok(KindEdit::Faces(faces)),
            (false, false) => {
                Err(not_value(EDIT, "faces", "one of 2, 4, 6, 8, 10, 12 or 20"))
            }
        })
        .transpose()
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

/// The tags (or people) asked for by title; a blank one or `none` asks for
/// none.
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
            fixture::{
                ARROW,
                FRAME,
                STICKER,
                TEXT,
                arguments,
                board_of,
                channel,
                client,
                json_reply,
                objects,
            },
            object::{
                EditArgs,
                EditFields,
                RemoveArgs,
                change::{assigned, edited, patch_of, refuse_path_beside, removed},
            },
        },
        domain::{
            change::{
                kind_edit::{
                    CardMove,
                    KindEdit,
                    Setting,
                    Span,
                    TableEdit,
                    TaskEdit,
                    Track,
                },
                media::{CropMask, FlipState, MediaEdit},
                receipt::Delivery,
                value::{ObjectEdit, ObjectPatch},
            },
            kinds::{
                code::{Language, Theme},
                task::Day,
            },
            palette::Color,
            rich_text::TextFormat,
        },
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
        let done = edited(&client, over(&socket), &args, None).await?;
        Ok(done
            .report
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

    #[test]
    fn a_roll_and_faces_are_edits_of_a_die() -> Result<()> {
        let kind_edits = |fields: EditFields| {
            patch_of(&fields, TextFormat::Plain, "").map(|patch| patch.kind_edits)
        };
        let roll = EditFields {
            roll: true,
            ..EditFields::default()
        };
        let faces = EditFields {
            faces: Some(6),
            ..EditFields::default()
        };
        assert_eq!(
            (kind_edits(roll)?, kind_edits(faces)?),
            (vec![KindEdit::Roll], vec![KindEdit::Faces(6)])
        );
        Ok(())
    }

    #[test]
    fn a_flip_and_a_back_are_edits_of_a_flip_card() -> Result<()> {
        let fields = EditFields {
            flip: true,
            back: Some("**B**".to_owned()),
            ..EditFields::default()
        };
        assert_eq!(
            patch_of(&fields, TextFormat::Markdown, "")?.kind_edits,
            vec![
                KindEdit::Flip,
                KindEdit::Back("**B**".to_owned(), TextFormat::Markdown)
            ]
        );
        Ok(())
    }

    #[test]
    fn a_language_and_a_theme_are_one_edit_of_a_code_block() -> Result<()> {
        let fields = EditFields {
            language: Some(Language::Python),
            theme: Some(Theme::Dark),
            ..EditFields::default()
        };
        assert_eq!(
            patch_of(&fields, TextFormat::Plain, "")?.kind_edits,
            vec![KindEdit::Code {
                language: Some(Language::Python),
                theme: Some(Theme::Dark)
            }]
        );
        Ok(())
    }

    #[test]
    fn a_cell_takes_the_text_and_one_structural_flag_is_one_table_edit() -> Result<()> {
        let kind_edits = |fields: EditFields| {
            patch_of(&fields, TextFormat::Markdown, "")
                .map(|patch| (patch.text, patch.kind_edits))
        };
        let cell = EditFields {
            cell: Some(" 1 , 2 ".to_owned()),
            text: Some("Hi".to_owned()),
            ..EditFields::default()
        };
        let delete = EditFields {
            delete_column: Some(3),
            ..EditFields::default()
        };
        assert_eq!(
            (kind_edits(cell)?, kind_edits(delete)?),
            (
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Cell {
                        row: 1,
                        column: 2,
                        text: "Hi".to_owned(),
                        format: TextFormat::Markdown
                    })]
                ),
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Delete(Track::Column, 3))]
                )
            )
        );
        Ok(())
    }

    #[test]
    fn a_cell_without_text_and_several_structural_flags_are_refused() {
        let refused = [
            EditFields {
                cell: Some("1,2".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                cell: Some("one".to_owned()),
                text: Some("x".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                cell: Some("1,2".to_owned()),
                text: Some("x".to_owned()),
                insert_row: Some(1),
                ..EditFields::default()
            },
            EditFields {
                insert_row: Some(1),
                delete_column: Some(1),
                ..EditFields::default()
            },
        ]
        .map(|fields| {
            patch_of(&fields, TextFormat::Plain, "")
                .err()
                .map(|error| error.to_string())
        });
        assert_eq!(
            refused,
            [
                "cell of `object edit` must be given with text or color",
                "cell of `object edit` must be row,column, e.g. 2,3",
                "insert_row of `object edit` must be left out with cell",
                "delete_column of `object edit` must be left out with another of insert_row, insert_column, delete_row, delete_column, merge, unmerge and clear",
            ]
            .map(|text| Some(text.to_owned()))
        );
    }

    #[test]
    fn a_task_palette_name_is_a_task_edit_and_a_token_stays_a_color() -> Result<()> {
        let colored = |raw: &str| {
            patch_of(
                &EditFields {
                    color: Some(raw.to_owned()),
                    ..EditFields::default()
                },
                TextFormat::Plain,
                "",
            )
            .map(|patch| (patch.color, patch.kind_edits))
        };
        assert_eq!(
            (colored("Red")?, colored("red4")?),
            (
                (
                    None,
                    vec![KindEdit::Task(TaskEdit {
                        color: Some(0xe8_726a),
                        ..TaskEdit::default()
                    })]
                ),
                (Some("red4".parse()?), Vec::new())
            )
        );
        Ok(())
    }

    #[test]
    fn merge_unmerge_clear_and_a_cell_colour_are_table_edits() -> Result<()> {
        let table_edit = |fields: EditFields| {
            patch_of(&fields, TextFormat::Plain, "")
                .map(|patch| (patch.color, patch.kind_edits))
        };
        let given = |cell: &str| EditFields {
            cell: Some(cell.to_owned()),
            color: Some("blue3".to_owned()),
            ..EditFields::default()
        };
        let merge = EditFields {
            merge: Some("1,1,2, 2".to_owned()),
            ..EditFields::default()
        };
        let unmerge = EditFields {
            unmerge: Some("3,1".to_owned()),
            ..EditFields::default()
        };
        let clear = EditFields {
            clear: Some("1,1,1,3".to_owned()),
            ..EditFields::default()
        };
        let blue: Color = "blue3".parse()?;
        let span = |first, last| Span { first, last };
        assert_eq!(
            [
                table_edit(merge)?,
                table_edit(unmerge)?,
                table_edit(clear)?,
                table_edit(given("2,3"))?,
                table_edit(given("1,1,2,2"))?,
            ],
            [
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Merge(span((1, 1), (2, 2))))]
                ),
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Unmerge { row: 3, column: 1 })]
                ),
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Clear(span((1, 1), (1, 3))))]
                ),
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Fill(
                        span((2, 3), (2, 3)),
                        blue.clone()
                    ))]
                ),
                (
                    None,
                    vec![KindEdit::Table(TableEdit::Fill(span((1, 1), (2, 2)), blue))]
                ),
            ]
        );
        Ok(())
    }

    #[test]
    fn the_task_flags_are_one_task_edit_and_none_takes_a_word_away() -> Result<()> {
        let task_edit = |fields: EditFields| {
            patch_of(&fields, TextFormat::Markdown, "").map(|patch| patch.kind_edits)
        };
        let day = |raw: &str| raw.parse::<Day>().ok();
        assert_eq!(
            [
                task_edit(EditFields {
                    status: Some("Done".to_owned()),
                    start: Some("2026-11-02".to_owned()),
                    due: Some("2026-12-01".to_owned()),
                    description: Some("Go live".to_owned()),
                    ..EditFields::default()
                })?,
                task_edit(EditFields {
                    status: Some("none".to_owned()),
                    due: Some("none".to_owned()),
                    description: Some("none".to_owned()),
                    ..EditFields::default()
                })?,
            ],
            [
                vec![KindEdit::Task(TaskEdit {
                    status: Some(Setting::To("Done".to_owned())),
                    start: day("2026-11-02").map(Setting::To),
                    due: day("2026-12-01").map(Setting::To),
                    description: Some(Setting::To("Go live".to_owned())),
                    format: TextFormat::Markdown,
                    ..TaskEdit::default()
                })],
                vec![KindEdit::Task(TaskEdit {
                    status: Some(Setting::Clear),
                    due: Some(Setting::Clear),
                    description: Some(Setting::Clear),
                    format: TextFormat::Markdown,
                    ..TaskEdit::default()
                })],
            ]
        );
        Ok(())
    }

    #[test]
    fn the_assignee_flag_joins_the_task_edit_and_none_clears() -> Result<()> {
        let task_edit = |assignee: &[&str], status: Option<&str>| {
            patch_of(
                &EditFields {
                    assignee: assignee.iter().map(|each| (*each).to_owned()).collect(),
                    status: status.map(str::to_owned),
                    ..EditFields::default()
                },
                TextFormat::Plain,
                "",
            )
            .map(|patch| patch.kind_edits)
        };
        let task = |status: Option<&str>, assignees: Option<&[&str]>| {
            vec![KindEdit::Task(TaskEdit {
                status: status.map(|raw| Setting::To(raw.to_owned())),
                assignees: assignees
                    .map(|named| named.iter().map(|each| (*each).to_owned()).collect()),
                ..TaskEdit::default()
            })]
        };
        assert_eq!(
            [
                task_edit(&["bob", "ann@"], None)?,
                task_edit(&["none"], None)?,
                task_edit(&["bob"], Some("done"))?,
                task_edit(&[], None)?,
            ],
            [
                task(None, Some(&["bob", "ann@"])),
                task(None, Some(&[])),
                task(Some("done"), Some(&["bob"])),
                Vec::new(),
            ]
        );
        Ok(())
    }

    #[test]
    fn the_kanban_flags_are_one_card_move_and_none_takes_the_card_out() {
        let moves = |kanban: Option<&str>, column: &[&str], swimlane: &[&str]| {
            let titles = |each: &[&str]| {
                each.iter()
                    .map(|title| (*title).to_owned())
                    .collect::<Vec<_>>()
            };
            patch_of(
                &EditFields {
                    kanban: kanban.map(str::to_owned),
                    column: titles(column),
                    swimlane: titles(swimlane),
                    ..EditFields::default()
                },
                TextFormat::Plain,
                "",
            )
            .map(|patch| patch.kind_edits)
            .ok()
        };
        let into =
            |kanban: Option<&str>, column: Option<&str>, swimlane: Option<&str>| {
                Some(vec![KindEdit::Card(CardMove::Into {
                    kanban: kanban.map(str::to_owned),
                    column: column.map(str::to_owned),
                    swimlane: swimlane.map(str::to_owned),
                })])
            };
        assert_eq!(
            [
                moves(Some("k1"), &["Done"], &["Fast"]),
                moves(None, &["Done"], &[]),
                moves(None, &[], &["Fast"]),
                moves(Some(" none "), &[], &[]),
                moves(None, &[], &[]),
            ],
            [
                into(Some("k1"), Some("Done"), Some("Fast")),
                into(None, Some("Done"), None),
                into(None, None, Some("Fast")),
                Some(vec![KindEdit::Card(CardMove::Out)]),
                Some(Vec::new()),
            ]
        );
    }

    #[test]
    fn the_kanban_flags_are_refused_beside_none_a_second_title_or_a_frame() {
        let refusal = |fields: EditFields| {
            patch_of(&fields, TextFormat::Plain, "")
                .err()
                .map(|error| error.to_string())
        };
        let kanban = |raw: &str| Some(raw.to_owned());
        let title = || vec!["Done".to_owned()];
        assert_eq!(
            [
                refusal(EditFields {
                    kanban: kanban("none"),
                    column: title(),
                    ..EditFields::default()
                }),
                refusal(EditFields {
                    kanban: kanban("none"),
                    swimlane: title(),
                    ..EditFields::default()
                }),
                refusal(EditFields {
                    column: vec!["Done".to_owned(), "To do".to_owned()],
                    ..EditFields::default()
                }),
                refusal(EditFields {
                    kanban: kanban("k1"),
                    frame: kanban("f1"),
                    ..EditFields::default()
                }),
                refusal(EditFields {
                    kanban: kanban(" "),
                    ..EditFields::default()
                }),
            ],
            [
                Some(
                    "column of `object edit` must be left out with kanban none"
                        .to_owned()
                ),
                Some(
                    "swimlane of `object edit` must be left out with kanban none"
                        .to_owned()
                ),
                Some("column of `object edit` must be one title".to_owned()),
                Some("frame of `object edit` must be left out with kanban".to_owned()),
                Some("kanban of `object edit` must be a kanban or none".to_owned()),
            ]
        );
    }

    fn assigning(named: &[&str]) -> Vec<ObjectEdit> {
        vec![ObjectEdit {
            object: "k1".to_owned(),
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Task(TaskEdit {
                    assignees: Some(
                        named.iter().map(|each| (*each).to_owned()).collect(),
                    ),
                    ..TaskEdit::default()
                })],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]
    }

    /// `assigning(named)` resolved over a workspace where the caller holds
    /// `role`, with the requests it sent.
    async fn assigned_by(
        role: &str,
        named: &[&str],
    ) -> Result<(Result<Vec<ObjectEdit>>, Vec<String>)> {
        let workspace = json!({
            "id": "w1",
            "users": [
                {"id": "u1", "name": "Ann", "email": "ann@x.io"},
                {"id": "u2", "name": "Bob", "email": "bob@x.io"},
            ],
            "workspace_users": [
                {"workspace_id": "w1", "user_id": "u1", "role": role},
                {"workspace_id": "w1", "user_id": "u2", "role": "member"},
            ],
        });
        let (fake, client) = client([
            json_reply(&json!({"id": BOARD, "type": "board", "workspace_id": "w1"})),
            json_reply(&workspace),
            json_reply(&json!({"id": "u1"})),
        ])?;
        let done = assigned(&client, BOARD, assigning(named)).await;
        let sent = fake
            .requests()
            .iter()
            .map(|request| format!("{:?} {}", request.method, request.url.path()))
            .collect();
        Ok((done, sent))
    }

    #[tokio::test]
    async fn members_are_found_by_email_name_or_id_after_one_workspace_read()
    -> Result<()> {
        let (done, sent) = assigned_by("member", &["BOB@x", "ann", "u2"]).await?;
        assert_eq!(
            (done?, sent),
            (
                assigning(&["u2", "u1", "u2"]),
                vec![
                    format!("Get /gapi/channel/{BOARD}"),
                    "Get /gapi/workspace/w1".to_owned(),
                    "Get /gapi/current_user".to_owned(),
                ]
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn none_clears_and_still_reads_the_workspace() -> Result<()> {
        let (done, sent) = assigned_by("admin", &[]).await?;
        assert_eq!((done?, sent.len()), (assigning(&[]), 3));
        Ok(())
    }

    #[tokio::test]
    async fn an_unknown_name_is_refused_with_the_members_listed() -> Result<()> {
        let (done, _) = assigned_by("member", &["Zed"]).await?;
        assert_eq!(
            done.err().map(|error| error.to_string()),
            Some(
                "user of `object edit` must be a member of the workspace: Ann (u1), Bob (u2)"
                    .to_owned()
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_dry_run_reads_the_workspace_before_the_card_is_checked() -> Result<()> {
        let (fake, client) = client([
            json_reply(&json!({"id": BOARD, "type": "board", "workspace_id": "w1"})),
            json_reply(&json!({
                "id": "w1",
                "users": [{"id": "u1", "name": "Ann"}, {"id": "u2", "name": "Bob"}],
                "workspace_users": [{"user_id": "u1", "role": "owner"}],
            })),
            json_reply(&json!({"id": "u1"})),
            channel(),
        ])?;
        let args = EditArgs {
            dry_run: WriteMode::DryRun,
            ..arguments(json!({"object": ["k1"], "assignee": ["bob"]}))?
        };
        let task = json!({
            "id": "k1", "type": "task-card", "position": {"x": 0, "y": 0},
            "titleDocumentId": "t1", "taskCardInfoId": "i1", "statusId": "to-do",
        });
        let socket = board_of(&[task]);
        // The fixture board holds no info entry: the card is refused after the reads.
        let done = edited(&client, over(&socket), &args, None).await;
        let sent: Vec<_> = fake
            .requests()
            .iter()
            .map(|request| format!("{:?} {}", request.method, request.url.path()))
            .collect();
        assert_eq!(
            (done.err().map(|error| error.to_string()), sent),
            (
                Some("task-card objects have no assignees to change".to_owned()),
                vec![
                    format!("Get /gapi/channel/{BOARD}"),
                    "Get /gapi/workspace/w1".to_owned(),
                    "Get /gapi/current_user".to_owned(),
                    format!("Get /gapi/channel/{BOARD}"),
                ]
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_guest_cannot_assign() -> Result<()> {
        let (done, _) = assigned_by("guest", &["Bob"]).await?;
        assert_eq!(
            done.err().map(|error| error.to_string()),
            Some(
                "assignee of `object edit` must be left out: only a member of the workspace assigns people"
                    .to_owned()
            )
        );
        Ok(())
    }

    #[test]
    fn a_bad_day_an_empty_status_or_a_start_after_the_due_day_is_refused() {
        let refused = [
            EditFields {
                due: Some("soon".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                status: Some(" ".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                start: Some("2026-12-02".to_owned()),
                due: Some("2026-12-01".to_owned()),
                ..EditFields::default()
            },
        ]
        .map(|fields| {
            patch_of(&fields, TextFormat::Plain, "")
                .err()
                .map(|error| error.to_string())
        });
        assert_eq!(
            refused,
            [
                "due of `object edit` must be a day as YYYY-MM-DD, e.g. 2026-11-06",
                "status of `object edit` must be a status or none",
                "start of `object edit` must be on or before the due day",
            ]
            .map(|text| Some(text.to_owned()))
        );
    }

    #[test]
    fn a_ragged_range_or_a_text_with_a_range_or_a_colour_is_refused() {
        let refused = [
            EditFields {
                merge: Some("2,1,1,2".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                clear: Some("1,1".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                unmerge: Some("1,1,2,2".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                cell: Some("1,1,2,2".to_owned()),
                text: Some("x".to_owned()),
                ..EditFields::default()
            },
            EditFields {
                cell: Some("1,1".to_owned()),
                text: Some("x".to_owned()),
                color: Some("blue3".to_owned()),
                ..EditFields::default()
            },
        ]
        .map(|fields| {
            patch_of(&fields, TextFormat::Plain, "")
                .err()
                .map(|error| error.to_string())
        });
        assert_eq!(
            refused,
            [
                "merge of `object edit` must be r1,c1,r2,c2 from 1, the first row and column not after the last, e.g. 1,1,2,2",
                "clear of `object edit` must be r1,c1,r2,c2 from 1, the first row and column not after the last, e.g. 1,1,2,2",
                "unmerge of `object edit` must be row,column, e.g. 2,3",
                "cell of `object edit` must be row,column, e.g. 2,3, with text",
                "color of `object edit` must be left out with text beside cell",
            ]
            .map(|text| Some(text.to_owned()))
        );
    }

    #[tokio::test]
    async fn a_flip_or_a_card_move_is_refused_beside_a_place_turn_or_size() -> Result<()>
    {
        let refused = [
            json!({"object": [STICKER], "flip": true, "at": "10,10"}),
            json!({"object": [STICKER], "flip": true, "rotate": 90}),
            json!({"object": [STICKER], "flip": true, "size": "10,10"}),
            json!({"object": [STICKER], "flip": true, "scale": 2}),
            json!({"object": [STICKER], "kanban": "K", "scale": 2}),
            json!({"object": [STICKER], "kanban": "none", "at": "10,10"}),
            json!({"object": [STICKER], "column": ["Done"], "size": "10,10"}),
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
            [
                "at of `object edit` must be left out with flip",
                "rotate of `object edit` must be left out with flip",
                "size of `object edit` must be left out with flip",
                "scale of `object edit` must be left out with flip",
                "scale of `object edit` must be left out with kanban",
                "at of `object edit` must be left out with kanban",
                "size of `object edit` must be left out with kanban",
            ]
            .map(|text| Some(text.to_owned()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn faces_off_the_die_set_and_more_than_fifty_rolls_are_refused() -> Result<()>
    {
        let many: Vec<String> = (0..51).map(|number| format!("o{number}")).collect();
        let refused = [
            json!({"object": [STICKER], "faces": 7}),
            json!({"object": many, "roll": true}),
            json!({"object": [STICKER], "faces": 6, "roll": true}),
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
                    "faces of `object edit` must be one of 2, 4, 6, 8, 10, 12 or 20"
                        .to_owned()
                ),
                Some(
                    "roll of `object edit` must be left out for more than 50 objects"
                        .to_owned()
                ),
                Some("faces of `object edit` must be left out with roll".to_owned()),
            ]
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
        remove_from(&objects(), given).await
    }

    async fn remove_from(
        board: &[serde_json::Value],
        given: serde_json::Value,
    ) -> Result<(Vec<String>, Delivery, serde_json::Value)> {
        let (_, client) = signed_in([])?;
        let args: RemoveArgs = arguments(given)?;
        let socket = board_of(board);
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
    async fn a_group_or_kanban_with_content_waits_for_the_confirmation() -> Result<()> {
        let at = json!({"x": 0, "y": 0});
        let held = async |kind: &str| {
            let board = [
                json!({"id": "p1", "type": kind, "position": at}),
                json!({"id": "c1", "type": "sticker", "position": at, "parentId": "p1"}),
            ];
            remove_from(&board, json!({"object": ["p1"]})).await
        };
        let needs = |done: (Vec<String>, Delivery, serde_json::Value)| done.2;
        assert_eq!(
            (needs(held("group").await?), needs(held("kanban").await?)),
            (
                json!({
                    "status": "held",
                    "needs": "removing the group and 1 object with it cannot be undone"
                }),
                json!({
                    "status": "held",
                    "needs": "removing the kanban and 1 object with it cannot be undone"
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

    #[test]
    fn a_crop_and_a_flip_are_edits_of_an_image() -> Result<()> {
        let fields = EditFields {
            crop: Some(CropMask::Wide),
            flip_image: Some(FlipState::Both),
            ..EditFields::default()
        };
        assert_eq!(
            patch_of(&fields, TextFormat::Plain, "")?.media,
            MediaEdit {
                name: None,
                crop: Some(CropMask::Wide),
                flip: Some(FlipState::Both),
                ..MediaEdit::default()
            }
        );
        Ok(())
    }

    #[test]
    fn a_crop_is_refused_beside_a_corner_a_size_or_a_turn() {
        let crop = EditFields {
            crop: Some(CropMask::Circle),
            ..EditFields::default()
        };
        let refused = [
            EditFields {
                at: Some("1,2".to_owned()),
                ..crop.clone()
            },
            EditFields {
                size: Some("10,10".to_owned()),
                ..crop.clone()
            },
            EditFields {
                rotate: Some(10.0),
                ..crop
            },
        ]
        .map(|fields| {
            patch_of(&fields, TextFormat::Plain, "")
                .err()
                .map(|error| error.to_string())
        });
        assert_eq!(
            refused,
            ["at", "size", "rotate"].map(|name| Some(format!(
                "{name} of `object edit` must be left out with --crop"
            )))
        );
    }

    #[rstest::rstest]
    #[case::items(
        json!({"board": "B", "path": "d.png", "items": [{"object": "a1", "text": "x"}]}),
        "path of `object edit` must be left out with --items"
    )]
    #[case::crop(
        json!({"board": "B", "path": "d.png", "object": ["a1"], "crop": "circle"}),
        "crop of `object edit` must be left out with --path"
    )]
    #[case::name(
        json!({"board": "B", "path": "d.png", "object": ["a1"], "name": "Logo"}),
        "name of `object edit` must be left out with --path"
    )]
    #[case::several(
        json!({"board": "B", "path": "d.png", "object": ["a1", "b2"]}),
        "path of `object edit` must be left out with several objects"
    )]
    fn a_path_is_refused_beside_what_it_cannot_share_a_call_with(
        #[case] given: serde_json::Value,
        #[case] said: &str,
    ) -> Result<()> {
        let args: EditArgs = arguments(given)?;
        assert_eq!(
            refuse_path_beside(&args)
                .err()
                .map(|error| error.to_string()),
            Some(said.to_owned())
        );
        Ok(())
    }
}
