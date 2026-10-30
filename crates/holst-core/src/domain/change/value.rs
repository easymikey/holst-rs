//! The values a change is made of: new objects, patches and their effects.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::{
        board::{
            Object,
            ObjectKind,
            arrow_look::{ArrowEndEdit, ArrowHeads, ArrowType, Bend, Label},
            frame_look::{FrameBackground, FrameRatio},
            shape_type::{ShapeType, StrokeStyle},
        },
        change::{kind_edit::KindEdit, receipt::Delivery},
        geometry::{Point, Size},
        id_source::IdSource,
        palette::Color,
        response::LinkPreview,
        rich_text::{
            Editor,
            Kept,
            Mark,
            Parsed,
            TextFormat,
            lines,
            parse,
            template::NoteTemplate,
        },
        text::joined,
        time::Timestamp,
    },
    error::Result,
};

/// A file the server holds, as the upload left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    /// The name the server gave it.
    pub name: String,
    /// The name it had on disk.
    pub original: String,
    /// Its MIME type.
    pub mime: String,
    pub bytes: u64,
}

/// An uploaded picture and its size in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageUpload {
    pub file: Upload,
    pub pixels: Size,
}

/// A new object and what it starts with.
#[derive(Debug, Clone, PartialEq)]
pub enum NewObject {
    Sticker {
        text: String,
        at: Point,
        size: Option<Size>,
        color: Option<Color>,
    },
    Text {
        text: String,
        at: Point,
    },
    Shape {
        text: String,
        at: Point,
        size: Option<Size>,
        color: Option<Color>,
        kind: ShapeType,
    },
    Frame {
        title: String,
        at: Point,
        size: Option<Size>,
        color: Option<Color>,
        look: FrameEdit,
    },
    Arrow {
        from: String,
        to: String,
        color: Option<Color>,
        look: ArrowLook,
    },
    /// A link card; an embedded one when `embed` and the preview offers it.
    Link {
        url: String,
        at: Point,
        preview: Option<LinkPreview>,
        embed: bool,
    },
    /// An uploaded picture, shown at `size` or at its pixels.
    Image {
        upload: ImageUpload,
        at: Point,
        size: Option<Size>,
    },
    File {
        upload: Upload,
        at: Point,
    },
    /// An object as the web client stores it, built by its kind's own code.
    Made(Made),
}

/// A new object of a kind the web client stores with more than its own map.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    /// The stored type; `Other` for kanban, mind-map-node, ...
    pub kind: ObjectKind,
    /// From the parent's corner when `parent` is given.
    pub at: Point,
    /// For the free corner and the frame check only.
    pub size: Option<Size>,
    /// A holder that is not a frame; no frame search then.
    pub parent: Option<String>,
    /// Besides the header, `position` and `parentId`.
    pub fields: Vec<(&'static str, Value)>,
    /// Each a new text document, its id stored under that field.
    pub texts: Vec<(&'static str, String)>,
    /// Code, in the root `Y.Text` `code-mirror-<documentId>`.
    pub code: Option<String>,
    /// The objects it holds, `parentId` = the new object.
    pub parts: Vec<Made>,
    /// The board's stores it adds to.
    pub entries: Vec<StoreEntry>,
}

/// An entry of one of the board's stores, a root map named `store`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreEntry {
    /// The root map's name, e.g. `kanban-infoStore`.
    pub store: &'static str,
    /// Drawn by the kind's code through the `new_id` closure it is given.
    pub id: String,
    pub fields: Vec<(&'static str, Value)>,
    /// Written as nested `Y.Array`s (kanban columns, tag ids).
    pub lists: Vec<(&'static str, Vec<Value>)>,
    /// Written only when the store has no entry `id` (the default statuses).
    pub keep: bool,
}

impl NewObject {
    /// The same object moved by `dx`, `dy`; an arrow has no corner to move.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotFinite`] when the corner
    /// overflows.
    pub fn shifted(&self, dx: f64, dy: f64) -> Result<NewObject> {
        let mut moved = self.clone();
        let corner = match &mut moved {
            NewObject::Sticker { at, .. }
            | NewObject::Text { at, .. }
            | NewObject::Shape { at, .. }
            | NewObject::Frame { at, .. }
            | NewObject::Link { at, .. }
            | NewObject::Image { at, .. }
            | NewObject::File { at, .. }
            | NewObject::Made(Made {
                at, parent: None, ..
            }) => Some(at),
            NewObject::Arrow { .. } | NewObject::Made(_) => None,
        };
        corner
            .map(|at| at.shifted(dx, dy).map(|to| *at = to))
            .transpose()?;
        Ok(moved)
    }
}

/// What an edit changes; at least one field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjectPatch {
    pub text: Option<String>,
    /// How `text` is written.
    pub format: TextFormat,
    pub at: Option<Point>,
    pub size: Option<Size>,
    pub color: Option<Color>,
    /// Clockwise degrees to turn to; the position follows, keeping the centre.
    pub rotation: Option<f64>,
    /// The factor a scaled kind or a text is drawn at.
    pub scale: Option<f64>,
    pub link_to: Option<LinkTo>,
    pub width: Option<TextWidth>,
    pub tick: Option<Tick>,
    pub look: TextLook,
    pub shape: ShapeEdit,
    pub arrow: ArrowEdit,
    pub frame: FrameEdit,
    pub reparent: Option<Reparent>,
    /// What only some kinds take; none for now.
    pub kind_edits: Vec<KindEdit>,
}

/// Where an edit puts an object: into a frame, or out of its frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reparent {
    /// Onto the board.
    Board,
    /// A frame: id, link, title or part of a title; the id once checked.
    Frame(String),
}

