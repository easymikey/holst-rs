//! Changes to a board's objects and how the server took them.

use std::fmt;

use itertools::Itertools as _;

use crate::error::{Error, hint::Hint};

/// A point on the board, as given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GivenPoint {
    pub x: f64,
    pub y: f64,
}

/// A width and a height, as given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GivenSize {
    pub width: f64,
    pub height: f64,
}

/// A rectangle on the board, by its corner and its extent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GivenRectangle {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What a change put off the board, as the user gave it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffBoardValue {
    At(GivenPoint),
    Size(GivenSize),
}

impl fmt::Display for GivenPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", short(self.x), short(self.y))
    }
}

impl fmt::Display for GivenSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", short(self.width), short(self.height))
    }
}

impl fmt::Display for GivenRectangle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{},{},{}",
            short(self.x),
            short(self.y),
            short(self.width),
            short(self.height)
        )
    }
}

impl fmt::Display for OffBoardValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OffBoardValue::At(at) => fmt::Display::fmt(at, f),
            OffBoardValue::Size(size) => fmt::Display::fmt(size, f),
        }
    }
}

/// A number as short text: past 1e15 in scientific form.
fn short(number: f64) -> impl fmt::Display {
    fmt::from_fn(move |f| match number {
        huge if huge.abs() >= 1e15 => write!(f, "{huge:e}"),
        other => write!(f, "{other}"),
    })
}

/// Why an object cannot go into a group, as the web client's menu refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupRefusal {
    Frame,
    MindMapNode,
    TableCell,
    InGroup,
    InColumn,
}

impl fmt::Display for GroupRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GroupRefusal::Frame => "a frame holds its content itself",
            GroupRefusal::MindMapNode => "a mind-map node stays in its map",
            GroupRefusal::TableCell => "a table cell stays in its table",
            GroupRefusal::InGroup => "it is in a group already; ungroup that one first",
            GroupRefusal::InColumn => "a card in a kanban column stays in it",
        })
    }
}

/// Why an object cannot be locked, as the web client's menu refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockRefusal {
    Held,
    NotFrameOrTable,
    Group(GroupRefusal),
}

impl fmt::Display for LockRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockRefusal::Held => f.write_str("it is locked already"),
            LockRefusal::NotFrameOrTable => {
                f.write_str("--only locks frames and tables, nothing else")
            }
            LockRefusal::Group(why) => write!(f, "{why}"),
        }
    }
}

