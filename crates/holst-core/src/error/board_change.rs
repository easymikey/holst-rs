//! Changes to a board's objects and how the server took them.

use std::fmt;

use holst_macros::Hint;
use itertools::Itertools as _;

use crate::{
    domain::{
        board::{ObjectKind, role::BoardRole},
        change::kind_edit::Track,
    },
    error::Error,
};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub enum GroupRefusal {
    #[display("a frame holds its content itself")]
    Frame,
    #[display("a mind-map node stays in its map")]
    MindMapNode,
    #[display("a table cell stays in its table")]
    TableCell,
    #[display("it is in a group already; ungroup that one first")]
    InGroup,
    #[display("a card in a kanban column stays in it")]
    InColumn,
}

/// Why an object cannot be locked, as the web client's menu refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub enum LockRefusal {
    #[display("it is locked already")]
    Held,
    #[display("only locks frames and tables, nothing else")]
    NotFrameOrTable,
    #[display("{_0}")]
    Group(GroupRefusal),
}

/// Why an object cannot move into or out of a frame.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::Display)]
pub enum MoveRefusal {
    #[display("it follows the objects it joins")]
    FollowsJoined,
    #[display("it stands where its members do")]
    StandsWithMembers,
    #[display("it is held by a {_0}")]
    HeldBy(ObjectKind),
    #[display("the frame lies inside it")]
    FrameInside,
}

/// The field of an object an edit asks for, as `object edit` words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum ObjectField {
    Text,
    #[strum(serialize = "text format")]
    TextFormat,
    Position,
    Size,
    Color,
    Width,
    Checklist,
    Marks,
    #[strum(serialize = "text color")]
    TextColor,
    #[strum(serialize = "text background")]
    TextBackground,
    Rotation,
    Scale,
    Tags,
    #[strum(serialize = "stored fields")]
    StoredFields,
    Shape,
    Stroke,
    #[strum(serialize = "stroke-width")]
    StrokeWidth,
    #[strum(serialize = "stroke-style")]
    StrokeStyle,
    Radius,
    Opacity,
    Line,
    Heads,
    Bend,
    From,
    To,
    Label,
    Ratio,
    Background,
    Content,
    Name,
    Crop,
    Flip,
    Path,
    Display,
    #[strum(serialize = "start_page")]
    StartPage,
    #[strum(serialize = "hide_details")]
    HideDetails,
    Reactions,
    Roll,
    Faces,
    Children,
    Back,
    Code,
    #[strum(serialize = "cell text")]
    CellText,
    #[strum(serialize = "rows and columns")]
    RowsAndColumns,
    #[strum(serialize = "merged cells")]
    MergedCells,
    #[strum(serialize = "cell content")]
    CellContent,
    #[strum(serialize = "cell color")]
    CellColor,
    #[strum(serialize = "text size")]
    TextSize,
    Author,
    #[strum(serialize = "text size and author")]
    TextSizeAndAuthor,
    Kanban,
    Column,
    Swimlane,
    Status,
    Start,
    Due,
    Description,
    Assignees,
}

/// What a file lacks when it refuses a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub enum FileState {
    #[display("file without the info to show its content")]
    NoContentInfo,
    #[display("file without page info")]
    NoPageInfo,
    #[display("file that is not a video")]
    NotVideo,
    #[display("video shown as a stamp")]
    VideoAsStamp,
}

/// Why a table cannot take an edit, as the web client's menu refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub enum TableRefusal {
    #[display("{track} {at} is not in the table: use 1 to {most}")]
    OutOfRange { track: Track, at: u32, most: usize },
    #[display("a table holds at most 2500 cells")]
    TooBig,
    #[display(
        "a merged block crosses {track} {at}: unmerge it in the web client first"
    )]
    MergedAcross { track: Track, at: u32 },
    #[display(
        "a merged block crosses the place of new {track} {at}: unmerge it in the web client first"
    )]
    MergedBetween { track: Track, at: u32 },
    #[display("{track} {at} is the last {track}: remove the table instead")]
    LastTrack { track: Track, at: u32 },
    #[display("merging takes at least two cells")]
    OneCell,
    #[display(
        "a merged block reaches out of cells {},{} to {},{}: take all of it or none",
        first.0, first.1, last.0, last.1
    )]
    Ragged { first: (u32, u32), last: (u32, u32) },
    #[display("cell {row},{column} does not lead a merged block")]
    NotMerged { row: u32, column: u32 },
}