impl Reparent {
    /// The id of the frame once checked; none for the board.
    #[must_use]
    pub fn frame(&self) -> Option<&str> {
        match self {
            Reparent::Board => None,
            Reparent::Frame(frame) => Some(frame),
        }
    }
}

/// How a text wraps its lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextWidth {
    /// By its content, as before it was widened.
    Auto,
    /// This wide as drawn, in board units; above 0.
    Fixed(f64),
}

/// A checklist item to tick or untick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// From 1, in reading order.
    pub item: NonZeroU32,
    pub done: bool,
}

/// What an object opens when clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTo {
    /// Takes the link away.
    Clear,
    /// A web address, scheme included.
    Url(String),
    /// Another object of the board; `share` plus `?objectId=` is its link.
    Object { reference: String, share: String },
}

/// What stands behind the text of an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backdrop {
    Clear,
    Color(Color),
}

/// How the whole text of an object looks; a field left out stays as it is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextLook {
    /// The marks the text carries and no others; empty clears them.
    pub marks: Option<Vec<Mark>>,
    pub color: Option<Color>,
    pub background: Option<Backdrop>,
}

/// How a new arrow is drawn: its line and heads, its dash and width.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArrowLook {
    pub arrow: ArrowEdit,
    pub outline: ShapeEdit,
}

/// What an edit changes of an arrow's line, heads, bend, ends and labels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArrowEdit {
    pub line: Option<ArrowType>,
    pub heads: ArrowHeads,
    pub bend: Option<Bend>,
    pub from: Option<ArrowEndEdit>,
    pub to: Option<ArrowEndEdit>,
    /// All the labels the arrow has; empty leaves them as they are.
    pub labels: Vec<Label>,
}

impl ArrowEdit {
    /// The names of the fields asked for, as `object edit` words them.
    #[must_use]
    pub fn fields(&self) -> Vec<&'static str> {
        [
            ("line", self.line.is_some()),
            (
                "heads",
                self.heads.start.is_some() || self.heads.end.is_some(),
            ),
            ("bend", self.bend.is_some()),
            ("from", self.from.is_some()),
            ("to", self.to.is_some()),
            ("label", !self.labels.is_empty()),
        ]
        .into_iter()
        .filter_map(|(name, given)| given.then_some(name))
        .collect()
    }
}

/// A frame's ratio, background and hidden content, given or changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameEdit {
    pub ratio: Option<FrameRatio>,
    pub background: Option<FrameBackground>,
    pub hidden: Option<bool>,
}

impl FrameEdit {
    /// The names of the fields asked for, as `object edit` words them.
    #[must_use]
    pub fn fields(&self) -> Vec<&'static str> {
        [
            ("ratio", self.ratio.is_some()),
            ("background", self.background.is_some()),
            ("content", self.hidden.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, given)| given.then_some(name))
        .collect()
    }
}

/// What an edit changes of a shape: its kind and look.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShapeEdit {
    pub kind: Option<ShapeType>,
    pub stroke: Option<Color>,
    /// A whole 0 to 10 for a shape, one of the arrow widths for an arrow.
    pub stroke_width: Option<f64>,
    pub stroke_style: Option<StrokeStyle>,
    pub radius: Option<u32>,
    /// 0.1 to 1, checked by the caller.
    pub opacity: Option<f64>,
}

