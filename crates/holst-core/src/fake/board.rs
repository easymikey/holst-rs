//! A filled board and the Y.Doc the board socket serves for it.

mod edits;

use serde_json::{Value, json};
use yrs::{
    Any,
    Doc,
    Map as _,
    MapPrelim,
    ReadTxn as _,
    StateVector,
    Text as _,
    Transact as _,
    TransactionMut,
    Update,
    Xml as _,
    XmlTextPrelim,
    XmlTextRef,
    types::ToJson as _,
    updates::decoder::Decode as _,
};

/// What a filled board holds, numbered from 1; the last three wrap one.
#[derive(Debug, Clone)]
pub enum FixtureObject {
    Sticker(String),
    FreeText(String),
    Shape(String),
    Frame(String),
    /// A group; its members name it with `inside`.
    Group,
    /// A kind the clients know and holst-rs has no variant for.
    MindMapNode(String),
    /// An image whose file the storage serves, by its stored name.
    Image(String),
    /// An image whose file the public storage refuses (private or gone).
    RefusedImage(String),
    /// An arrow from the object with the first number to the second.
    Arrow(u32, u32),
    /// A link card: the address and the title of the page.
    Link(String, String),
    /// A file the storage serves, by its stored name, and its size in bytes.
    File(String, u64),
    /// The object with one field of its document replaced.
    Set(Box<FixtureObject>, String, Value),
    /// The object inside the frame with this number.
    Inside(Box<FixtureObject>, u32),
}

/// A board as the socket serves it: the document it changes and who may.
#[derive(Debug, Clone)]
pub(crate) struct Board {
    pub(crate) id: String,
    pub(crate) document: Doc,
    pub(crate) read_only: bool,
}

const CREATED: i64 = 1_700_000_000_000;

/// Returns a document holding everything `source` holds now.
pub(crate) fn copied_document(source: &Doc) -> Doc {
    let document = Doc::new();
    let state = source
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    if let Ok(update) = Update::decode_v2(&state) {
        // an update the source itself produced always applies
        drop(document.transact_mut().apply_update(update));
    }
    document
}

/// Returns the document as a client builds it: objects and their texts.
pub(crate) fn fixture_document(board: &str, objects: &[FixtureObject]) -> Doc {
    let document = Doc::new();
    add_objects(&document, board, 0, objects);
    document
}

/// The document of the board `board` holding `objects`, as a snapshot keeps it:
/// one Yjs v2 update.
pub(crate) fn snapshot_bytes(board: &str, objects: &[FixtureObject]) -> Vec<u8> {
    fixture_document(board, objects)
        .transact()
        .encode_state_as_update_v2(&StateVector::default())
}

/// Writes `objects` into the document of the board, numbered after the `held`
/// objects it holds.
pub(crate) fn add_objects(
    document: &Doc,
    board: &str,
    held: u32,
    objects: &[FixtureObject],
) {
    let roots = document.get_or_insert_map("objects");
    let documents = document.get_or_insert_map("documents");
    let mut txn = document.transact_mut();
    objects.iter().zip(held + 1..).for_each(|(object, number)| {
        let ordinal = ordinal(board, number);
        let id = object_id(board, number);
        let text_id = format!("30000000-0000-4000-8000-{ordinal:012}");
        let fields = object.fields(board, number, &text_id);
        roots.insert(
            &mut txn,
            id.as_str(),
            MapPrelim::from_iter(any_fields(
                fields.as_object().cloned().unwrap_or_default(),
            )),
        );
        object.text().into_iter().for_each(|text| {
            let root =
                documents.insert(&mut txn, text_id.as_str(), XmlTextPrelim::new(""));
            fill(&mut txn, &root, text, ordinal);
        });
    });
    drop(txn);
}

/// Returns the fields of every object of the document, in the order they
/// came.
pub(crate) fn objects_json(document: &Doc) -> Vec<Value> {
    let objects = document.get_or_insert_map("objects");
    let txn = document.transact();
    let mut all: Vec<Value> = objects
        .iter(&txn)
        .filter_map(|(_, value)| serde_json::to_value(value.to_json(&txn)).ok())
        .collect();
    all.sort_by_key(|object| {
        object.get("id").and_then(Value::as_str).map(str::to_owned)
    });
    all
}