/// A board change that was refused, malformed or not accepted.
#[derive(Debug, thiserror::Error, Hint)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum BoardChangeError {
    #[error("the board is read-only for you: nothing was changed")]
    #[hint(AskOwner)]
    ReadOnly,

    #[error(
        "copying out of this board is held to {needs} and above: your right here is {holds}; \
         nothing was copied"
    )]
    #[hint(AskOwner)]
    CopyWithheld { needs: BoardRole, holds: BoardRole },

    #[error(
        "locking and unlocking on this board is held to {needs} and above: your right here \
         is {holds}; nothing was changed"
    )]
    #[hint(AskOwner)]
    LockWithheld { needs: BoardRole, holds: BoardRole },

    #[error(
        "hiding and showing frame content on this board is held to {needs} and above: your \
         right here is {holds}; nothing was changed"
    )]
    #[hint(AskOwner)]
    HideWithheld { needs: BoardRole, holds: BoardRole },

    #[error(
        "removing objects on this board is held to {needs} and above: your right here is \
         {holds}; a restore removes what the backup lacks, so nothing was changed"
    )]
    #[hint(AskOwner)]
    DeleteWithheld { needs: BoardRole, holds: BoardRole },

    #[error(
        "the target board already has {objects} objects: only an empty board takes an import; \
         nothing was changed"
    )]
    #[hint(MergeOrEmpty)]
    NotEmpty { objects: usize },

    #[error("\"{reference}\" does not name an object")]
    #[hint(NameObject)]
    NoObject { reference: String },

    #[error("{kind} objects have no {field} to change")]
    NotChangeable {
        kind: ObjectKind,
        field: ObjectField,
    },

    #[error("circle-cropped image objects have no {field} to change")]
    CroppedImage { field: ObjectField },

    #[error("{state} objects have no {field} to change")]
    UnshownFile {
        state: FileState,
        field: ObjectField,
    },

    #[error(
        "a wheel spins among at least 2 choices still in play: this one has {choices}"
    )]
    NotSpinnable { choices: usize },

    #[error("{kind} objects cannot move into or out of a frame: {why}")]
    NotMovable { kind: ObjectKind, why: MoveRefusal },

    #[error(
        "a {held} in a {container} has no position to change: the {container} places it"
    )]
    PlacedByContainer {
        held: ObjectKind,
        container: ObjectKind,
    },

    #[error(
        "a {held} in a {container} has no size to change: the {container} sizes it"
    )]
    SizedByContainer {
        held: ObjectKind,
        container: ObjectKind,
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

    #[error("\"{reference}\" does not name a tag of the board")]
    NoTag { reference: String },

    #[error("another tag of the board is already titled \"{title}\"")]
    TagTaken { title: String },

    #[error("the text has no checklist item {item}: it has {count}")]
    NoCheckItem { item: u32, count: usize },

    #[error("the file has no page {page}: it has {count}")]
    NoPage { page: u32, count: u32 },

    #[error("\"{raw}\" is not a point: expected x,y, e.g. 100,-40")]
    NotPoint { raw: String },

    #[error("{what} is off the board: coordinates and sizes go up to 1e12")]
    OffBoard { what: OffBoardValue },

    #[error("\"{raw}\" is not a size: expected WIDTH,HEIGHT, e.g. 192,192")]
    NotSize { raw: String },

    #[error(
        "\"{raw}\" is not a palette color: expected a name and a shade 1-12, e.g. yellow4"
    )]
    NotColor { raw: String },

    #[error(
        "\"{raw}\" is not a palette color: expected one of {}",
        shades_of(.family)
    )]
    NotShade { raw: String, family: &'static str },

    #[error("\"{raw}\" is not {expected}")]
    NotArrowLook { raw: String, expected: &'static str },
    #[error("\"{raw}\" is not a valid {field}: expected {expected}")]
    BadValue {
        field: &'static str,
        raw: String,
        expected: &'static str,
    },

    #[error("a sticker is square or twice as wide as tall, not {width}×{height}")]
    StickerSize { width: f64, height: f64 },

    #[error("\"{reference}\" is not a frame: only frames hold objects")]
    #[hint(ListObjects)]
    NotFrame { reference: String },

    #[error(
        "frame \"{reference}\" hides its content: objects put in it would vanish for viewers; \
         show its content in the web client first"
    )]
    ContentHidden { reference: String },

    #[error("the object at {at} would stick out of frame \"{reference}\" ({frame})")]
    #[hint(PositionInFrame)]
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
    #[hint(ListObjects)]
    NotGroup { reference: String },

    #[error("\"{reference}\" cannot be locked: {why}")]
    NotLockable { reference: String, why: LockRefusal },

    #[error(
        "\"{reference}\" is not locked itself: only a locked group, frame or table can be \
         unlocked"
    )]
    #[hint(ListObjects)]
    NotLocked { reference: String },

    #[error("nothing is locked on the board: nothing was changed")]
    NothingToUnlock,

    #[error("\"{reference}\" cannot be aligned or spread: {why}")]
    NotArrangeable {
        reference: String,
        why: GroupRefusal,
    },

    #[error(
        "arranging needs at least {needs} objects that can be moved, not {given}: nothing \
         was changed"
    )]
    TooFewToArrange { needs: usize, given: usize },

    #[error("{why}: nothing was changed")]
    TableRefused { why: TableRefusal },

    #[error(
        "the server did not accept the change within {seconds} s — it may or may not be on \
         the board"
    )]
    #[hint(CheckBeforeRetry)]
    AcceptTimeout { seconds: u64 },

    #[error(
        "the board connection closed before the server accepted the change — it may or may \
         not be on the board"
    )]
    #[hint(CheckBeforeRetry)]
    ClosedBeforeAccept,

    #[error(
        "update {update}{} was sent but the server did not accept it — it may or may not be on \
         the board",
        objects_note(.objects)
    )]
    #[hint(ReadBeforeRetry)]
    NotAccepted {
        update: u64,
        objects: Vec<String>,
        #[source]
        cause: Box<Error>,
    },
}

