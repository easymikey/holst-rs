//! What a change names and asks, checked against the board before writing.

use std::collections::{BTreeSet, HashMap};

use itertools::Itertools as _;
use serde_json::Value;

use crate::{
    domain::{
        area::scene::Scene,
        board::{
            Object,
            ObjectKind,
            arrow_look::{ARROW_WIDTHS, ArrowEndEdit},
        },
        change::{
            value::{
                Arrangement,
                ArrowEdit,
                BoardChange,
                LinkTo,
                NewObject,
                NoteContent,
                ObjectEdit,
                ObjectPatch,
                Pasted,
                RemoveRight,
                Reparent,
                ShapeEdit,
                TextLook,
            },
            write_shape::line_of,
        },
        geometry,
        geometry::{Point, Rectangle, Size},
        rich_text::{Editor, Kept, TextFormat, parse},
    },
    error::{
        Error,
        Result,
        board_change::{
            BoardChangeError,
            GivenPoint,
            GivenRectangle,
            GivenSize,
            GroupRefusal,
            LockRefusal,
            OffBoardValue,
        },
    },
    protocol::document::{
        COLUMN_ID,
        CONTENT_HIDDEN,
        CREATED,
        END,
        HEIGHT,
        LOCKED,
        OBJECT_ID,
        PARENT_ID,
        POSITION,
        START,
        TYPE,
        WIDTH,
        X,
        Y,
        Z_INDEX,
    },
};

/// Written on a sticker stuck to another object.
const STICKY_POSITION: &str = "stickyPosition";
/// The author's number in an object's `created`.
const AUTHOR: &str = "a";
/// The type of a node of a mind map, which the web client never groups.
const MIND_MAP_NODE: &str = "mind-map-node";

#[derive(Debug, Clone, Default)]
pub(crate) struct Placement {
    pub(super) parent: Option<String>,
    pub(super) format: TextFormat,
}

impl CheckedChange<'_> {
    /// What of the markdown to be written stays text in its editor.
    pub(super) fn kept(&self) -> BTreeSet<Kept> {
        let of = |text: &str, kind: &ObjectKind| parse(text, &Editor::of(kind)).kept;
        match self {
            CheckedChange::AddAll(entries) => entries
                .iter()
                .filter(|(_, placement)| placement.format == TextFormat::Markdown)
                .filter_map(|(new, _)| {
                    written_text(new).map(|(kind, text)| of(text, &kind))
                })
                .flatten()
                .collect(),
            CheckedChange::EditAll(edits) => edits
                .iter()
                .filter(|checked| checked.edit.patch.format == TextFormat::Markdown)
                .filter_map(|checked| {
                    checked
                        .edit
                        .patch
                        .text
                        .as_deref()
                        .map(|text| of(text, &checked.kind))
                })
                .flatten()
                .collect(),
            CheckedChange::Note(content) => content.parsed().kept,
            CheckedChange::RemoveAll(_)
            | CheckedChange::Paste(_)
            | CheckedChange::Group(_)
            | CheckedChange::Lock(_)
            | CheckedChange::Ungroup(_)
            | CheckedChange::LockOnly(_)
            | CheckedChange::Unlock(_)
            | CheckedChange::Arrange(..) => BTreeSet::new(),
        }
    }
}

/// The kind and text of a new object that holds rich text.
fn written_text(new: &NewObject) -> Option<(ObjectKind, &str)> {
    match new {
        NewObject::Sticker { text, .. } => Some((ObjectKind::Sticker, text)),
        NewObject::Text { text, .. } => Some((ObjectKind::Text, text)),
        NewObject::Shape { text, .. } => Some((ObjectKind::Shape, text)),
        NewObject::Frame { .. }
        | NewObject::Arrow { .. }
        | NewObject::Link { .. }
        | NewObject::Image { .. }
        | NewObject::File { .. } => None,
    }
}

pub(crate) enum CheckedChange<'a> {
    Note(&'a NoteContent),
    AddAll(Vec<(NewObject, Placement)>),
    EditAll(Vec<CheckedEdit<'a>>),
    RemoveAll(Vec<String>),
    Paste(&'a Pasted),
    /// The members of a new group, once each, the top ones only.
    Group(Vec<String>),
    /// The members of a new locked group, once each, the top ones only.
    Lock(Vec<String>),
    Ungroup(Vec<String>),
    /// Frames and tables to flag locked.
    LockOnly(Vec<String>),
    /// Locked groups to dissolve and locked frames and tables to unlock.
    Unlock(Vec<String>),
    /// The top objects to move against each other, once each.
    Arrange(Vec<String>, Arrangement),
}

/// What a new group is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Grouping {
    /// Objects picked and moved together.
    Plain,
    /// Objects kept as they are: the group carries the lock flag.
    Locked,
}

/// An edit of many with the object it names found.
pub(crate) struct CheckedEdit<'a> {
    pub(super) id: String,
    pub(super) kind: ObjectKind,
    pub(super) edit: &'a ObjectEdit,
    /// The `linkTo` to store, the object it names found.
    pub(super) link_to: Option<String>,
    /// Where the object goes, the frame found.
    pub(super) reparent: Option<Reparent>,
}

pub(crate) fn on_board(at: Option<&Point>, size: Option<&Size>) -> Result<()> {
    let near = |number: f64| number.abs() <= geometry::MAX_COORDINATE;
    let far_at = at.filter(|at| !(near(at.x()) && near(at.y()))).map(|at| {
        OffBoardValue::At(GivenPoint {
            x: at.x(),
            y: at.y(),
        })
    });
    let far_size = size
        .filter(|size| !(near(size.width()) && near(size.height())))
        .map(|size| {
            OffBoardValue::Size(GivenSize {
                width: size.width(),
                height: size.height(),
            })
        });
    far_at.or(far_size).map_or(Ok(()), |what| {
        Err(Error::BoardChange(BoardChangeError::OffBoard { what }))
    })
}

const fn placed(new: &NewObject) -> (Option<&Point>, Option<&Size>) {
    match new {
        NewObject::Sticker { at, size, .. }
        | NewObject::Shape { at, size, .. }
        | NewObject::Frame { at, size, .. }
        | NewObject::Image { at, size, .. } => (Some(at), size.as_ref()),
        NewObject::Text { at, .. }
        | NewObject::Link { at, .. }
        | NewObject::File { at, .. } => (Some(at), None),
        NewObject::Arrow { .. } => (None, None),
    }
}

