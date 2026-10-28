//! Board objects from the `objects`, `documents` and `usersIds` Yjs maps.

pub mod comment;
pub mod detail;
pub mod query;
pub mod shape;
pub mod state_vector;
#[cfg(test)] pub(crate) mod tests;

use std::collections::HashMap;

use itertools::Itertools as _;
use serde::Serialize;
use serde_json::Value;
use yrs::{
    Any,
    Doc,
    Map as _,
    Out,
    ReadTxn,
    Text as _,
    Transact as _,
    Xml as _,
    XmlTextRef,
    types::{ToJson as _, text::YChange},
};

use crate::{
    domain::{
        area::scene::Scene,
        geometry::{Point, Size},
    },
    error::{Error, Result, input::InputError},
    protocol::document,
};

/// What an object is; types nothing handles specially keep their name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "kebab-case")]
pub enum ObjectKind {
    Sticker,
    StickerStack,
    Text,
    Shape,
    Document,
    Frame,
    Arrow,
    Image,
    File,
    Stamp,
    Group,
    Table,
    TableCell,
    Code,
    Drawing,
    Dice,
    Wheel,
    Flipcard,
    Task,
    Link,
    #[serde(untagged)]
    Other(String),
}

/// Every kind with a variant, in the order listings and errors name them.
const KNOWN: [ObjectKind; 20] = [
    ObjectKind::Sticker,
    ObjectKind::StickerStack,
    ObjectKind::Text,
    ObjectKind::Shape,
    ObjectKind::Document,
    ObjectKind::Frame,
    ObjectKind::Arrow,
    ObjectKind::Image,
    ObjectKind::File,
    ObjectKind::Stamp,
    ObjectKind::Group,
    ObjectKind::Table,
    ObjectKind::TableCell,
    ObjectKind::Code,
    ObjectKind::Drawing,
    ObjectKind::Dice,
    ObjectKind::Wheel,
    ObjectKind::Flipcard,
    ObjectKind::Task,
    ObjectKind::Link,
];

impl ObjectKind {
    /// The kind for a `type` field of the document.
    #[must_use]
    pub fn of(name: &str) -> Self {
        KNOWN
            .iter()
            .find(|kind| kind.as_str() == name)
            .cloned()
            .unwrap_or_else(|| ObjectKind::Other(name.to_owned()))
    }

    /// The name used in the document.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // PROTOCOL: object type names as stored.
        match self {
            ObjectKind::Sticker => "sticker",
            ObjectKind::StickerStack => "sticker-stack",
            ObjectKind::Text => "simple-text",
            ObjectKind::Shape => "shape",
            ObjectKind::Document => "card",
            ObjectKind::Frame => "frame",
            ObjectKind::Arrow => "arrow",
            ObjectKind::Image => "image",
            ObjectKind::File => "file",
            ObjectKind::Stamp => "stamp",
            ObjectKind::Group => "group",
            ObjectKind::Table => "table",
            ObjectKind::TableCell => "table-cell",
            ObjectKind::Code => "code",
            ObjectKind::Drawing => "drawing",
            ObjectKind::Dice => "dice",
            ObjectKind::Wheel => "spinner-wheel",
            ObjectKind::Flipcard => "flip-card",
            ObjectKind::Task => "task-card",
            ObjectKind::Link => "link",
            ObjectKind::Other(name) => name,
        }
    }

    /// The word a person reads and types (`text`, `document`, `flipcard`,
    /// `wheel`, `task`); a kind without a variant keeps its stored name.
    #[must_use]
    pub fn short_name(&self) -> &str {
        match self {
            ObjectKind::Text => "text",
            ObjectKind::Document => "document",
            ObjectKind::Flipcard => "flipcard",
            ObjectKind::Wheel => "wheel",
            ObjectKind::Task => "task",
            ObjectKind::Sticker
            | ObjectKind::StickerStack
            | ObjectKind::Shape
            | ObjectKind::Frame
            | ObjectKind::Arrow
            | ObjectKind::Image
            | ObjectKind::File
            | ObjectKind::Stamp
            | ObjectKind::Group
            | ObjectKind::Table
            | ObjectKind::TableCell
            | ObjectKind::Code
            | ObjectKind::Drawing
            | ObjectKind::Dice
            | ObjectKind::Link
            | ObjectKind::Other(_) => self.as_str(),
        }
    }

    /// The size of a new object, and of one stored without a size.
    #[must_use]
    pub const fn default_size(&self) -> Option<Size> {
        match self {
            ObjectKind::Sticker | ObjectKind::StickerStack => Some(Size::STICKER),
            ObjectKind::Shape => Some(Size::SHAPE),
            ObjectKind::Frame => Some(Size::FRAME),
            ObjectKind::Document => Some(Size::CARD),
            ObjectKind::Text
            | ObjectKind::Arrow
            | ObjectKind::Image
            | ObjectKind::File
            | ObjectKind::Stamp
            | ObjectKind::Group
            | ObjectKind::Table
            | ObjectKind::TableCell
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
}

impl ObjectKind {
    /// The kind a person types: a name as listings show it (`text`) or as the
    /// document stores it (`simple-text`), in any case. Any other name is
    /// [`ObjectKind::Other`]; a blank one is no kind.
    ///
    /// ```
    /// use holst_core::domain::board::ObjectKind;
    ///
    /// assert_eq!(ObjectKind::typed("Text"), Some(ObjectKind::Text));
    /// assert_eq!(
    ///     ObjectKind::typed(" Mind-Map-Node "),
    ///     Some(ObjectKind::Other("mind-map-node".into()))
    /// );
    /// assert_eq!(ObjectKind::typed("  "), None);
    /// ```
    #[must_use]
    pub fn typed(raw: &str) -> Option<Self> {
        let asked = raw.trim().to_ascii_lowercase();
        if asked.is_empty() {
            None
        } else {
            Some(
                KNOWN
                    .iter()
                    .find(|kind| kind.short_name() == asked || kind.as_str() == asked)
                    .cloned()
                    .unwrap_or(ObjectKind::Other(asked)),
            )
        }
    }

    /// Accepts a kind with a variant, or one of the `present` kinds.
    ///
    /// # Errors
    /// [`InputError::NotObjectKind`] for an [`ObjectKind::Other`] that is not
    /// among `present`; it lists the kinds with a variant and the `present`
    /// ones.
    pub fn check_among(
        &self,
        present: impl Iterator<Item = ObjectKind> + Clone,
    ) -> Result<()> {
        if KNOWN.contains(self) || present.clone().any(|kind| kind == *self) {
            Ok(())
        } else {
            Err(Error::Input(InputError::NotObjectKind {
                raw: self.as_str().to_owned(),
                known: KNOWN
                    .iter()
                    .cloned()
                    .chain(present)
                    .map(|kind| kind.short_name().to_owned())
                    .unique()
                    .collect(),
            }))
        }
    }
}

/// The two ends of an arrow: object ids, when attached.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArrowEnds {
    pub start: Option<String>,
    pub end: Option<String>,
    #[serde(skip)]
    pub drawn: Option<DrawnArrowEnds>,
}