impl ShapeEdit {
    /// The name of the first field asked for, as `object edit` words it.
    #[must_use]
    pub fn first_field(&self) -> Option<&'static str> {
        self.fields().first().copied()
    }

    /// The names of the fields asked for, as `object edit` words them.
    #[must_use]
    pub fn fields(&self) -> Vec<&'static str> {
        [
            ("shape", self.kind.is_some()),
            ("stroke", self.stroke.is_some()),
            ("stroke-width", self.stroke_width.is_some()),
            ("stroke-style", self.stroke_style.is_some()),
            ("radius", self.radius.is_some()),
            ("opacity", self.opacity.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, given)| given.then_some(name))
        .collect()
    }
}

/// Where an object stands among the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum LayerOrder {
    Front,
    Back,
    /// One step toward the front, past the next overlapping object.
    Forward,
    /// One step toward the back, past the next overlapping object.
    Backward,
}

/// Where aligned objects meet in the box they cover together.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Alignment {
    Left,
    /// Centred between the left and right edges.
    Center,
    Right,
    Top,
    /// Centred between the top and bottom edges.
    Middle,
    Bottom,
}

/// The direction objects are spread along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// How objects are laid out against each other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrangement {
    /// Every object to one side of the box they cover together.
    Align(Alignment),
    /// The gaps between the objects made equal, the outer two staying.
    Distribute(Axis),
}

/// One object of an edit of many: the patch and what it cannot hold.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectEdit {
    pub object: String,
    pub patch: ObjectPatch,
    pub stacking: Option<LayerOrder>,
    /// The tags by title; empty clears them, none leaves them.
    pub tags: Option<Vec<String>>,
}

/// Objects taken from a board to put on a board, and by how much they move.
#[derive(Debug, Clone, PartialEq)]
pub struct Pasted {
    /// The copies' sources: frames first, back to front, then the others.
    pub objects: Vec<Object>,
    /// The source board's tags: id to title.
    pub tags: BTreeMap<String, String>,
    pub shift: Point,
}

/// A new object of an add of many; an arrow end `#N` is an earlier entry.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntry {
    pub new: NewObject,
    pub frame: Option<String>,
    /// How the object's text is written.
    pub format: TextFormat,
}

/// What a board's note becomes: a text, or one of the built-in notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteContent {
    Text { text: String, format: TextFormat },
    Template(NoteTemplate),
}

impl NoteContent {
    /// The blocks the note is written as, and what of the markdown stays text.
    #[must_use]
    pub fn parsed(&self) -> Parsed {
        match self {
            NoteContent::Text {
                text,
                format: TextFormat::Markdown,
            } => parse(text, &Editor::note()),
            NoteContent::Text {
                text,
                format: TextFormat::Plain,
            } => Parsed {
                blocks: lines(text),
                kept: BTreeSet::new(),
            },
            NoteContent::Template(template) => Parsed {
                blocks: template.blocks(),
                kept: BTreeSet::new(),
            },
        }
    }
}

/// One change to a board.
#[derive(Debug, Clone, PartialEq)]
pub enum BoardChange {
    /// The board's note, replaced.
    Note(NoteContent),
    /// New objects in one write, in order.
    AddAll(Vec<NewEntry>),
    EditAll(Vec<ObjectEdit>),
    RemoveAll(Vec<String>),
    /// Copies of objects, new ids and all.
    Paste(Pasted),
    /// The named objects, put into one new group.
    Group(Vec<String>),
    /// The named groups, dissolved; their members stay.
    Ungroup(Vec<String>),
    /// The named objects, put into one new locked group.
    Lock(Vec<String>),
    /// The named frames and tables, flagged locked; nothing is grouped.
    LockOnly(Vec<String>),
    /// Locked groups dissolved, locked frames and tables unlocked.
    Unlock(Vec<String>),
    /// Everything locked on the board, unlocked.
    UnlockAll,
    /// The named objects, each with what it holds, moved against each other.
    Arrange {
        objects: Vec<String>,
        how: Arrangement,
    },
    /// A tag of the board, by title or id, renamed, recolored or removed.
    Tag {
        tag: String,
        change: TagChange,
    },
}

/// What is done to a tag definition; a removal wins over the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagChange {
    pub name: Option<String>,
    pub color: Option<Color>,
    /// Delete the tag and take its id off every sticker and task card.
    pub remove: bool,
}

/// What happened to an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectChangeKind {
    Created,
    Changed,
    Removed,
}

impl ObjectChangeKind {
    /// `created`, `changed` or `removed`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ObjectChangeKind::Created => "created",
            ObjectChangeKind::Changed => "changed",
            ObjectChangeKind::Removed => "removed",
        }
    }

    /// The verb a report starts with: `created`, or `would create` when dry.
    #[must_use]
    pub const fn verb(self, delivery: Delivery) -> &'static str {
        match (delivery, self) {
            (Delivery::Built, ObjectChangeKind::Created) => "would create",
            (Delivery::Built, ObjectChangeKind::Changed) => "would change",
            (Delivery::Built, ObjectChangeKind::Removed) => "would remove",
            (Delivery::Accepted, effect) => effect.as_str(),
        }
    }
}