fn shades_of(family: &str) -> String {
    (1..=12).map(|shade| format!("{family}{shade}")).join(", ")
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
    use rstest::rstest;

    use crate::{
        domain::{
            board::{ObjectKind, role::BoardRole},
            change::kind_edit::Track,
        },
        error::{
            Error,
            board_change::{
                BoardChangeError,
                BoardChangeErrorDiscriminants,
                FileState,
                GivenPoint,
                GivenRectangle,
                GroupRefusal,
                LockRefusal,
                MoveRefusal,
                ObjectField,
                OffBoardValue,
                TableRefusal,
            },
            hint::{Surface, decided::assert_all_listed},
        },
    };

    pub(in crate::error) fn samples() -> Vec<BoardChangeError> {
        input_samples()
            .into_iter()
            .chain(value_samples())
            .chain(ack_samples())
            .collect()
    }

    fn input_samples() -> Vec<BoardChangeError> {
        let text = || String::from("x");
        vec![
            BoardChangeError::ReadOnly,
            BoardChangeError::CopyWithheld {
                needs: BoardRole::Owner,
                holds: BoardRole::Viewer,
            },
            BoardChangeError::NotEmpty { objects: 1 },
            BoardChangeError::NoObject { reference: text() },
            BoardChangeError::NotChangeable {
                kind: ObjectKind::Sticker,
                field: ObjectField::Text,
            },
            BoardChangeError::CroppedImage {
                field: ObjectField::Radius,
            },
            BoardChangeError::UnshownFile {
                state: FileState::NotVideo,
                field: ObjectField::HideDetails,
            },
            BoardChangeError::NotSpinnable { choices: 1 },
            BoardChangeError::TableRefused {
                why: TableRefusal::TooBig,
            },
            BoardChangeError::NotMovable {
                kind: ObjectKind::Arrow,
                why: MoveRefusal::FollowsJoined,
            },
            BoardChangeError::PlacedByContainer {
                held: ObjectKind::Task,
                container: ObjectKind::Other("kanban".into()),
            },
            BoardChangeError::SizedByContainer {
                held: ObjectKind::Task,
                container: ObjectKind::Other("kanban".into()),
            },
        ]
    }

    fn value_samples() -> Vec<BoardChangeError> {
        let text = || String::from("x");
        vec![
            BoardChangeError::Locked { reference: text() },
            BoardChangeError::NotYours { reference: text() },
            BoardChangeError::NothingToChange,
            BoardChangeError::NoTag { reference: text() },
            BoardChangeError::TagTaken { title: text() },
            BoardChangeError::NoCheckItem { item: 1, count: 0 },
            BoardChangeError::NoPage { page: 4, count: 3 },
            BoardChangeError::NotPoint { raw: text() },
            BoardChangeError::OffBoard {
                what: OffBoardValue::At(GivenPoint { x: 0.0, y: 0.0 }),
            },
            BoardChangeError::NotSize { raw: text() },
            BoardChangeError::NotColor { raw: text() },
            BoardChangeError::NotShade {
                raw: text(),
                family: "red",
            },
            BoardChangeError::NotArrowLook {
                raw: text(),
                expected: "x",
            },
            BoardChangeError::BadValue {
                field: "x",
                raw: text(),
                expected: "x",
            },
            BoardChangeError::StickerSize {
                width: 1.0,
                height: 3.0,
            },
        ]
    }

    fn ack_samples() -> Vec<BoardChangeError> {
        let text = || String::from("x");
        vec![
            BoardChangeError::NotFrame { reference: text() },
            BoardChangeError::ContentHidden { reference: text() },
            BoardChangeError::TooFewToGroup { given: 1 },
            BoardChangeError::NotGroupable {
                reference: text(),
                why: GroupRefusal::Frame,
            },
            BoardChangeError::NotGroup { reference: text() },
            BoardChangeError::LockWithheld {
                needs: BoardRole::Owner,
                holds: BoardRole::Editor,
            },
            BoardChangeError::HideWithheld {
                needs: BoardRole::Owner,
                holds: BoardRole::Editor,
            },
            BoardChangeError::DeleteWithheld {
                needs: BoardRole::Owner,
                holds: BoardRole::Editor,
            },
            BoardChangeError::NotLockable {
                reference: text(),
                why: LockRefusal::Held,
            },
            BoardChangeError::NotLockable {
                reference: text(),
                why: LockRefusal::NotFrameOrTable,
            },
            BoardChangeError::NotLocked { reference: text() },
            BoardChangeError::NothingToUnlock,
            BoardChangeError::NotArrangeable {
                reference: text(),
                why: GroupRefusal::TableCell,
            },
            BoardChangeError::TooFewToArrange { needs: 2, given: 1 },
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
            BoardChangeError::AcceptTimeout { seconds: 1 },
            BoardChangeError::ClosedBeforeAccept,
            BoardChangeError::NotAccepted {
                update: 1,
                objects: Vec::new(),
                cause: Box::new(Error::from(BoardChangeError::ClosedBeforeAccept)),
            },
        ]
    }

    #[rstest]
    #[case::lock_group(
        LockRefusal::Group(GroupRefusal::Frame),
        "a frame holds its content itself"
    )]
    #[case::move_held(MoveRefusal::HeldBy(ObjectKind::Table), "it is held by a table")]
    #[case::table_range(
        TableRefusal::OutOfRange { track: Track::Row, at: 9, most: 3 },
        "row 9 is not in the table: use 1 to 3"
    )]
    #[case::table_ragged(
        TableRefusal::Ragged { first: (1, 2), last: (3, 4) },
        "a merged block reaches out of cells 1,2 to 3,4: take all of it or none"
    )]
    fn refusals_say_their_reason(
        #[case] refusal: impl std::fmt::Display,
        #[case] said: &str,
    ) {
        assert_eq!(refusal.to_string(), said);
    }

    #[rstest]
    #[case::sized(
        BoardChangeError::SizedByContainer {
            held: ObjectKind::Task,
            container: ObjectKind::Other("kanban".into()),
        },
        "a task-card in a kanban has no size to change: the kanban sizes it"
    )]
    #[case::not_color(
        BoardChangeError::NotColor { raw: "black4".into() },
        "\"black4\" is not a palette color: expected a name and a shade 1-12, e.g. yellow4"
    )]
    #[case::not_shade(
        BoardChangeError::NotShade { raw: "red13".into(), family: "red" },
        "\"red13\" is not a palette color: expected one of red1, red2, red3, red4, red5, red6, red7, red8, red9, red10, red11, red12"
    )]
    fn the_new_refusals_say_what_was_wrong(
        #[case] error: BoardChangeError,
        #[case] said: &str,
    ) {
        assert_eq!(error.to_string(), said);
    }

    #[test]
    fn not_empty_and_no_object_say_the_fact_alone() {
        let said = [
            BoardChangeError::NotEmpty { objects: 3 }.to_string(),
            BoardChangeError::NoObject {
                reference: "z".into(),
            }
            .to_string(),
        ];
        assert_eq!(
            said,
            [
                "the target board already has 3 objects: only an empty board takes an import; nothing was changed",
                "\"z\" does not name an object",
            ]
        );
    }

    #[test]
    fn not_empty_names_the_merge_of_each_surface() {
        let hint = BoardChangeError::NotEmpty { objects: 3 }.hint();
        let on = |surface| hint.map(|hint| hint.text(surface));
        assert!(on(Surface::Cli).is_some_and(|text| text.contains("--merge")));
        assert!(on(Surface::Mcp).is_some_and(|text| text.contains("merge: true")));
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_all_listed::<_, BoardChangeErrorDiscriminants>(&samples());
    }
}