/// Where an arrow's two ends are on the board, as it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DrawnArrowEnds {
    pub start: Point,
    pub end: Point,
}

/// One object of a board, as much as the listing needs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Object {
    pub id: String,
    pub kind: ObjectKind,
    pub parent: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub z_index: f64,
    pub text: String,
    pub color: Option<String>,
    pub ends: Option<ArrowEnds>,
    pub author: Option<String>,
    #[serde(skip)]
    pub file: Option<StoredFile>,
    #[serde(skip)]
    pub fields: Value,
}

/// The file behind an image or a file object, as the storage keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredFile {
    pub name: String,
    pub original: Option<String>,
}

/// Returns the stored file of an image (`name`) or a file object (`fileName`).
///
/// ```
/// use holst_core::domain::board::{ObjectKind, StoredFile, stored_file};
/// use serde_json::json;
///
/// let image = json!({"name": "a1.png", "originalName": "cat.png"});
/// assert_eq!(
///     stored_file(&ObjectKind::Image, &image),
///     Some(StoredFile {
///         name: "a1.png".into(),
///         original: Some("cat.png".into())
///     })
/// );
/// let file = json!({"fileName": "b2.docx", "displayFileName": "Plan.docx"});
/// assert!(stored_file(&ObjectKind::File, &file).is_some());
/// assert_eq!(stored_file(&ObjectKind::Sticker, &image), None);
/// ```
#[must_use]
pub fn stored_file(kind: &ObjectKind, fields: &Value) -> Option<StoredFile> {
    let (name, original) = [
        (ObjectKind::Image, (document::NAME, document::ORIGINAL_NAME)),
        (
            ObjectKind::File,
            (document::FILE_NAME, document::DISPLAY_FILE_NAME),
        ),
    ]
    .into_iter()
    .find_map(|(stored, columns)| (stored == *kind).then_some(columns))?;
    let text = |field: &str| {
        fields
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
    };
    Some(StoredFile {
        name: text(name)?,
        original: text(original),
    })
}

/// A board's objects and whether the user may change them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardContent {
    pub edit_right: EditRight,
    pub objects: Vec<Object>,
}

#[derive(Debug, Serialize)]
struct LinkedObject<'a> {
    #[serde(flatten)]
    object: &'a Object,
    link: String,
    #[serde(flatten)]
    drawn: Option<DrawnArrowEnds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<LinkedFile<'a>>,
}

#[derive(Debug, Serialize)]
struct LinkedFile<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    original: Option<&'a str>,
    url: String,
}

