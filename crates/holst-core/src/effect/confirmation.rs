//! The Confirmation rule of both shells for risky changes; pure.

use crate::{
    domain::{
        board::ObjectKind,
        change::value::{ChangeUpdate, ChangedObject, ObjectChangeKind},
        kinds::kanban,
    },
    effect::board_write::WriteMode,
    error::confirmation::{ChangeRisk, Unconfirmed},
};

/// Whether a built change may go out; if not, it stays unsent.
pub type ReleaseCheck = fn(&ChangeUpdate) -> bool;

pub const ALWAYS: ReleaseCheck = |_| true;

/// Holds back every change: its call is not confirmed and takes several.
pub const NEVER: ReleaseCheck = |_| false;

/// Holds back a change that removes a frame, group or kanban with its content.
pub const UNLESS_FRAME_CONTENT: ReleaseCheck =
    |built| frame_content_count(&built.objects).is_none();

/// Holds back a change that removes table cells that held text.
pub const UNLESS_CELL_TEXT: ReleaseCheck =
    |built| cell_text_unconfirmed(&built.objects).is_none();

/// The one rule of both shells for a change that takes more along or cannot be
/// undone: `unconfirmed` stands unless the call is `confirmed` or a dry run,
/// which changes nothing.
///
/// Order of checks, the same on every surface: the target and its destination
/// are found first, so a mistake in them is named before a confirmation is
/// asked for; then this check; only then is anything sent.
///
/// # Errors
/// The `unconfirmed`, when there is one, `confirmed` is false and `mode` sends.
pub fn unless_confirmed(
    unconfirmed: Option<Unconfirmed>,
    confirmed: bool,
    mode: WriteMode,
) -> Result<(), Unconfirmed> {
    match (unconfirmed, confirmed, mode) {
        (Some(unconfirmed), false, WriteMode::Send) => Err(unconfirmed),
        (Some(_) | None, true, _)
        | (Some(_) | None, false, WriteMode::DryRun)
        | (None, false, WriteMode::Send) => Ok(()),
    }
}

/// The kind word of the first object touched and how many objects are removed
/// after it, when it is a frame, a group or a kanban.
fn frame_content_count(touched: &[ChangedObject]) -> Option<(&str, usize)> {
    let more = touched
        .iter()
        .skip(1)
        .filter(|object| object.change == ObjectChangeKind::Removed)
        .count();
    match (touched.first().map(|object| &object.kind), more) {
        (Some(kind), 1..) if holds_content(kind) => Some((kind.as_str(), more)),
        (Some(_) | None, _) => None,
    }
}

fn holds_content(kind: &ObjectKind) -> bool {
    matches!(kind, ObjectKind::Frame | ObjectKind::Group)
        || kind.as_str() == kanban::STORED
}

/// The refusal to remove a frame, group or kanban together with its objects
/// unasked; none when `touched` takes no other object along with one.
#[must_use]
pub fn frame_content_unconfirmed(touched: &[ChangedObject]) -> Option<Unconfirmed> {
    frame_content_count(touched).map(|(kind, more)| Unconfirmed {
        change: match more {
            1 => format!("removing the {kind} and 1 object with it").into(),
            more => format!("removing the {kind} and {more} objects with it").into(),
        },
        risk: ChangeRisk::Irreversible,
    })
}