/// A board change that was refused, malformed or not accepted.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum BoardChangeError {
    #[error("the board is read-only for you: nothing was changed")]
    ReadOnly,

    #[error(
        "copying out of this board is held to {needs} and above: your right here is {holds}; \
         nothing was copied"
    )]
    CopyWithheld { needs: String, holds: &'static str },

    #[error(
        "locking and unlocking on this board is held to {needs} and above: your right here \
         is {holds}; nothing was changed"
    )]
    LockWithheld { needs: String, holds: &'static str },

    #[error(
        "hiding and showing frame content on this board is held to {needs} and above: your \
         right here is {holds}; nothing was changed"
    )]
    HideWithheld { needs: String, holds: &'static str },

    #[error(
        "the target board already has {objects} objects: import only into an empty board, or \
         merge with --merge; nothing was changed"
    )]
    NotEmpty { objects: usize },

    #[error(
        "\"{reference}\" does not name an object: give its full id, a link with objectId=, or at \
         least the first 8 hex characters of its id"
    )]
    NoObject { reference: String },

    #[error("{kind} objects have no {field} to change")]
    NotChangeable { kind: String, field: &'static str },

    #[error(
        "a {held} in a {container} has no position to change: the {container} places it"
    )]
    PlacedByContainer {
        held: &'static str,
        container: &'static str,
    },

    #[error(
        "\"{reference}\" is locked on the board; unlock it in the web client first"
    )]
    Locked { reference: String },

    #[error(
        "\"{reference}\" was made by someone else, and this board lets only its owners \
         remove that"
    )]
    NotYours { reference: String },

    #[error(
        "nothing to change: give a text, a position, a size, a color, an order or tags"
    )]
    NothingToChange,

    #[error("the text has no checklist item {item}: it has {count}")]
    NoCheckItem { item: u32, count: usize },

    #[error("\"{raw}\" is not a point: expected x,y, e.g. 100,-40")]
    NotPoint { raw: String },

    #[error("{what} is off the board: coordinates and sizes go up to 1e12")]
    OffBoard { what: OffBoardValue },

    #[error("\"{raw}\" is not a size: expected WIDTHxHEIGHT, e.g. 192x192")]
    NotSize { raw: String },

    #[error(
        "\"{raw}\" is not a palette color: expected a name and a shade 1-12, e.g. yellow4"
    )]
    NotColor { raw: String },

    #[error("\"{raw}\" is not {expected}")]
    NotArrowLook { raw: String, expected: &'static str },

    #[error("\"{raw}\" is not a shape kind: expected {known}")]
    NotShapeKind { raw: String, known: &'static str },

    #[error("a sticker is square or twice as wide as tall, not {width}×{height}")]
    StickerSize { width: f64, height: f64 },

    #[error("\"{reference}\" is not a frame: only frames hold objects")]
    NotFrame { reference: String },

    #[error(
        "frame \"{reference}\" hides its content: objects put in it would vanish for viewers; \
         show its content in the web client first"
    )]
    ContentHidden { reference: String },

    #[error(
        "the object at {at} would stick out of frame \"{reference}\" ({frame}): --at counts from \
         the frame's corner"
    )]
    OutsideFrame {
        reference: String,
        frame: GivenRectangle,
        at: GivenPoint,
    },

    #[error("a group needs at least 2 objects, not {given}: nothing was changed")]
    TooFewToGroup { given: usize },

    #[error("\"{reference}\" cannot be grouped: {why}")]
    NotGroupable {
        reference: String,
        why: GroupRefusal,
    },

    #[error("\"{reference}\" is not a group: only groups can be ungrouped")]
    NotGroup { reference: String },

    #[error("\"{reference}\" cannot be locked: {why}")]
    NotLockable { reference: String, why: LockRefusal },

    #[error(
        "\"{reference}\" is not locked itself: only a locked group, frame or table can be \
         unlocked"
    )]
    NotLocked { reference: String },

    #[error("nothing is locked on the board: nothing was changed")]
    NothingToUnlock,

    #[error(
        "the server did not accept the change within {seconds} s — it may or may not be on \
         the board"
    )]
    AcceptTimeout { seconds: u64 },

    #[error(
        "the board connection closed before the server accepted the change — it may or may \
         not be on the board"
    )]
    ClosedBeforeAccept,

    #[error(
        "update {update}{} was sent but the server did not accept it — it may or may not be on \
         the board",
        objects_note(.objects)
    )]
    NotAccepted {
        update: u64,
        objects: Vec<String>,
        #[source]
        cause: Box<Error>,
    },
}

impl BoardChangeError {
    /// What to do next, decided for every variant.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            BoardChangeError::ReadOnly
            | BoardChangeError::CopyWithheld { .. }
            | BoardChangeError::LockWithheld { .. }
            | BoardChangeError::HideWithheld { .. } => Some(Hint::AskOwner),
            BoardChangeError::NoObject { .. }
            | BoardChangeError::NotFrame { .. }
            | BoardChangeError::NotGroup { .. }
            | BoardChangeError::NotLocked { .. }
            | BoardChangeError::OutsideFrame { .. } => Some(Hint::ListObjects),
            BoardChangeError::AcceptTimeout { .. }
            | BoardChangeError::ClosedBeforeAccept => Some(Hint::CheckBeforeRetry),
            BoardChangeError::NotAccepted { .. } => Some(Hint::ReadBeforeRetry),
            BoardChangeError::NotEmpty { .. }
            | BoardChangeError::NotChangeable { .. }
            | BoardChangeError::PlacedByContainer { .. }
            | BoardChangeError::Locked { .. }
            | BoardChangeError::NotYours { .. }
            | BoardChangeError::NothingToChange
            | BoardChangeError::NoCheckItem { .. }
            | BoardChangeError::NotPoint { .. }
            | BoardChangeError::OffBoard { .. }
            | BoardChangeError::NotSize { .. }
            | BoardChangeError::NotColor { .. }
            | BoardChangeError::NotShapeKind { .. }
            | BoardChangeError::NotArrowLook { .. }
            | BoardChangeError::StickerSize { .. }
            | BoardChangeError::TooFewToGroup { .. }
            | BoardChangeError::NotGroupable { .. }
            | BoardChangeError::NotLockable { .. }
            | BoardChangeError::NothingToUnlock
            | BoardChangeError::ContentHidden { .. } => None,
        }
    }
}