#[derive(Debug, Serialize)]
struct ObjectList<'a> {
    edit_right: EditRight,
    objects: Vec<LinkedObject<'a>>,
}

impl BoardContent {
    /// Returns the content as JSON: `access`, objects with links, arrows with
    /// ends, stored files with their address under `storage`.
    ///
    /// # Errors
    /// [`crate::error::http::HttpError::Json`] if serialization fails.
    pub fn linked(
        &self,
        link: impl Fn(&str) -> String,
        drawn_ends: impl Fn(&Object) -> Option<DrawnArrowEnds>,
        storage: &str,
    ) -> Result<Value> {
        let objects: Vec<LinkedObject<'_>> = self
            .objects
            .iter()
            .map(|object| LinkedObject {
                object,
                link: link(&object.id),
                drawn: drawn_ends(object),
                file: object.file.as_ref().map(|file| LinkedFile {
                    name: &file.name,
                    original: file.original.as_deref(),
                    url: format!("{storage}{}", file.name),
                }),
            })
            .collect();
        Ok(serde_json::to_value(ObjectList {
            edit_right: self.edit_right,
            objects,
        })?)
    }
}

/// Every object of a board document, in reading order: top to bottom, left to
/// right.
#[must_use]
pub fn objects(document: &Doc) -> Vec<Object> {
    let objects = document.get_or_insert_map(document::OBJECTS);
    let documents = document.get_or_insert_map(document::DOCUMENTS);
    let users = document.get_or_insert_map(document::USERS_IDS);
    let txn = document.transact();
    let authors: HashMap<u64, String> = users
        .iter(&txn)
        .filter_map(|(user, index)| {
            number(&index.to_json(&txn)).map(|index| (index, user.to_owned()))
        })
        .collect();
    let listed: Vec<Object> = objects
        .iter(&txn)
        .filter_map(|(_, value)| json(&value.to_json(&txn)))
        .filter_map(|fields| {
            object(
                fields,
                |document_id| text_of(&txn, &documents, document_id),
                &authors,
            )
        })
        .collect();
    // Arrows first: an attached end stores (0, 0), the drawn start is real.
    with_drawn_ends(listed)
        .into_iter()
        .sorted_by(|a, b| {
            let (a_x, a_y) = sorted_at(a);
            let (b_x, b_y) = sorted_at(b);
            // The id breaks ties: the map has no stable order.
            a_y.total_cmp(&b_y)
                .then(a_x.total_cmp(&b_x))
                .then_with(|| a.id.cmp(&b.id))
        })
        .collect()
}

/// Where an object sorts: its drawn arrow start, else its stored position.
fn sorted_at(object: &Object) -> (f64, f64) {
    object
        .ends
        .as_ref()
        .and_then(|ends| ends.drawn)
        .map_or((object.x, object.y), |drawn| {
            (drawn.start.x(), drawn.start.y())
        })
}

fn with_drawn_ends(mut objects: Vec<Object>) -> Vec<Object> {
    let scene = Scene::new(&objects);
    let points: Vec<Option<DrawnArrowEnds>> = objects
        .iter()
        .map(|object| scene.drawn_ends(object))
        .collect();
    objects.iter_mut().zip(points).for_each(|(object, points)| {
        object.ends.iter_mut().for_each(|ends| ends.drawn = points);
    });
    objects
}

pub(crate) fn object_of(fields: Value) -> Option<Object> {
    object(fields, |_| String::new(), &HashMap::new())
}

/// `fields` as an object, its text read from the `documents` map.
pub(crate) fn object_with_text<T: ReadTxn>(
    fields: Value,
    txn: &T,
    documents: &yrs::MapRef,
) -> Option<Object> {
    object(fields, |id| text_of(txn, documents, id), &HashMap::new())
}

/// Whether `fields` is an object a listing shows: a string `id` and `type`,
/// the two fields `object` cannot do without.
pub(crate) fn is_listed(fields: &Value) -> bool {
    [document::ID, document::TYPE]
        .iter()
        .all(|name| fields.get(name).is_some_and(Value::is_string))
}