/// Returns the objects as the web client keeps them in a template: the text of
/// a JSON array.
pub(crate) fn template_data(objects: &[FixtureObject]) -> String {
    Value::Array(objects_json(&fixture_document("template", objects))).to_string()
}

/// Returns the id of the `number`th object filled on the board, counting from
/// 1.
pub(crate) fn object_id(board: &str, number: u32) -> String {
    format!("20000000-0000-4000-8000-{:012}", ordinal(board, number))
}

// Unique across boards: the board's own tail, then the object's number.
fn ordinal(board: &str, number: u32) -> u64 {
    let tail = board.rsplit('-').next().unwrap_or_default();
    u64::from_str_radix(tail, 16).unwrap_or_default() * 1000 + u64::from(number)
}

/// The fields of the objects of `parts`, later ones replacing earlier ones.
fn merged(parts: [Value; 2]) -> Value {
    parts
        .into_iter()
        .filter_map(|part| part.as_object().cloned())
        .flatten()
        .collect()
}

/// The fields as yrs values; a field with no such value is left out.
pub(crate) fn any_fields(
    fields: impl IntoIterator<Item = (String, Value)>,
) -> Vec<(String, Any)> {
    fields
        .into_iter()
        .filter_map(|(name, value)| {
            Any::from_json(&value.to_string())
                .ok()
                .map(|any| (name, any))
        })
        .collect()
}

impl FixtureObject {
    /// Puts the object at `x`, `y` instead of in the row of the fixture.
    #[must_use]
    pub fn at(self, x: i32, y: i32) -> Self {
        self.set("position", json!({ "x": x, "y": y }))
    }

    /// Gives the object this width and height.
    #[must_use]
    pub fn sized(self, width: u32, height: u32) -> Self {
        self.set("width", json!(width)).set("height", json!(height))
    }

    /// Stacks the object at `z_index`; a higher one is above.
    #[must_use]
    pub fn stacked(self, z_index: u32) -> Self {
        self.set("zIndex", json!(z_index))
    }

    /// Makes the object a child of the frame or group with this number; only
    /// its `parentId` changes, its place stays what it is.
    #[must_use]
    pub fn inside(self, frame: u32) -> Self {
        Self::Inside(Box::new(self), frame)
    }

    /// Replaces the document field `field` of the object with `value`.
    #[must_use]
    pub fn set(self, field: &str, value: Value) -> Self {
        Self::Set(Box::new(self), field.to_owned(), value)
    }

    /// The stored file the object points at, and whether the storage serves it.
    pub(crate) fn stored(&self) -> Option<(&str, bool)> {
        match self {
            Self::Image(stored) | Self::File(stored, _) => Some((stored, true)),
            Self::RefusedImage(stored) => Some((stored, false)),
            Self::Set(inner, ..) | Self::Inside(inner, _) => inner.stored(),
            Self::Sticker(_)
            | Self::FreeText(_)
            | Self::Shape(_)
            | Self::Frame(_)
            | Self::Group
            | Self::MindMapNode(_)
            | Self::Arrow(..)
            | Self::Link(..) => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Sticker(text)
            | Self::FreeText(text)
            | Self::Shape(text)
            | Self::MindMapNode(text) => Some(text),
            Self::Set(inner, ..) | Self::Inside(inner, _) => inner.text(),
            Self::Frame(_)
            | Self::Group
            | Self::Image(_)
            | Self::RefusedImage(_)
            | Self::Arrow(..)
            | Self::Link(..)
            | Self::File(..) => None,
        }
    }

    fn fields(&self, board: &str, number: u32, document: &str) -> Value {
        let common = json!({
            "id": object_id(board, number),
            "zIndex": 1000 + number,
            "created": { "a": 1, "t": CREATED },
            "updated": null,
            "position": { "x": 100 + 250 * (number - 1), "y": 100 },
        });
        merged([common, self.own(board, document)])
    }

