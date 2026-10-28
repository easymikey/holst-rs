//! The Confirmation rule of both shells for risky changes; pure.

use crate::{
    domain::{
        board::ObjectKind,
        change::value::{ChangeUpdate, ChangedObject, ObjectChangeKind},
    },
    effect::board_write::WriteMode,
    error::confirmation::{ChangeRisk, Unconfirmed},
};

/// Whether a built change may go out; if not, it stays unsent.
pub type ReleaseCheck = fn(&ChangeUpdate) -> bool;

pub const ALWAYS: ReleaseCheck = |_| true;

/// Holds back every change: its call is not confirmed and takes several.
pub const NEVER: ReleaseCheck = |_| false;

/// Holds back a change that removes a frame together with other objects.
pub const UNLESS_FRAME_CONTENT: ReleaseCheck =
    |built| frame_content_count(&built.objects).is_none();

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

/// How many objects go with a removed frame, when the first object touched is
/// a frame and others are removed after it.
fn frame_content_count(touched: &[ChangedObject]) -> Option<usize> {
    let more = touched
        .iter()
        .skip(1)
        .filter(|object| object.change == ObjectChangeKind::Removed)
        .count();
    match (touched.first().map(|object| &object.kind), more) {
        (Some(ObjectKind::Frame), 1..) => Some(more),
        (Some(_) | None, _) => None,
    }
}

/// The refusal to remove a frame together with its objects unasked; none when
/// `touched` takes no other object along with a frame.
#[must_use]
pub fn frame_content_unconfirmed(touched: &[ChangedObject]) -> Option<Unconfirmed> {
    frame_content_count(touched).map(|more| Unconfirmed {
        change: match more {
            1 => "removing the frame and 1 object with it".into(),
            more => format!("removing the frame and {more} objects with it").into(),
        },
        risk: ChangeRisk::Irreversible,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