/// `change` checked against the board; `mine` is the author number of the
/// user, if the board knows one, for what `right` lets a removal take.
pub(crate) fn check<'a>(
    change: &'a BoardChange,
    states: &HashMap<String, Value>,
    right: RemoveRight,
    mine: Option<u32>,
) -> Result<CheckedChange<'a>> {
    let on_added = |new: &NewObject| {
        let (at, size) = placed(new);
        on_board(at, size)
    };
    match change {
        BoardChange::AddAll(entries) => {
            entries.iter().try_for_each(|entry| on_added(&entry.new))
        }
        BoardChange::EditAll(edits) => edits.iter().try_for_each(|edit| {
            on_board(edit.patch.at.as_ref(), edit.patch.size.as_ref())
        }),
        BoardChange::Paste(pasted) => pasted_on_board(pasted),
        BoardChange::Note(_)
        | BoardChange::RemoveAll(_)
        | BoardChange::Group(_)
        | BoardChange::Ungroup(_)
        | BoardChange::Lock(_)
        | BoardChange::LockOnly(_)
        | BoardChange::Unlock(_)
        | BoardChange::UnlockAll
        | BoardChange::Arrange { .. } => Ok(()),
    }?;
    match change {
        BoardChange::Note(content) => Ok(CheckedChange::Note(content)),
        BoardChange::AddAll(entries) => entries
            .iter()
            .try_fold(Vec::new(), |mut done, entry| {
                let (new, placement) =
                    checked_add(&entry.new, entry.frame.as_deref(), states, &done)?;
                done.push((
                    new,
                    Placement {
                        format: entry.format,
                        ..placement
                    },
                ));
                Ok::<_, Error>(done)
            })
            .map(CheckedChange::AddAll),
        BoardChange::EditAll(edits) => edits
            .iter()
            .map(|edit| {
                let id = find(states, &edit.object)?;
                unlocked(states, &id, &edit.object)?;
                let kind = kind_of(states, &id);
                let held = held_in(states, &id, &kind);
                changeable_edit(&kind, held, edit)?;
                arrow_fits(states, &id, &edit.patch.arrow)?;
                let link_to = edit
                    .patch
                    .link_to
                    .as_ref()
                    .map(|link| link_text(states, link))
                    .transpose()?;
                let reparent =
                    reparented(states, (&id, &kind), edit.patch.reparent.as_ref())?;
                Ok(CheckedEdit {
                    id,
                    kind,
                    edit,
                    link_to,
                    reparent,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(CheckedChange::EditAll),
        BoardChange::RemoveAll(objects) => {
            removable(states, objects, right, mine).map(CheckedChange::RemoveAll)
        }
        BoardChange::Paste(pasted) => Ok(CheckedChange::Paste(pasted)),
        BoardChange::Group(objects) => {
            groupable(states, objects).map(CheckedChange::Group)
        }
        BoardChange::Ungroup(objects) => {
            ungroupable(states, objects).map(CheckedChange::Ungroup)
        }
        BoardChange::Lock(objects) => {
            lockable(states, objects).map(CheckedChange::Lock)
        }
        BoardChange::LockOnly(objects) => {
            flaggable(states, objects).map(CheckedChange::LockOnly)
        }
        BoardChange::Unlock(objects) => {
            unlockable(states, objects).map(CheckedChange::Unlock)
        }
        BoardChange::UnlockAll => all_locked(states).map(CheckedChange::Unlock),
        BoardChange::Arrange { objects, how } => {
            arrangeable(states, objects).map(|ids| CheckedChange::Arrange(ids, *how))
        }
    }
}

/// The ids of the objects named that an alignment or a spread moves: the top
/// ones, once each. As the web client does, it refuses the whole selection if
/// one is locked, a table cell, a mind-map node or a card in a kanban column;
/// an arrow stuck to an object follows it and is left out.
// BUNDLE: main.a33651a4.js 209325-209330, 209412-209415, 209481-209487.
fn arrangeable(
    states: &HashMap<String, Value>,
    named: &[String],
) -> Result<Vec<String>> {
    let found = named
        .iter()
        .map(|reference| find(states, reference).map(|id| (reference, id)))
        .collect::<Result<Vec<_>>>()?;
    let refused = found.iter().find_map(|(reference, id)| {
        let kept = refusal_to_group(states, id, Grouping::Locked).filter(|why| {
            matches!(
                why,
                GroupRefusal::TableCell
                    | GroupRefusal::MindMapNode
                    | GroupRefusal::InColumn
            )
        });
        kept.map(|why| BoardChangeError::NotArrangeable {
            reference: (*reference).clone(),
            why,
        })
        .or_else(|| {
            locked(states, id).then(|| BoardChangeError::Locked {
                reference: (*reference).clone(),
            })
        })
    });
    refused.map_or_else(
        || {
            top_named(states, named).map(|top| {
                top.into_iter()
                    .map(|(_, id)| id)
                    .filter(|id| !joined_arrow(states, id))
                    .collect()
            })
        },
        |error| Err(Error::BoardChange(error)),
    )
}

/// Whether the object is an arrow with an end stuck to an object.
fn joined_arrow(states: &HashMap<String, Value>, id: &str) -> bool {
    kind_of(states, id) == ObjectKind::Arrow
        && [START, END].iter().any(|side| {
            states
                .get(id)
                .and_then(|fields| fields.get(*side))
                .and_then(|end| end.get(OBJECT_ID))
                .and_then(Value::as_str)
                .is_some()
        })
}

/// Whether the object itself carries the lock flag.
pub(super) fn flagged(states: &HashMap<String, Value>, object: &str) -> bool {
    states
        .get(object)
        .and_then(|fields| fields.get(LOCKED))
        .and_then(Value::as_bool)
        .unwrap_or_default()
}

/// The objects named, once each, the top ones only: what lies inside another
/// named object goes with it. Each is the id with the reference that named it.
fn top_named<'a>(
    states: &HashMap<String, Value>,
    named: &'a [String],
) -> Result<Vec<(&'a String, String)>> {
    let found = named
        .iter()
        .map(|reference| find(states, reference).map(|id| (reference, id)))
        .collect::<Result<Vec<_>>>()?;
    let asked: BTreeSet<String> = found.iter().map(|(_, id)| id.clone()).collect();
    Ok(found
        .into_iter()
        .unique_by(|(_, id)| id.clone())
        .filter(|(_, id)| {
            ancestry(states, id)
                .iter()
                .skip(1)
                .all(|above| !asked.contains(above.as_str()))
        })
        .collect())
}

/// The ids of the objects named, once each, that may go into one new group:
/// at least two, none of a kind the web client keeps out of groups.
fn groupable(states: &HashMap<String, Value>, named: &[String]) -> Result<Vec<String>> {
    let top = top_named(states, named)?;
    let too_few = (top.len() < 2).then(|| {
        Error::BoardChange(BoardChangeError::TooFewToGroup { given: top.len() })
    });
    let refused = top.iter().find_map(|(reference, id)| {
        refusal_to_group(states, id, Grouping::Plain).map(|why| {
            Error::BoardChange(BoardChangeError::NotGroupable {
                reference: (*reference).clone(),
                why,
            })
        })
    });
    too_few
        .or(refused)
        .map_or_else(|| Ok(top.iter().map(|(_, id)| id.clone()).collect()), Err)
}

/// The ids of the objects named, once each, that may go into one new locked
/// group: any number, frames included, none locked or kept out of groups.
fn lockable(states: &HashMap<String, Value>, named: &[String]) -> Result<Vec<String>> {
    let top = top_named(states, named)?;
    let refused = top.iter().find_map(|(reference, id)| {
        refusal_to_lock(states, id).map(|why| {
            Error::BoardChange(BoardChangeError::NotLockable {
                reference: (*reference).clone(),
                why,
            })
        })
    });
    refused.map_or_else(|| Ok(top.iter().map(|(_, id)| id.clone()).collect()), Err)
}

/// Why the web client's menu will not lock the object with a group, if it will
/// not.
fn refusal_to_lock(states: &HashMap<String, Value>, id: &str) -> Option<LockRefusal> {
    refusal_to_group(states, id, Grouping::Locked)
        .map(LockRefusal::Group)
        .or_else(|| locked(states, id).then_some(LockRefusal::Held))
}

/// The ids of the frames and tables named, once each, that are not locked.
fn flaggable(states: &HashMap<String, Value>, named: &[String]) -> Result<Vec<String>> {
    let found = named
        .iter()
        .map(|reference| find(states, reference).map(|id| (reference, id)))
        .collect::<Result<Vec<_>>>()?;
    found
        .iter()
        .unique_by(|(_, id)| id)
        .map(|(reference, id)| {
            let refusal =
                if matches!(kind_of(states, id), ObjectKind::Frame | ObjectKind::Table)
                {
                    locked(states, id).then_some(LockRefusal::Held)
                } else {
                    Some(LockRefusal::NotFrameOrTable)
                };
            refusal.map_or_else(
                || Ok(id.clone()),
                |why| {
                    Err(Error::BoardChange(BoardChangeError::NotLockable {
                        reference: (*reference).clone(),
                        why,
                    }))
                },
            )
        })
        .collect()
}

/// Whether the object itself is a group, frame or table that is locked: what
/// an unlock takes off.
fn locked_itself(states: &HashMap<String, Value>, id: &str) -> bool {
    let holder = matches!(
        kind_of(states, id),
        ObjectKind::Group | ObjectKind::Frame | ObjectKind::Table
    );
    holder && flagged(states, id)
}

/// The ids of the objects named, once each; each must be locked itself.
fn unlockable(
    states: &HashMap<String, Value>,
    named: &[String],
) -> Result<Vec<String>> {
    let found = named
        .iter()
        .map(|reference| find(states, reference).map(|id| (reference, id)))
        .collect::<Result<Vec<_>>>()?;
    found
        .iter()
        .unique_by(|(_, id)| id)
        .map(|(reference, id)| {
            if locked_itself(states, id) {
                Ok(id.clone())
            } else {
                Err(Error::BoardChange(BoardChangeError::NotLocked {
                    reference: (*reference).clone(),
                }))
            }
        })
        .collect()
}

/// The ids of everything locked on the board.
fn all_locked(states: &HashMap<String, Value>) -> Result<Vec<String>> {
    let ids: Vec<String> = states
        .keys()
        .filter(|id| locked_itself(states, id))
        .sorted_unstable()
        .cloned()
        .collect();
    if ids.is_empty() {
        Err(Error::BoardChange(BoardChangeError::NothingToUnlock))
    } else {
        Ok(ids)
    }
}

/// Why the web client's menu will not group the object, if it will not.
fn refusal_to_group(
    states: &HashMap<String, Value>,
    id: &str,
    grouping: Grouping,
) -> Option<GroupRefusal> {
    let in_group = ancestry(states, id)
        .iter()
        .skip(1)
        .any(|above| kind_of(states, above) == ObjectKind::Group);
    let kind = kind_of(states, id);
    let by_kind = match &kind {
        ObjectKind::Frame => {
            (grouping == Grouping::Plain).then_some(GroupRefusal::Frame)
        }
        ObjectKind::TableCell => Some(GroupRefusal::TableCell),
        ObjectKind::Other(name) => {
            (name == MIND_MAP_NODE).then_some(GroupRefusal::MindMapNode)
        }
        ObjectKind::Sticker
        | ObjectKind::StickerStack
        | ObjectKind::Text
        | ObjectKind::Shape
        | ObjectKind::Document
        | ObjectKind::Arrow
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Stamp
        | ObjectKind::Group
        | ObjectKind::Table
        | ObjectKind::Code
        | ObjectKind::Drawing
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link => None,
    };
    by_kind
        .or_else(|| held_in(states, id, &kind).map(|_| GroupRefusal::InColumn))
        .or_else(|| in_group.then_some(GroupRefusal::InGroup))
}

/// The ids of the groups named, once each; none may be locked.
fn ungroupable(
    states: &HashMap<String, Value>,
    named: &[String],
) -> Result<Vec<String>> {
    let found = named
        .iter()
        .map(|reference| find(states, reference).map(|id| (reference, id)))
        .collect::<Result<Vec<_>>>()?;
    found
        .iter()
        .unique_by(|(_, id)| id)
        .map(|(reference, id)| {
            if kind_of(states, id) == ObjectKind::Group {
                unlocked(states, id, reference).map(|()| id.clone())
            } else {
                Err(Error::BoardChange(BoardChangeError::NotGroup {
                    reference: (*reference).clone(),
                }))
            }
        })
        .collect()
}

/// Whether the web client keeps `id` as it is: a locked frame or table, a cell
/// of a locked table, or anything in a locked group.
pub(super) fn locked(states: &HashMap<String, Value>, id: &str) -> bool {
    let own = match kind_of(states, id) {
        ObjectKind::Frame | ObjectKind::Table => flagged(states, id),
        ObjectKind::TableCell => {
            states.get(id).and_then(parent_of).is_some_and(|table| {
                kind_of(states, table) == ObjectKind::Table && flagged(states, table)
            })
        }
        ObjectKind::Sticker
        | ObjectKind::StickerStack
        | ObjectKind::Text
        | ObjectKind::Shape
        | ObjectKind::Document
        | ObjectKind::Arrow
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Stamp
        | ObjectKind::Group
        | ObjectKind::Code
        | ObjectKind::Drawing
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => false,
    };
    own || ancestry(states, id).iter().any(|above| {
        kind_of(states, above) == ObjectKind::Group && flagged(states, above)
    })
}

fn unlocked(states: &HashMap<String, Value>, id: &str, reference: &str) -> Result<()> {
    if locked(states, id) {
        Err(Error::BoardChange(BoardChangeError::Locked {
            reference: reference.to_owned(),
        }))
    } else {
        Ok(())
    }
}

/// The ids named, when none of what their removal takes is locked, and, under
/// [`RemoveRight::OwnOnly`], all of it is the user's or goes with theirs.
fn removable<'a>(
    states: &HashMap<String, Value>,
    named: impl IntoIterator<Item = &'a String>,
    right: RemoveRight,
    mine: Option<u32>,
) -> Result<Vec<String>> {
    let named = named
        .into_iter()
        .map(|reference| find(states, reference).map(|id| (reference, id)))
        .collect::<Result<Vec<_>>>()?;
    let blocked = named.iter().find_map(|(reference, id)| {
        removal(states, std::slice::from_ref(id))
            .into_iter()
            .find(|taken| locked(states, taken))
            .map(|taken| {
                if taken == *id {
                    (*reference).clone()
                } else {
                    taken
                }
            })
    });
    let refusal = blocked
        .map(|reference| BoardChangeError::Locked { reference })
        .or_else(|| {
            (right == RemoveRight::OwnOnly)
                .then(|| foreign_among(states, &named, mine))
                .flatten()
                .map(|reference| BoardChangeError::NotYours { reference })
        });
    refusal.map_or_else(
        || Ok(named.iter().map(|(_, id)| id.clone()).collect()),
        |error| Err(Error::BoardChange(error)),
    )
}

/// The reference of the first of `named` whose removal takes what `mine` did
/// not make and that does not go with something that is.
fn foreign_among(
    states: &HashMap<String, Value>,
    named: &[(&String, String)],
    mine: Option<u32>,
) -> Option<String> {
    let going: BTreeSet<String> = named
        .iter()
        .flat_map(|(_, id)| subtree(states, id))
        .collect();
    named.iter().find_map(|(reference, id)| {
        subtree(states, id)
            .into_iter()
            .find(|taken| !may_take(states, taken, &going, mine))
            .map(|taken| {
                if taken == *id {
                    (*reference).clone()
                } else {
                    taken
                }
            })
    })
}

/// Whether the web client lets a user with the author number `mine` remove
/// `id` among the objects `going`, on a board that keeps removal to its
/// makers: what they made, a group (its content is checked on its own), and a
/// sticker stuck to an object that goes too.
fn may_take(
    states: &HashMap<String, Value>,
    id: &str,
    going: &BTreeSet<String>,
    mine: Option<u32>,
) -> bool {
    states.get(id).is_none_or(|fields| {
        let made_by_me = mine.is_some_and(|mine| {
            fields
                .get(CREATED)
                .and_then(|created| created.get(AUTHOR))
                .and_then(Value::as_u64)
                == Some(u64::from(mine))
        });
        let stuck_to_going = fields
            .get(STICKY_POSITION)
            .and_then(|sticky| sticky.get(PARENT_ID))
            .and_then(Value::as_str)
            .is_some_and(|parent| {
                going.contains(parent) || !states.contains_key(parent)
            });
        made_by_me || stuck_to_going || kind_of(states, id) == ObjectKind::Group
    })
}

/// Each object first, then what it holds, the arrows on it and the stamps
/// stuck to it.
pub(super) fn removal(states: &HashMap<String, Value>, ids: &[String]) -> Vec<String> {
    ids.iter()
        .flat_map(|id| {
            let held = subtree(states, id);
            let arrows: BTreeSet<String> = states
                .iter()
                .filter(|(_, fields)| {
                    ends_in(fields, START, &held) || ends_in(fields, END, &held)
                })
                .map(|(arrow, _)| arrow.clone())
                .collect();
            let stamps: BTreeSet<String> = states
                .iter()
                .filter(|(_, fields)| stuck_to(fields, &held))
                .map(|(stamp, _)| stamp.clone())
                .collect();
            let rest: BTreeSet<String> = held
                .into_iter()
                .chain(arrows)
                .chain(stamps)
                .filter(|other| other != id)
                .collect();
            std::iter::once(id.clone()).chain(rest)
        })
        .unique()
        .collect()
}

/// Whether the object is stuck to one of `held`, as a reaction stamp is.
pub(super) fn stuck_to(fields: &Value, held: &BTreeSet<String>) -> bool {
    fields
        .get(STICKY_POSITION)
        .and_then(|sticky| sticky.get(PARENT_ID))
        .and_then(Value::as_str)
        .is_some_and(|parent| held.contains(parent))
}

/// Whether the arrow's `side` end is stuck to an object of `held`.
pub(super) fn ends_in(fields: &Value, side: &str, held: &BTreeSet<String>) -> bool {
    fields
        .get(side)
        .and_then(|end| end.get(OBJECT_ID))
        .and_then(Value::as_str)
        .is_some_and(|object| held.contains(object))
}

/// Whether a frame shows its content, so that objects in it can be seen.
pub(super) fn shows_content(states: &HashMap<String, Value>, id: &str) -> bool {
    !states
        .get(id)
        .and_then(|fields| fields.get(CONTENT_HIDDEN))
        .and_then(Value::as_bool)
        .unwrap_or_default()
}

/// Where an edit puts the object, checked: a frame that shows its content and
/// does not lie inside the object, for an object that a frame or the board
/// holds. BUNDLE: main.a33651a4.js 171095-171097; 128.8f630629.js 2109-2121.
fn reparented(
    states: &HashMap<String, Value>,
    (id, kind): (&str, &ObjectKind),
    asked: Option<&Reparent>,
) -> Result<Option<Reparent>> {
    asked
        .map(|into| {
            movable(states, (id, kind))?;
            match into {
                Reparent::Board => Ok(Reparent::Board),
                Reparent::Frame(reference) => {
                    frame_taking(states, (id, kind), reference).map(Reparent::Frame)
                }
            }
        })
        .transpose()
}

fn not_movable(kind: &ObjectKind, why: String) -> Error {
    Error::BoardChange(BoardChangeError::NotMovable {
        kind: kind.as_str().to_owned(),
        why,
    })
}

/// The object is held by a frame or by the board only.
fn movable(
    states: &HashMap<String, Value>,
    (id, kind): (&str, &ObjectKind),
) -> Result<()> {
    let holder = states
        .get(id)
        .and_then(parent_of)
        .map(|up| kind_of(states, up))
        .filter(|up| *up != ObjectKind::Frame);
    match (kind, holder) {
        (ObjectKind::Arrow, _) => {
            Err(not_movable(kind, "it follows the objects it joins".into()))
        }
        (ObjectKind::Group, _) => {
            Err(not_movable(kind, "it stands where its members do".into()))
        }
        (_, Some(up)) => Err(not_movable(
            kind,
            format!("it is held by a {}", up.as_str()),
        )),
        (_, None) => Ok(()),
    }
}

/// The id of the frame named, once it is a frame that shows its content and
/// does not lie inside the object.
fn frame_taking(
    states: &HashMap<String, Value>,
    (id, kind): (&str, &ObjectKind),
    reference: &str,
) -> Result<String> {
    let frame = open_frame(states, reference)?;
    (!subtree(states, id).contains(&frame))
        .then_some(frame)
        .ok_or_else(|| not_movable(kind, "the frame lies inside it".into()))
}

/// The id of the frame named, once it is a frame that shows its content.
fn open_frame(states: &HashMap<String, Value>, reference: &str) -> Result<String> {
    let id = frame_in(states, reference)?;
    let reference = reference.to_owned();
    match (kind_of(states, &id), shows_content(states, &id)) {
        (ObjectKind::Frame, true) => Ok(id),
        (ObjectKind::Frame, false) => {
            Err(Error::BoardChange(BoardChangeError::ContentHidden {
                reference,
            }))
        }
        _ => Err(Error::BoardChange(BoardChangeError::NotFrame { reference })),
    }
}

/// The nearest frame above an object.
pub(super) fn frame_above(states: &HashMap<String, Value>, id: &str) -> Option<String> {
    ancestry(states, id)
        .into_iter()
        .skip(1)
        .find(|up| kind_of(states, up) == ObjectKind::Frame)
}

/// The top-left corner of a frame; the board's for none.
pub(super) fn corner_of(
    states: &HashMap<String, Value>,
    frame: Option<&str>,
) -> (f64, f64) {
    frame
        .and_then(|frame| states.get(frame))
        .and_then(rectangle_of)
        .map_or((0.0, 0.0), |area| (area.x, area.y))
}

/// Every copy that moves stays on the board.
fn pasted_on_board(pasted: &Pasted) -> Result<()> {
    pasted
        .objects
        .iter()
        .filter(|object| object.kind != ObjectKind::Arrow)
        .try_for_each(|object| {
            let corner = Point::new(object.x, object.y)?
                .shifted(pasted.shift.x(), pasted.shift.y())?;
            on_board(Some(&corner), None)
        })
}

/// A new object with where it goes; `earlier` are the entries of the same
/// change before it, which an arrow's `#N` ends name.
fn checked_add(
    new: &NewObject,
    frame: Option<&str>,
    states: &HashMap<String, Value>,
    earlier: &[(NewObject, Placement)],
) -> Result<(NewObject, Placement)> {
    frame.map_or_else(
        || {
            let parent = new_rectangle(new)
                .and_then(|rect| parent_frame(states, &rect, &BTreeSet::new(), None));
            check_new(new, states, parent, earlier)
        },
        |frame| checked_in(new, frame, states, earlier),
    )
}

/// A new object inside `frame`, from its corner; `frame` is an object, or `#N`
/// for a frame among the `earlier` entries, which stays `#N` for the writer.
fn checked_in(
    new: &NewObject,
    frame: &str,
    states: &HashMap<String, Value>,
    earlier: &[(NewObject, Placement)],
) -> Result<(NewObject, Placement)> {
    let not_frame = || {
        Error::BoardChange(BoardChangeError::NotFrame {
            reference: frame.to_owned(),
        })
    };
    let fresh = earlier_entry(frame);
    let (id, area) = fresh.map_or_else(
        || {
            let id = open_frame(states, frame)?;
            let area = states
                .get(&id)
                .and_then(rectangle_of)
                .ok_or_else(not_frame)?;
            Ok::<_, Error>((id, area))
        },
        |slot| {
            let area = earlier
                .get(slot)
                .map(|(before, _)| before)
                .filter(|before| matches!(before, NewObject::Frame { .. }))
                .and_then(new_rectangle)
                .ok_or_else(not_frame)?;
            Ok((frame.to_owned(), area))
        },
    )?;
    let placed = new.shifted(area.x, area.y)?;
    let rect = new_rectangle(&placed);
    let outside = rect.filter(|rect| !area.contains(rect));
    outside.map_or(Ok(()), |escaped| {
        Err(Error::BoardChange(BoardChangeError::OutsideFrame {
            reference: frame.to_owned(),
            frame: GivenRectangle {
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height,
            },
            at: GivenPoint {
                x: escaped.x,
                y: escaped.y,
            },
        }))
    })?;
    let parent = rect
        .filter(|_| fresh.is_none())
        .and_then(|rect| parent_frame(states, &rect, &BTreeSet::new(), Some(&id)))
        .unwrap_or(id);
    check_new(&placed, states, Some(parent), earlier)
}

fn check_new(
    new: &NewObject,
    states: &HashMap<String, Value>,
    parent: Option<String>,
    earlier: &[(NewObject, Placement)],
) -> Result<(NewObject, Placement)> {
    // An arrow is kept with the ids its ends were found by.
    let new = match new {
        NewObject::Arrow {
            from,
            to,
            color,
            look,
        } => {
            look.outline
                .stroke_width
                .map_or(Ok(()), |width| stroke_width_fits(&ObjectKind::Arrow, width))?;
            NewObject::Arrow {
                from: end_of(states, earlier, from)?,
                to: end_of(states, earlier, to)?,
                color: color.clone(),
                look: look.clone(),
            }
        }
        NewObject::Sticker { .. }
        | NewObject::Text { .. }
        | NewObject::Shape { .. }
        | NewObject::Frame { .. }
        | NewObject::Link { .. }
        | NewObject::Image { .. }
        | NewObject::File { .. } => new.clone(),
    };
    let parent = parent.or_else(|| match &new {
        NewObject::Arrow { from, to, .. } => common_frame(
            states,
            &chain(states, earlier, from),
            &chain(states, earlier, to),
        ),
        NewObject::Sticker { .. }
        | NewObject::Text { .. }
        | NewObject::Shape { .. }
        | NewObject::Frame { .. }
        | NewObject::Link { .. }
        | NewObject::Image { .. }
        | NewObject::File { .. } => None,
    });
    sticker_size(&new).map(|()| {
        (
            new,
            Placement {
                parent,
                format: TextFormat::Plain,
            },
        )
    })
}

/// The entry an arrow end `#N` names, counted from 0.
pub(super) fn earlier_entry(end: &str) -> Option<usize> {
    end.strip_prefix('#')?.parse::<usize>().ok()?.checked_sub(1)
}

/// An arrow end as it is kept: `#N` of an earlier entry as it is, an object
/// by its id.
fn end_of(
    states: &HashMap<String, Value>,
    earlier: &[(NewObject, Placement)],
    end: &str,
) -> Result<String> {
    match earlier_entry(end) {
        Some(slot) if slot < earlier.len() => Ok(end.to_owned()),
        Some(_) | None => find(states, end),
    }
}

/// The frames around an arrow end, innermost first.
fn chain(
    states: &HashMap<String, Value>,
    earlier: &[(NewObject, Placement)],
    end: &str,
) -> Vec<String> {
    earlier_entry(end).map_or_else(
        || ancestry(states, end),
        |slot| {
            earlier
                .get(slot)
                .and_then(|(_, placement)| placement.parent.as_deref())
                .map(|parent| ancestry(states, parent))
                .unwrap_or_default()
        },
    )
}

fn new_rectangle(new: &NewObject) -> Option<Rectangle> {
    let placed = |at: &Point, size: &Option<Size>, kind: &ObjectKind| {
        let (width, height) = sized(size.as_ref(), kind);
        Rectangle::new(at.x(), at.y(), width, height)
    };
    match new {
        NewObject::Sticker { at, size, .. } => {
            Some(placed(at, size, &ObjectKind::Sticker))
        }
        NewObject::Text { at, .. }
        | NewObject::Link { at, .. }
        | NewObject::File { at, .. } => Some(Rectangle::new(at.x(), at.y(), 0.0, 0.0)),
        NewObject::Image {
            upload, at, size, ..
        } => {
            let shown = size.unwrap_or(upload.pixels);
            Some(Rectangle::new(
                at.x(),
                at.y(),
                shown.width(),
                shown.height(),
            ))
        }
        NewObject::Shape { at, size, .. } => Some(placed(at, size, &ObjectKind::Shape)),
        NewObject::Frame { at, size, .. } => Some(placed(at, size, &ObjectKind::Frame)),
        NewObject::Arrow { .. } => None,
    }
}

pub(super) fn sized(size: Option<&Size>, kind: &ObjectKind) -> (f64, f64) {
    size.copied()
        .or_else(|| kind.default_size())
        .map_or((0.0, 0.0), |size| (size.width(), size.height()))
}

pub(super) fn rectangle_of(fields: &Value) -> Option<Rectangle> {
    let number = |value: Option<&Value>| value.and_then(Value::as_f64);
    let position = fields.get(POSITION)?;
    let kind =
        ObjectKind::of(fields.get(TYPE).and_then(Value::as_str).unwrap_or_default());
    let (width, height) = number(fields.get(WIDTH))
        .zip(number(fields.get(HEIGHT)))
        .or_else(|| {
            kind.default_size()
                .map(|size| (size.width(), size.height()))
        })
        .unwrap_or_default();
    Some(Rectangle::new(
        number(position.get(X))?,
        number(position.get(Y))?,
        width,
        height,
    ))
}

pub(super) fn parent_of(fields: &Value) -> Option<&str> {
    fields.get(PARENT_ID).and_then(Value::as_str)
}

fn ancestry(states: &HashMap<String, Value>, id: &str) -> Vec<String> {
    std::iter::successors(Some(id.to_owned()), |id| {
        states.get(id).and_then(parent_of).map(str::to_owned)
    })
    .take(states.len() + 1)
    .collect()
}

pub(super) fn parent_frame(
    states: &HashMap<String, Value>,
    rect: &Rectangle,
    held: &BTreeSet<String>,
    within: Option<&str>,
) -> Option<String> {
    let inside = within.map(|frame| subtree(states, frame));
    let layer = |id: &str| {
        let z = states
            .get(id)
            .and_then(|f| f.get(Z_INDEX))
            .and_then(Value::as_f64);
        (ancestry(states, id).len(), z.unwrap_or_default())
    };
    states
        .iter()
        .filter(|(id, fields)| {
            kind_of(states, id) == ObjectKind::Frame
                && shows_content(states, id)
                && !held.contains(*id)
                && inside.as_ref().is_none_or(|inside| inside.contains(*id))
                && rectangle_of(fields).is_some_and(|area| area.contains(rect))
        })
        .map(|(id, _)| (layer(id), id))
        .max_by(|(a, a_id), (b, b_id)| {
            a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(b_id.cmp(a_id))
        })
        .map(|(_, id)| id.clone())
}

fn common_frame(
    states: &HashMap<String, Value>,
    from: &[String],
    to: &[String],
) -> Option<String> {
    from.iter()
        .filter(|id| kind_of(states, id) == ObjectKind::Frame)
        .find(|id| to.contains(id))
        .cloned()
}

fn sticker_size(new: &NewObject) -> Result<()> {
    match new {
        NewObject::Sticker {
            size: Some(size), ..
        } => sticker_shape(*size),
        NewObject::Sticker { size: None, .. }
        | NewObject::Text { .. }
        | NewObject::Shape { .. }
        | NewObject::Frame { .. }
        | NewObject::Arrow { .. }
        | NewObject::Link { .. }
        | NewObject::Image { .. }
        | NewObject::File { .. } => Ok(()),
    }
}

fn sticker_shape(size: Size) -> Result<()> {
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    if near(size.width(), size.height()) || near(size.width(), 2.0 * size.height()) {
        Ok(())
    } else {
        Err(Error::BoardChange(BoardChangeError::StickerSize {
            width: size.width(),
            height: size.height(),
        }))
    }
}

/// An edit of many asks for something, and the object can take all of it.
fn changeable_edit(
    kind: &ObjectKind,
    held: Option<Held>,
    edit: &ObjectEdit,
) -> Result<()> {
    let tags = match (&edit.tags, kind) {
        (None, _) | (Some(_), ObjectKind::Sticker) => Ok(()),
        (Some(_), _) => Err(Error::BoardChange(BoardChangeError::NotChangeable {
            kind: kind.as_str().to_owned(),
            field: "tags",
        })),
    };
    match (&edit.patch, edit.stacking, &edit.tags) {
        (
            ObjectPatch {
                text: None,
                at: None,
                size: None,
                color: None,
                rotation: None,
                scale: None,
                link_to: None,
                width: None,
                tick: None,
                format: _,
                look:
                    TextLook {
                        marks: None,
                        color: None,
                        background: None,
                    },
                shape,
                arrow,
                frame,
                reparent: None,
            },
            None,
            None,
        ) if shape.first_field().is_none()
            && arrow.fields().is_empty()
            && frame.fields().is_empty() =>
        {
            Err(Error::BoardChange(BoardChangeError::NothingToChange))
        }
        (patch, _, _) => fits(kind, held, patch)
            .and(scales(kind, held, patch))
            .and(spins(kind, patch))
            .and(framed(kind, patch))
            .and(tags),
    }
}

/// An arrow edit that can be drawn: the bend suits the line, and an end on a
/// side names a box other than the arrow.
fn arrow_fits(
    states: &HashMap<String, Value>,
    id: &str,
    arrow: &ArrowEdit,
) -> Result<()> {
    let line = line_of(arrow.line, states.get(id).unwrap_or(&Value::Null));
    let bend = arrow
        .bend
        .filter(|bend| !bend.suits(line))
        .map_or(Ok(()), |bend| Err(bend.unfit()));
    let boxed = |end: &Option<ArrowEndEdit>| match end {
        Some(ArrowEndEdit::Side { object, .. }) => {
            let target = find(states, object)?;
            (target != id
                && kind_of(states, &target) != ObjectKind::Arrow
                && states.get(&target).and_then(rectangle_of).is_some())
            .then_some(())
            .ok_or_else(|| {
                Error::BoardChange(BoardChangeError::NotArrowLook {
                    raw: object.clone(),
                    expected: "a box to attach an arrow to",
                })
            })
        }
        Some(ArrowEndEdit::Free(_)) | None => Ok(()),
    };
    bend.and_then(|()| boxed(&arrow.from))
        .and_then(|()| boxed(&arrow.to))
}

/// The text stored as `linkTo`: empty to take the link away.
fn link_text(states: &HashMap<String, Value>, link: &LinkTo) -> Result<String> {
    match link {
        LinkTo::Clear => Ok(String::new()),
        LinkTo::Url(address) => Ok(address.clone()),
        LinkTo::Object { reference, share } => {
            find(states, reference).map(|id| format!("{share}?objectId={id}"))
        }
    }
}

/// An object and the container that places it.
#[derive(Debug, Clone, Copy)]
struct Held {
    object: &'static str,
    container: &'static str,
}

/// The object as named with the container that places it: a table cell sits
/// in its table, a task card with a `columnId` in a kanban.
fn held_in(
    states: &HashMap<String, Value>,
    id: &str,
    kind: &ObjectKind,
) -> Option<Held> {
    let in_column = states
        .get(id)
        .and_then(|fields| fields.get(COLUMN_ID))
        .is_some_and(Value::is_string);
    match kind {
        ObjectKind::TableCell => Some(Held {
            object: "table-cell",
            container: "table",
        }),
        ObjectKind::Task if in_column => Some(Held {
            object: "task-card",
            container: "kanban",
        }),
        ObjectKind::Sticker
        | ObjectKind::StickerStack
        | ObjectKind::Text
        | ObjectKind::Shape
        | ObjectKind::Document
        | ObjectKind::Frame
        | ObjectKind::Arrow
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Stamp
        | ObjectKind::Group
        | ObjectKind::Table
        | ObjectKind::Code
        | ObjectKind::Drawing
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => None,
    }
}

/// Whether the kind is a frame, if the patch asks for a ratio, background or
/// hidden content.
fn framed(kind: &ObjectKind, patch: &ObjectPatch) -> Result<()> {
    let field = patch.frame.fields().first().copied();
    field
        .filter(|_| *kind != ObjectKind::Frame)
        .map_or(Ok(()), |field| {
            Err(Error::BoardChange(BoardChangeError::NotChangeable {
                kind: kind.as_str().to_owned(),
                field,
            }))
        })
}

/// Whether the kind takes the rotation the patch asks for.
fn spins(kind: &ObjectKind, patch: &ObjectPatch) -> Result<()> {
    patch.rotation.filter(|_| !turns(kind)).map_or(Ok(()), |_| {
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            kind: kind.as_str().to_owned(),
            field: "rotation",
        }))
    })
}