    // What the kind adds to the fields every object has.
    fn own(&self, board: &str, document: &str) -> Value {
        match self {
            Self::Sticker(_) => json!({
                "type": "sticker", "documentId": document, "width": 192, "height": 192,
                "fillColor": { "color": "yellow4", "opacity": 1 }, "textScale": 1,
            }),
            Self::FreeText(_) => json!({
                "type": "simple-text", "documentId": document,
                "fontFamily": "Inter", "lineHeight": "150%",
            }),
            Self::Shape(_) => json!({
                "type": "shape", "documentId": document, "shapeType": "square",
                "width": 200, "height": 120, "fixedSize": true,
                "fillColor": { "color": "white6", "opacity": 1 },
                "strokeColor": { "color": "gray12", "opacity": 1 }, "strokeWidth": 2,
            }),
            Self::MindMapNode(_) => json!({
                "type": "mind-map-node", "documentId": document, "width": 200, "height": 80,
            }),
            Self::Image(stored) | Self::RefusedImage(stored) => json!({
                "type": "image", "name": stored, "originalName": stored,
                "width": 200, "height": 150,
            }),
            Self::Frame(label) => json!({
                "type": "frame", "width": 800, "height": 600, "labelText": label,
                "fillColor": { "color": "white3", "opacity": 1 },
            }),
            Self::Group => json!({ "type": "group", "ignoreZIndex": true }),
            Self::Arrow(from, to) => json!({
                "type": "arrow", "position": { "x": 0, "y": 0 },
                "start": { "objectId": object_id(board, *from), "point": { "x": 0, "y": 0 } },
                "end": { "objectId": object_id(board, *to), "point": { "x": 0, "y": 0 } },
                "arrowType": "elbow",
                "strokeColor": { "color": "gray8", "opacity": 1 }, "strokeWidth": 2,
            }),
            Self::Link(url, title) => json!({
                "type": "link", "link": url, "title": title,
                "description": "", "imageKey": "",
            }),
            Self::File(stored, size) => json!({
                "type": "file", "fileName": stored, "displayFileName": stored,
                "fileSize": size, "fileType": "application/octet-stream",
            }),
            Self::Set(inner, field, value) => merged([
                inner.own(board, document),
                Value::from_iter([(field.clone(), value.clone())]),
            ]),
            Self::Inside(inner, frame) => merged([
                inner.own(board, document),
                json!({ "parentId": object_id(board, *frame) }),
            ]),
        }
    }
}

/// Writes the text as the web client does: a wrapper per line, its paragraph
/// first in it; a line of `- [ ] ` or `- [x] ` is a checklist item.
fn fill(txn: &mut TransactionMut<'_>, root: &XmlTextRef, text: &str, ordinal: u64) {
    let key = |part: u32| {
        format!(
            "40000000-0000-4000-8000-{:012}",
            ordinal * 100 + u64::from(part)
        )
    };
    text.split('\n').zip(0_u32..).for_each(|(line, at)| {
        let wrapper = root.insert_embed(txn, at, XmlTextPrelim::new(""));
        wrapper.insert_attribute(txn, "type", "wrapper");
        wrapper.insert_attribute(txn, "key", key(at * 2));
        let item = [("- [ ] ", false), ("- [x] ", true)]
            .into_iter()
            .find_map(|(marker, done)| Some((line.strip_prefix(marker)?, done)));
        let paragraph = wrapper.insert_embed(
            txn,
            0,
            XmlTextPrelim::new(item.map_or(line, |(rest, _)| rest)),
        );
        paragraph.insert_attribute(
            txn,
            "type",
            item.map_or("paragraph", |_| "checkbox-list-item"),
        );
        paragraph.insert_attribute(txn, "key", key(at * 2 + 1));
        if let Some((_, done)) = item {
            paragraph.insert_attribute(txn, "checked", Any::Bool(done));
        }
    });
}