fn object(
    fields: Value,
    text: impl Fn(&str) -> String,
    authors: &HashMap<u64, String>,
) -> Option<Object> {
    let field = |name: &str| fields.get(name);
    let float = |value: Option<&Value>| value.and_then(Value::as_f64);
    let kind = ObjectKind::of(field(document::TYPE)?.as_str()?);
    let note = field(document::DOCUMENT_ID)
        .and_then(Value::as_str)
        .map(&text)
        .or_else(|| field(document::TEXTS).map(arrow_text));
    let label = field(document::LABEL_TEXT)
        .and_then(Value::as_str)
        .map(str::to_owned);
    // An arrow keeps (0, 0) as its position; where it starts is the useful place.
    let at = if kind == ObjectKind::Arrow {
        field(document::START).and_then(|start| start.get(document::POINT))
    } else {
        field(document::POSITION)
    };

    Some(Object {
        id: field(document::ID)?.as_str()?.to_owned(),
        parent: field(document::PARENT_ID)
            .and_then(Value::as_str)
            .map(str::to_owned),
        x: float(at.and_then(|at| at.get(document::X))).unwrap_or_default(),
        y: float(at.and_then(|at| at.get(document::Y))).unwrap_or_default(),
        width: float(field(document::WIDTH)),
        height: float(field(document::HEIGHT)),
        z_index: float(field(document::Z_INDEX)).unwrap_or_default(),
        text: note.or(label).unwrap_or_default(),
        color: field(document::FILL_COLOR)
            .and_then(|fill| fill.get(document::COLOR))
            .and_then(Value::as_str)
            .map(str::to_owned),
        ends: matches!(kind, ObjectKind::Arrow).then(|| ArrowEnds {
            start: end_object(&fields, document::START),
            end: end_object(&fields, document::END),
            drawn: None,
        }),
        author: field(document::CREATED)
            .and_then(|created| created.get(document::AUTHOR_INDEX))
            .and_then(Value::as_u64)
            .and_then(|index| authors.get(&index).cloned()),
        file: stored_file(&kind, &fields),
        kind,
        fields,
    })
}

fn arrow_text(texts: &Value) -> String {
    let labels: Vec<String> = texts
        .as_array()
        .into_iter()
        .flatten()
        .map(|label| {
            label
                .get(document::EDITOR)
                .map(slate_text)
                .or_else(|| {
                    label
                        .get(document::TEXT)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default()
        })
        .filter(|label| !label.is_empty())
        .collect();
    labels.join("\n")
}

fn slate_text(nodes: &Value) -> String {
    match nodes {
        Value::Array(blocks) => blocks
            .iter()
            .map(slate_text)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(node) => node
            .get(document::TEXT)
            .and_then(Value::as_str)
            .map_or_else(
                || {
                    let children = node
                        .get(document::CHILDREN)
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let separator = children
                        .iter()
                        .find(|child| child.get(document::CHILDREN).is_some())
                        .map_or("", |_| "\n");
                    children
                        .iter()
                        .map(slate_text)
                        .collect::<Vec<_>>()
                        .join(separator)
                },
                str::to_owned,
            ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            String::new()
        }
    }
}

fn end_object(fields: &Value, end: &str) -> Option<String> {
    fields
        .get(end)
        .and_then(|end| end.get(document::OBJECT_ID))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// A loaded board: the user's right and its whole document.
#[derive(Debug, Clone)]
pub struct LoadedBoard {
    pub right: EditRight,
    pub document: Doc,
}

pub(crate) fn json(any: &Any) -> Option<Value> {
    serde_json::to_value(any).ok()
}

pub(crate) fn any(value: &Value) -> Result<Any> {
    Ok(serde_json::from_value(value.clone())?)
}

fn number(any: &Any) -> Option<u64> {
    json(any).and_then(|value| value.as_u64())
}

fn text_of<T: ReadTxn>(txn: &T, documents: &yrs::MapRef, document: &str) -> String {
    match documents.get(txn, document) {
        Some(Out::YXmlText(root)) => plain(txn, &root),
        _ => String::new(),
    }
}

/// Whether an element sits inside a line of text: a link or a mention.
fn inline<T: ReadTxn>(txn: &T, element: &XmlTextRef) -> bool {
    matches!(
        element.get_attribute(txn, "type"),
        Some(Out::Any(Any::String(kind))) if matches!(&*kind, "link" | "mention")
    )
}

fn plain<T: ReadTxn>(txn: &T, element: &XmlTextRef) -> String {
    let parts: Vec<(bool, String)> = element
        .diff(txn, YChange::identity)
        .into_iter()
        .map(|chunk| match chunk.insert {
            Out::Any(Any::String(run)) => (false, run.to_string()),
            Out::YXmlText(child) => (!inline(txn, &child), plain(txn, &child)),
            other @ (Out::Any(_)
            | Out::YText(_)
            | Out::YArray(_)
            | Out::YMap(_)
            | Out::YXmlElement(_)
            | Out::YXmlFragment(_)
            | Out::YDoc(_)
            | Out::UndefinedRef(_)) => (false, other.to_string(txn)),
        })
        .collect();
    let separator = parts.iter().find(|(block, _)| *block).map_or("", |_| "\n");
    parts
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(separator)
}

/// The user's right on a board, as the server grants it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditRight {
    ReadOnly,
    Editable,
}

impl EditRight {
    /// `read-only` or `editable`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            EditRight::ReadOnly => "read-only",
            EditRight::Editable => "editable",
        }
    }
}