/// Whether the kind has a rotation.
fn turns(kind: &ObjectKind) -> bool {
    // BUNDLE: no rotation for these (main.a33651a4.js 204237, 206092, 171083).
    !matches!(
        kind,
        ObjectKind::Table
            | ObjectKind::TableCell
            | ObjectKind::Frame
            | ObjectKind::Arrow
            | ObjectKind::Group
    )
}

/// Whether the web client resizes the kind (main.a33651a4.js 110870, 114350,
/// 114993, 117329, 118604, 120086, 121781, 123507, 124216, 109786); a kanban
/// has no stored size, only a scale.
fn resizable(kind: &ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Sticker
            | ObjectKind::StickerStack
            | ObjectKind::Text
            | ObjectKind::Shape
            | ObjectKind::Frame
            | ObjectKind::Table
            | ObjectKind::Code
            | ObjectKind::Drawing
            | ObjectKind::Dice
            | ObjectKind::Wheel
            | ObjectKind::Flipcard
            | ObjectKind::Task
            | ObjectKind::Link
            | ObjectKind::File
    ) || matches!(kind, ObjectKind::Other(name) if name == "phosphor-icon")
}

/// Whether the kind takes the `scale` the patch asks for, and a task card in
/// a kanban a new size at all: the kanban sizes it (main.a33651a4.js 114995).
fn scales(kind: &ObjectKind, held: Option<Held>, patch: &ObjectPatch) -> Result<()> {
    let refuse = |named: String, field| {
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            kind: named,
            field,
        }))
    };
    match (held, patch.size.is_some() || patch.scale.is_some()) {
        (Some(Held { object, container }), true) if *kind == ObjectKind::Task => {
            refuse(format!("{object} in a {container}"), "size")
        }
        _ if patch.scale.is_some() && !scaled(kind) => {
            refuse(kind.as_str().to_owned(), "scale")
        }
        _ => Ok(()),
    }
}