/// The refusal to remove table cells that held text unasked (a removed cell
/// names `text` among its fields when it did); none without such a cell.
#[must_use]
pub fn cell_text_unconfirmed(touched: &[ChangedObject]) -> Option<Unconfirmed> {
    let cells = touched
        .iter()
        .filter(|object| {
            object.kind == ObjectKind::TableCell
                && ((object.change == ObjectChangeKind::Removed
                    && object.fields.contains(&"text"))
                    || object.fields.contains(&"text cleared"))
        })
        .count();
    match cells {
        0 => None,
        1 => Some(Unconfirmed::new(
            "removing 1 table cell with text",
            ChangeRisk::Contents,
        )),
        cells => Some(Unconfirmed::new(
            format!("removing {cells} table cells with text"),
            ChangeRisk::Contents,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_table_cells_with_text_is_held_back_and_names_the_count() {
        let cell = |text: bool| ChangedObject {
            fields: text.then_some("text").into_iter().collect(),
            ..removed(ObjectKind::TableCell)
        };
        let refusal = |cells: &[ChangedObject]| {
            cell_text_unconfirmed(cells).map(|unconfirmed| unconfirmed.to_string())
        };
        assert_eq!(refusal(&[cell(false), removed(ObjectKind::Sticker)]), None);
        assert_eq!(
            refusal(&[cell(true), cell(false)]).as_deref(),
            Some("removing 1 table cell with text takes everything inside along")
        );
        assert_eq!(
            refusal(&[cell(true), cell(true)]).as_deref(),
            Some("removing 2 table cells with text takes everything inside along")
        );
    }

    fn removed(kind: ObjectKind) -> ChangedObject {
        ChangedObject {
            id: "x".into(),
            kind,
            change: ObjectChangeKind::Removed,
            fields: Vec::new(),
            link: None,
        }
    }

    #[test]
    fn a_refusal_stands_only_for_an_unconfirmed_send() {
        let refusal = || {
            frame_content_unconfirmed(&[
                removed(ObjectKind::Frame),
                removed(ObjectKind::Sticker),
            ])
        };
        let outcomes = [
            (false, WriteMode::Send),
            (true, WriteMode::Send),
            (false, WriteMode::DryRun),
            (true, WriteMode::DryRun),
        ]
        .map(|(confirmed, mode)| unless_confirmed(refusal(), confirmed, mode).is_err());
        assert_eq!(outcomes, [true, false, false, false]);
        assert_eq!(unless_confirmed(None, false, WriteMode::Send), Ok(()));
    }

    #[test]
    fn removing_a_frame_with_objects_is_held_back_and_names_the_count() {
        let refusal = |kinds: &[ObjectKind]| {
            let touched: Vec<_> = kinds.iter().cloned().map(removed).collect();
            frame_content_unconfirmed(&touched)
                .map(|unconfirmed| unconfirmed.to_string())
        };
        assert_eq!(refusal(&[ObjectKind::Sticker, ObjectKind::Arrow]), None);
        assert_eq!(refusal(&[ObjectKind::Frame]), None);
        assert_eq!(
            refusal(&[ObjectKind::Group, ObjectKind::Sticker]).as_deref(),
            Some("removing the group and 1 object with it cannot be undone")
        );
        assert_eq!(
            refusal(&[ObjectKind::Other("kanban".into()), ObjectKind::Task]).as_deref(),
            Some("removing the kanban and 1 object with it cannot be undone")
        );
        assert_eq!(refusal(&[ObjectKind::Table, ObjectKind::TableCell]), None);
        assert_eq!(
            refusal(&[ObjectKind::Frame, ObjectKind::Sticker]).as_deref(),
            Some("removing the frame and 1 object with it cannot be undone")
        );
        assert_eq!(
            refusal(&[
                ObjectKind::Frame,
                ObjectKind::Sticker,
                ObjectKind::Text,
                ObjectKind::Arrow,
            ])
            .as_deref(),
            Some("removing the frame and 3 objects with it cannot be undone")
        );
        let built = |kinds: &[ObjectKind]| ChangeUpdate {
            update: Vec::new(),
            objects: kinds.iter().cloned().map(removed).collect(),
            kept: std::collections::BTreeSet::new(),
        };
        assert_eq!(
            (
                UNLESS_FRAME_CONTENT(&built(&[ObjectKind::Frame, ObjectKind::Sticker])),
                UNLESS_FRAME_CONTENT(&built(&[ObjectKind::Sticker])),
            ),
            (false, true)
        );
    }
}