impl BoardChangeError {
    #[must_use]
    pub(super) fn mcp_words(&self) -> Option<String> {
        match self {
            BoardChangeError::NotEmpty { objects } => Some(format!(
                "the target board already has {objects} objects: import only into an empty board, \
                 or merge with merge: true; nothing was changed"
            )),
            BoardChangeError::NoObject { reference } => Some(format!(
                "\"{reference}\" does not name an object: give an @N from the object tool, its full id, \
                 a link with objectId=, or at least the first 8 hex characters of its id"
            )),
            BoardChangeError::OutsideFrame {
                reference,
                frame,
                at,
            } => Some(format!(
                "the object at {at} would stick out of frame \"{reference}\" ({frame}): x,y count \
                 from the frame's corner"
            )),
            BoardChangeError::ReadOnly
            | BoardChangeError::CopyWithheld { .. }
            | BoardChangeError::LockWithheld { .. }
            | BoardChangeError::HideWithheld { .. }
            | BoardChangeError::NotChangeable { .. }
            | BoardChangeError::PlacedByContainer { .. }
            | BoardChangeError::Locked { .. }
            | BoardChangeError::NotYours { .. }
            | BoardChangeError::NothingToChange
            | BoardChangeError::NoCheckItem { .. }
            | BoardChangeError::NotPoint { .. }
            | BoardChangeError::OffBoard { .. }
            | BoardChangeError::NotSize { .. }
            | BoardChangeError::NotColor { .. }
            | BoardChangeError::NotShapeKind { .. }
            | BoardChangeError::NotArrowLook { .. }
            | BoardChangeError::StickerSize { .. }
            | BoardChangeError::NotFrame { .. }
            | BoardChangeError::TooFewToGroup { .. }
            | BoardChangeError::NotGroupable { .. }
            | BoardChangeError::NotGroup { .. }
            | BoardChangeError::NotLockable { .. }
            | BoardChangeError::NotLocked { .. }
            | BoardChangeError::NothingToUnlock
            | BoardChangeError::ContentHidden { .. }
            | BoardChangeError::AcceptTimeout { .. }
            | BoardChangeError::ClosedBeforeAccept
            | BoardChangeError::NotAccepted { .. } => None,
        }
    }
}