/// Whether the kind is drawn at a `scale` or a `textScale`.
fn scaled(kind: &ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Sticker
            | ObjectKind::StickerStack
            | ObjectKind::Text
            | ObjectKind::Code
            | ObjectKind::Drawing
            | ObjectKind::Dice
            | ObjectKind::Wheel
            | ObjectKind::Flipcard
            | ObjectKind::Task
            | ObjectKind::Link
            | ObjectKind::File
    ) || matches!(kind, ObjectKind::Other(name) if name == "kanban")
}

/// Whether the kind has what the patch changes.
fn fits(kind: &ObjectKind, held: Option<Held>, patch: &ObjectPatch) -> Result<()> {
    let refuse = |field| {
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            kind: kind.as_str().to_owned(),
            field,
        }))
    };
    let text = matches!(
        kind,
        ObjectKind::Sticker | ObjectKind::Text | ObjectKind::Shape | ObjectKind::Frame
    );
    let rich = matches!(
        kind,
        ObjectKind::Sticker | ObjectKind::Text | ObjectKind::Shape
    );
    let place = !matches!(kind, ObjectKind::Arrow);
    let size = resizable(kind);
    let color = matches!(
        kind,
        ObjectKind::Sticker | ObjectKind::Shape | ObjectKind::Frame | ObjectKind::Arrow
    );
    match (patch, held) {
        (ObjectPatch { text: Some(_), .. }, _) if !text => refuse("text"),
        (
            ObjectPatch {
                text: Some(_),
                format: TextFormat::Markdown,
                ..
            },
            _,
        ) if *kind == ObjectKind::Frame => refuse("text format"),
        (ObjectPatch { at: Some(_), .. }, _) if !place => refuse("position"),
        (ObjectPatch { at: Some(_), .. }, Some(Held { object, container })) => {
            Err(Error::BoardChange(BoardChangeError::PlacedByContainer {
                held: object,
                container,
            }))
        }
        (ObjectPatch { size: Some(_), .. }, _) if !size => refuse("size"),
        (ObjectPatch { color: Some(_), .. }, _) if !color => refuse("color"),
        (ObjectPatch { width: Some(_), .. }, _) if *kind != ObjectKind::Text => {
            refuse("width")
        }
        (ObjectPatch { tick: Some(_), .. }, _) if !rich => refuse("checklist"),
        (ObjectPatch { shape, arrow, .. }, _)
            if let Some(field) = unfit_field(kind, shape, arrow) =>
        {
            refuse(field)
        }
        (
            ObjectPatch {
                look: TextLook { marks: Some(_), .. },
                ..
            },
            _,
        ) if !rich => refuse("marks"),
        (
            ObjectPatch {
                look: TextLook { color: Some(_), .. },
                ..
            },
            _,
        ) if !rich => refuse("text color"),
        (
            ObjectPatch {
                look:
                    TextLook {
                        background: Some(_),
                        ..
                    },
                ..
            },
            _,
        ) if !rich => refuse("text background"),
        (
            ObjectPatch {
                shape:
                    ShapeEdit {
                        stroke_width: Some(width),
                        ..
                    },
                ..
            },
            _,
        ) => stroke_width_fits(kind, *width),
        (
            ObjectPatch {
                size: Some(asked), ..
            },
            _,
        ) if matches!(kind, ObjectKind::Sticker) => sticker_shape(*asked),
        (ObjectPatch { .. }, _) => Ok(()),
    }
}

