//! The values a change is made of: new objects, patches and their effects.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        board::{Object, ObjectKind, shape::ShapeKind},
        change::receipt::Delivery,
        geometry::{Point, Size},
        id_source::IdSource,
        palette::Color,
        response::LinkPreview,
        rich_text::{Kept, TextFormat},
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
        kind: ShapeKind,
    },
    Frame {
        title: String,
        at: Point,
        size: Option<Size>,
        color: Option<Color>,
    },
    Arrow {
        from: String,
        to: String,
        color: Option<Color>,
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
}

impl NewObject {
    /// The same object moved by `dx`, `dy`; an arrow has no corner to move.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotFinite`] when the corner
    /// overflows.
    pub fn shifted(&self, dx: f64, dy: f64) -> Result<NewObject> {
        let moved = |at: &Point| at.shifted(dx, dy);
        Ok(match self {
            NewObject::Sticker {
                text,
                at,
                size,
                color,
            } => NewObject::Sticker {
                text: text.clone(),
                at: moved(at)?,
                size: *size,
                color: color.clone(),
            },
            NewObject::Text { text, at } => NewObject::Text {
                text: text.clone(),
                at: moved(at)?,
            },
            NewObject::Shape {
                text,
                at,
                size,
                color,
                kind,
            } => NewObject::Shape {
                text: text.clone(),
                at: moved(at)?,
                size: *size,
                color: color.clone(),
                kind: *kind,
            },
            NewObject::Frame {
                title: label,
                at,
                size,
                color,
            } => NewObject::Frame {
                title: label.clone(),
                at: moved(at)?,
                size: *size,
                color: color.clone(),
            },
            NewObject::Arrow { .. } => self.clone(),
            NewObject::Link {
                url,
                at,
                preview,
                embed,
            } => NewObject::Link {
                url: url.clone(),
                at: moved(at)?,
                preview: preview.clone(),
                embed: *embed,
            },
            NewObject::Image { upload, at, size } => NewObject::Image {
                upload: upload.clone(),
                at: moved(at)?,
                size: *size,
            },
            NewObject::File { upload, at } => NewObject::File {
                upload: upload.clone(),
                at: moved(at)?,
            },
        })
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
}

/// Where an object stands among the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum LayerOrder {
    Front,
    Back,
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

/// One change to a board.
#[derive(Debug, Clone, PartialEq)]
pub enum BoardChange {
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
            board::shape::ShapeKind,
            change::value::{ImageUpload, NewObject, Upload},
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
        kind: ShapeKind::Ellipse,
    })]
    #[case::frame(|at| NewObject::Frame {
        title: "a".into(),
        at,
        size: Some(Size::FRAME),
        color: "blue3".parse().ok(),
    })]
    #[case::arrow(|_| NewObject::Arrow {
        from: "a".into(),
        to: "b".into(),
        color: "gray8".parse().ok(),
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
    fn shifting_moves_the_corner_and_keeps_the_rest(
        #[case] make: fn(Point) -> NewObject,
    ) -> Result<()> {
        let moved = make(Point::new(1.0, 2.0)?).shifted(10.0, 5.0)?;
        assert_eq!(moved, make(Point::new(11.0, 7.0)?));
        Ok(())
    }
}