fn objects_note(objects: &[String]) -> impl fmt::Display {
    fmt::from_fn(move |f| {
        if objects.is_empty() {
            Ok(())
        } else {
            write!(f, " (objects {})", objects.iter().join(", "))
        }
    })
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        Error,
        board_change::{
            BoardChangeError,
            BoardChangeErrorDiscriminants,
            GivenPoint,
            GivenRectangle,
            GroupRefusal,
            LockRefusal,
            OffBoardValue,
        },
        hint::{Hint, Surface, decided::assert_every_variant},
    };

    pub(in crate::error) fn samples() -> Vec<(BoardChangeError, Option<Hint>)> {
        input_samples().into_iter().chain(ack_samples()).collect()
    }

    fn input_samples() -> Vec<(BoardChangeError, Option<Hint>)> {
        let text = || String::from("x");
        vec![
            (BoardChangeError::ReadOnly, Some(Hint::AskOwner)),
            (
                BoardChangeError::CopyWithheld {
                    needs: text(),
                    holds: "viewer",
                },
                Some(Hint::AskOwner),
            ),
            (BoardChangeError::NotEmpty { objects: 1 }, None),
            (
                BoardChangeError::NoObject { reference: text() },
                Some(Hint::ListObjects),
            ),
            (
                BoardChangeError::NotChangeable {
                    kind: text(),
                    field: "text",
                },
                None,
            ),
            (
                BoardChangeError::PlacedByContainer {
                    held: "task-card",
                    container: "kanban",
                },
                None,
            ),
            (BoardChangeError::Locked { reference: text() }, None),
            (BoardChangeError::NotYours { reference: text() }, None),
            (BoardChangeError::NothingToChange, None),
            (BoardChangeError::NoCheckItem { item: 1, count: 0 }, None),
            (BoardChangeError::NotPoint { raw: text() }, None),
            (
                BoardChangeError::OffBoard {
                    what: OffBoardValue::At(GivenPoint { x: 0.0, y: 0.0 }),
                },
                None,
            ),
            (BoardChangeError::NotSize { raw: text() }, None),
            (BoardChangeError::NotColor { raw: text() }, None),
            (
                BoardChangeError::NotArrowLook {
                    raw: text(),
                    expected: "x",
                },
                None,
            ),
            (
                BoardChangeError::NotShapeKind {
                    raw: text(),
                    known: "ellipse",
                },
                None,
            ),
            (
                BoardChangeError::StickerSize {
                    width: 1.0,
                    height: 3.0,
                },
                None,
            ),
        ]
    }

    fn ack_samples() -> Vec<(BoardChangeError, Option<Hint>)> {
        let text = || String::from("x");
        vec![
            (
                BoardChangeError::NotFrame { reference: text() },
                Some(Hint::ListObjects),
            ),
            (BoardChangeError::ContentHidden { reference: text() }, None),
            (BoardChangeError::TooFewToGroup { given: 1 }, None),
            (
                BoardChangeError::NotGroupable {
                    reference: text(),
                    why: GroupRefusal::Frame,
                },
                None,
            ),
            (
                BoardChangeError::NotGroup { reference: text() },
                Some(Hint::ListObjects),
            ),
            (
                BoardChangeError::LockWithheld {
                    needs: text(),
                    holds: "editor",
                },
                Some(Hint::AskOwner),
            ),
            (
                BoardChangeError::HideWithheld {
                    needs: text(),
                    holds: "editor",
                },
                Some(Hint::AskOwner),
            ),
            (
                BoardChangeError::NotLockable {
                    reference: text(),
                    why: LockRefusal::Held,
                },
                None,
            ),
            (
                BoardChangeError::NotLocked { reference: text() },
                Some(Hint::ListObjects),
            ),
            (BoardChangeError::NothingToUnlock, None),
            (
                BoardChangeError::OutsideFrame {
                    reference: text(),
                    frame: GivenRectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    at: GivenPoint { x: 2.0, y: 2.0 },
                },
                Some(Hint::ListObjects),
            ),
            (
                BoardChangeError::AcceptTimeout { seconds: 1 },
                Some(Hint::CheckBeforeRetry),
            ),
            (
                BoardChangeError::ClosedBeforeAccept,
                Some(Hint::CheckBeforeRetry),
            ),
            (
                BoardChangeError::NotAccepted {
                    update: 1,
                    objects: Vec::new(),
                    cause: Box::new(Error::from(BoardChangeError::ClosedBeforeAccept)),
                },
                Some(Hint::ReadBeforeRetry),
            ),
        ]
    }

    #[test]
    fn not_empty_names_the_merge_of_each_surface() {
        let err = Error::from(BoardChangeError::NotEmpty { objects: 3 });
        let cli = err.on(Surface::Cli);
        let mcp = err.on(Surface::Mcp);
        assert!(cli.contains("--merge"), "{cli}");
        assert!(mcp.contains("merge: true"), "{mcp}");
        assert!(!cli.contains("--force") && !mcp.contains("--force"));
        assert!(!mcp.contains("--"), "{mcp}");
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, BoardChangeErrorDiscriminants>(
            samples(),
            BoardChangeError::hint,
        );
    }
}