/// An object the change touches, with its web link once a link is known.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangedObject {
    pub id: String,
    pub kind: ObjectKind,
    pub change: ObjectChangeKind,
    pub fields: Vec<&'static str>,
    pub link: Option<String>,
}

impl ChangedObject {
    /// A new object, with no fields to name.
    #[must_use]
    pub fn created(id: String, kind: ObjectKind) -> Self {
        Self {
            id,
            kind,
            change: ObjectChangeKind::Created,
            fields: Vec::new(),
            link: None,
        }
    }

    /// A deleted object, with no fields to name.
    #[must_use]
    pub fn removed(id: String, kind: ObjectKind) -> Self {
        Self {
            change: ObjectChangeKind::Removed,
            ..Self::created(id, kind)
        }
    }

    /// `self` with the link `link` gives its id, unless it is removed.
    #[must_use]
    pub fn with_link(&self, link: impl Fn(&str) -> String) -> Self {
        let link = match self.change {
            ObjectChangeKind::Removed => None,
            ObjectChangeKind::Created | ObjectChangeKind::Changed => {
                Some(link(&self.id))
            }
        };
        Self {
            link,
            ..self.clone()
        }
    }

    /// `: text, position` after the verb, or nothing without fields.
    #[must_use]
    pub fn fields_note(&self) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match self.fields.as_slice() {
            [] => Ok(()),
            fields => write!(f, ": {}", joined(fields, ", ")),
        })
    }
}

/// A change built against a loaded document: the update and what it touches.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeUpdate {
    pub update: Vec<u8>,
    pub objects: Vec<ChangedObject>,
    /// What of the markdown written stayed text.
    pub kept: BTreeSet<Kept>,
}

/// Whose objects a removal may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveRight {
    Anyone,
    /// Only what the user made, and what goes with it.
    OwnOnly,
}

/// What a change needs from the outside: the time and fresh ids.
pub struct ChangeSources<'a> {
    pub built_at: Timestamp,
    pub ids: &'a dyn IdSource,
}

impl fmt::Debug for ChangeSources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChangeSources")
            .field("now", &self.built_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use rstest::rstest;

    use crate::{
        domain::{
            board::{ObjectKind, shape_type::ShapeType},
            change::value::{
                ArrowLook,
                FrameEdit,
                ImageUpload,
                Made,
                NewObject,
                Upload,
            },
            geometry::{Point, Size},
            response::LinkPreview,
        },
        error::Result,
    };

    fn upload() -> Upload {
        Upload {
            name: "n.png".into(),
            original: "o.png".into(),
            mime: "image/png".into(),
            bytes: 7,
        }
    }

    #[rstest]
    #[case::sticker(|at| NewObject::Sticker {
        text: "a".into(),
        at,
        size: Some(Size::STICKER),
        color: "yellow4".parse().ok(),
    })]
    #[case::text(|at| NewObject::Text { text: "a".into(), at })]
    #[case::shape(|at| NewObject::Shape {
        text: "a".into(),
        at,
        size: Some(Size::SHAPE),
        color: "green4".parse().ok(),
        kind: ShapeType::Ellipse,
    })]
    #[case::frame(|at| NewObject::Frame {
        title: "a".into(),
        at,
        size: Some(Size::FRAME),
        color: "blue3".parse().ok(),
        look: FrameEdit::default(),
    })]
    #[case::arrow(|_| NewObject::Arrow {
        from: "a".into(),
        to: "b".into(),
        color: "gray8".parse().ok(),
        look: ArrowLook::default(),
    })]
    #[case::link(|at| NewObject::Link {
        url: "https://example.com".into(),
        at,
        preview: Some(LinkPreview::default()),
        embed: true,
    })]
    #[case::image(|at| NewObject::Image {
        upload: ImageUpload { file: upload(), pixels: Size::STICKER },
        at,
        size: Some(Size::SHAPE),
    })]
    #[case::file(|at| NewObject::File { upload: upload(), at })]
    #[case::made(|at| NewObject::Made(Made {
        kind: ObjectKind::Other("dice".into()),
        at,
        size: None,
        parent: None,
        fields: Vec::new(),
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }))]
    fn shifting_moves_the_corner_and_keeps_the_rest(
        #[case] make: fn(Point) -> NewObject,
    ) -> Result<()> {
        let moved = make(Point::new(1.0, 2.0)?).shifted(10.0, 5.0)?;
        assert_eq!(moved, make(Point::new(11.0, 7.0)?));
        Ok(())
    }
}