/// The first field of an edit that only other kinds have: a shape has no
/// arrow line, an arrow has only a dash and a width of the outline.
fn unfit_field(
    kind: &ObjectKind,
    shape: &ShapeEdit,
    arrow: &ArrowEdit,
) -> Option<&'static str> {
    let line = arrow.fields().first().copied();
    let outline_only = shape
        .fields()
        .into_iter()
        .find(|field| !matches!(*field, "stroke-style" | "stroke-width"));
    match (*kind == ObjectKind::Shape, *kind == ObjectKind::Arrow) {
        (true, _) => line,
        (_, true) => outline_only,
        _ => shape.first_field().or(line),
    }
}

/// Whether the width is one the web client offers a shape (a whole 0 to 10)
/// or an arrow (its nine widths).
fn stroke_width_fits(kind: &ObjectKind, width: f64) -> Result<()> {
    let refused = |expected| {
        let raw = format!("{width}");
        Error::BoardChange(BoardChangeError::NotArrowLook { raw, expected })
    };
    let whole = width.fract() == 0.0 && (0.0..=10.0).contains(&width);
    match (
        *kind == ObjectKind::Arrow,
        ARROW_WIDTHS.contains(&width),
        whole,
    ) {
        (true, true, _) | (false, _, true) => Ok(()),
        (true, false, _) => Err(refused(
            "an arrow stroke width: 1, 1.5, 2, 4, 5, 6, 9, 13 or 17",
        )),
        (false, _, false) => {
            Err(refused("a shape stroke width: a whole number 0 to 10"))
        }
    }
}

pub(crate) fn find(states: &HashMap<String, Value>, reference: &str) -> Result<String> {
    crate::domain::lookup::rules::find_id(states.keys().map(String::as_str), reference)
}

fn frame_in(states: &HashMap<String, Value>, reference: &str) -> Result<String> {
    let wanted = crate::domain::id::object_in(reference)
        .map_or_else(|| reference.trim().to_owned(), String::from);
    let wanted = wanted.as_str();
    let exact = states
        .get_key_value(wanted)
        .map(|(id, _)| id)
        .or_else(|| states.keys().find(|id| id.eq_ignore_ascii_case(wanted)));
    exact.cloned().map_or_else(
        || {
            let frames: Vec<Object> = states
                .iter()
                .filter(|(id, _)| kind_of(states, id) == ObjectKind::Frame)
                .filter_map(|(_, fields)| {
                    crate::domain::board::object_of(fields.clone())
                })
                .collect();
            Scene::new(&frames)
                .frame(reference)
                .map(|frame| frame.id.clone())
        },
        Ok,
    )
}

pub(super) fn kind_of(states: &HashMap<String, Value>, id: &str) -> ObjectKind {
    ObjectKind::of(
        states
            .get(id)
            .and_then(|fields| fields.get(TYPE))
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
}

pub(super) fn subtree(states: &HashMap<String, Value>, root: &str) -> BTreeSet<String> {
    std::iter::successors(Some(BTreeSet::from([root.to_owned()])), |held| {
        let grown: BTreeSet<String> = states
            .iter()
            .filter(|(_, fields)| {
                parent_of(fields).is_some_and(|parent| held.contains(parent))
            })
            .map(|(id, _)| id.clone())
            .chain(held.iter().cloned())
            .collect();
        (grown.len() > held.len()).then_some(grown)
    })
    .last()
    .unwrap_or_default()
}
