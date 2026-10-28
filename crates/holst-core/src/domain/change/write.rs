//! A checked change written into the document: new fields, moves, removals.

use std::collections::{BTreeSet, HashMap};

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{
    Any,
    Map as _,
    MapPrelim,
    MapRef,
    Out,
    ReadTxn,
    Text as _,
    TransactionMut,
    Xml as _,
    XmlTextPrelim,
    XmlTextRef,
};

use crate::{
    domain::{
        area::scene::{Scene, text_scale},
        board::{ObjectKind, any, object_with_text},
        change::{
            build::author_number,
            check::{
                CheckedChange,
                Grouping,
                Placement,
                earlier_entry,
                ends_in,
                kind_of,
                parent_frame,
                parent_of,
                rectangle_of,
                removal,
                shows_content,
                sized,
                stuck_to,
                subtree,
            },
            journal,
            rich::fill_blocks,
            value::{
                ArrowLook,
                ChangeSources,
                ChangedObject,
                ImageUpload,
                LayerOrder,
                NewObject,
                ObjectChangeKind,
                ObjectPatch,
                Upload,
            },
            write_group::{dissolve_after_removal, group, lock_only, ungroup, unlock},
            write_many::{edit_all, paste},
            write_shape::{arrow_changes, shape_changes},
        },
        geometry::{Point, Rectangle, Size},
        id_source::IdSource,
        palette::Color,
        response::LinkPreview,
        rich_text::{Editor, TextFormat, parse},
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{
        ARROW_TYPE,
        COLOR,
        CONTROL,
        CREATED,
        DISPLAY_FILE_NAME,
        DOCUMENT_ID,
        END,
        FILE_NAME,
        FILL_COLOR,
        HEIGHT,
        ID,
        LABEL_TEXT,
        LINK,
        LINK_INFO,
        NAME,
        OBJECT_ID,
        OPACITY,
        ORIGINAL_NAME,
        PARENT_ID,
        POINT,
        POSITION,
        RELATIVE_POINT,
        SHAPE_TYPE,
        SHOW_AUTHOR,
        START,
        STROKE_COLOR,
        STROKE_WIDTH,
        TAG_IDS,
        TEXT_SCALE,
        TYPE,
        WIDTH,
        X,
        Y,
        Z_INDEX,
    },
};

const LAYER_STEP: f64 = 1000.0;
/// The short side of a sticker before `textScale`; the long one is twice it.
const STICKER_SIDE: f64 = 192.0;

pub(super) struct DocumentRoots {
    pub(super) objects: MapRef,
    pub(super) documents: MapRef,
    pub(super) users: MapRef,
    pub(super) journal: MapRef,
    pub(super) tags: MapRef,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AuthorStamp {
    pub(crate) author: u32,
    pub(crate) at: Timestamp,
}

impl AuthorStamp {
    pub(super) fn json(self) -> Value {
        json!({ "a": self.author, "t": self.at.as_millis() })
    }
}

pub(super) fn jitter(id: &str) -> f64 {
    f64::from(author_number(id, 7)) / f64::from(0x8000_0000_u32)
}

pub(super) struct WriteTransaction<'a> {
    pub(super) roots: &'a DocumentRoots,
    pub(super) states: &'a HashMap<String, Value>,
    pub(super) stamp: AuthorStamp,
    pub(super) context: &'a ChangeSources<'a>,
}

/// What an edit of many sets beside the patch: the layer and the tags' ids.
#[derive(Debug, Default)]
pub(super) struct Alongside {
    pub(super) layer: Option<f64>,
    pub(super) tag_ids: Option<Vec<String>>,
}

#[derive(Clone, Copy)]
pub(super) struct EditedObject<'a> {
    pub(super) id: &'a str,
    pub(super) kind: &'a ObjectKind,
    pub(super) patch: &'a ObjectPatch,
    pub(super) also: &'a Alongside,
}

pub(super) fn write_change(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    checked: CheckedChange<'_>,
) -> Result<Vec<ChangedObject>> {
    match checked {
        CheckedChange::AddAll(entries) => add_all(transaction, scope, &entries),
        CheckedChange::EditAll(edits) => edit_all(transaction, scope, &edits),
        CheckedChange::RemoveAll(ids) => remove_all(transaction, scope, &ids),
        CheckedChange::Paste(pasted) => paste(transaction, scope, pasted),
        CheckedChange::Group(members) => {
            group(transaction, scope, &members, Grouping::Plain)
        }
        CheckedChange::Lock(members) => {
            group(transaction, scope, &members, Grouping::Locked)
        }
        CheckedChange::Ungroup(groups) => ungroup(transaction, scope, &groups),
        CheckedChange::LockOnly(ids) => lock_only(transaction, scope, &ids),
        CheckedChange::Unlock(ids) => unlock(transaction, scope, &ids),
    }
}

/// An object just written: what it touches and the fields it holds.
struct Made {
    changed: ChangedObject,
    fields: Value,
}

/// New objects one after another; an arrow's `#N` end is the id of the N-th.
/// Each is written against the board with the ones before it, so it is above
/// them.
fn add_all(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    entries: &[(NewObject, Placement)],
) -> Result<Vec<ChangedObject>> {
    let mut states = scope.states.clone();
    let mut touched: Vec<ChangedObject> = Vec::new();
    // imperative: each object is written against the ones before it
    for (new, placement) in entries {
        let new = with_ids(new, &touched);
        let placement = Placement {
            parent: placement
                .parent
                .as_deref()
                .map(|parent| id_of(parent, &touched)),
            format: placement.format,
        };
        let made = add(
            transaction,
            &WriteTransaction {
                roots: scope.roots,
                states: &states,
                stamp: scope.stamp,
                context: scope.context,
            },
            &new,
            placement,
        )?;
        states.insert(made.changed.id.clone(), made.fields);
        touched.push(made.changed);
    }
    Ok(touched)
}

/// The id of the entry `#N` names among the `made`; anything else as it is.
fn id_of(reference: &str, made: &[ChangedObject]) -> String {
    earlier_entry(reference)
        .and_then(|slot| made.get(slot))
        .map_or_else(|| reference.to_owned(), |entry| entry.id.clone())
}

/// `new` with an arrow's `#N` ends turned into the ids of the entries made.
fn with_ids(new: &NewObject, made: &[ChangedObject]) -> NewObject {
    match new {
        NewObject::Arrow {
            from,
            to,
            color,
            look,
        } => NewObject::Arrow {
            from: id_of(from, made),
            to: id_of(to, made),
            color: color.clone(),
            look: look.clone(),
        },
        NewObject::Sticker { .. }
        | NewObject::Text { .. }
        | NewObject::Shape { .. }
        | NewObject::Frame { .. }
        | NewObject::Link { .. }
        | NewObject::Image { .. }
        | NewObject::File { .. } => new.clone(),
    }
}

fn add(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    new: &NewObject,
    placement: Placement,
) -> Result<Made> {
    let WriteTransaction {
        roots,
        states,
        stamp,
        context,
    } = *scope;
    let id = context.ids.new_id();
    let (kind, fields, text) = stored_fields(new, states);
    let parent = placement
        .parent
        .map_or(json!({}), |parent| json!({ (PARENT_ID): parent }));
    let document = text
        .map(|text| {
            let document = context.ids.new_id();
            let root = roots.documents.insert(
                transaction,
                document.as_str(),
                XmlTextPrelim::new(""),
            );
            fill_as(
                transaction,
                &root,
                text,
                (placement.format, &kind),
                context.ids,
            );
            document
        })
        .map_or(json!({}), |document| json!({ (DOCUMENT_ID): document }));
    let common = json!({
        (ID): id, (TYPE): kind.as_str(), (Z_INDEX): layer(states, new, &id),
        (CREATED): stamp.json(), "updated": null,
    });
    let pairs: Vec<(String, Value)> = [common, fields, document, parent]
        .into_iter()
        .filter_map(|part| part.as_object().cloned())
        .flatten()
        .collect();
    let all = pairs
        .iter()
        .map(|(name, value)| any(value).map(|value| (name.clone(), value)))
        .collect::<Result<Vec<(String, Any)>>>()?;
    roots
        .objects
        .insert(transaction, id.as_str(), MapPrelim::from_iter(all));
    Ok(Made {
        changed: ChangedObject {
            id,
            kind,
            change: ObjectChangeKind::Created,
            fields: Vec::new(),
            link: None,
        },
        fields: Value::Object(pairs.into_iter().collect()),
    })
}

/// The layer of `id` past every other object's, in front or at the back.
pub(super) fn beyond(
    states: &HashMap<String, Value>,
    id: &str,
    stacking: LayerOrder,
) -> f64 {
    let layers = states
        .iter()
        .filter(|(other, _)| other.as_str() != id)
        .filter_map(|(_, fields)| fields.get(Z_INDEX).and_then(Value::as_f64));
    let lift = LAYER_STEP + LAYER_STEP * jitter(id);
    match stacking {
        LayerOrder::Back => layers.reduce(f64::min).unwrap_or(0.0) - lift,
        LayerOrder::Front => layers.reduce(f64::max).unwrap_or(0.0) + lift,
    }
}

fn layer(states: &HashMap<String, Value>, new: &NewObject, id: &str) -> f64 {
    let stacking = match new {
        NewObject::Frame { .. } => LayerOrder::Back,
        NewObject::Sticker { .. }
        | NewObject::Text { .. }
        | NewObject::Shape { .. }
        | NewObject::Arrow { .. }
        | NewObject::Link { .. }
        | NewObject::Image { .. }
        | NewObject::File { .. } => LayerOrder::Front,
    };
    beyond(states, id, stacking)
}

fn stored_fields<'a>(
    new: &'a NewObject,
    states: &HashMap<String, Value>,
) -> (ObjectKind, Value, Option<&'a str>) {
    match new {
        NewObject::Sticker {
            text,
            at,
            size,
            color,
        } => {
            let (width, height) = sized(size.as_ref(), &ObjectKind::Sticker);
            (
                ObjectKind::Sticker,
                json!({
                    (POSITION): at, (WIDTH): width, (HEIGHT): height,
                    (FILL_COLOR): fill(color.as_ref(), "yellow4"), (TEXT_SCALE): 1,
                    (SHOW_AUTHOR): false,
                }),
                Some(text.as_str()),
            )
        }
        NewObject::Text { text, at } => (
            ObjectKind::Text,
            json!({ (POSITION): at, "fontFamily": "Inter", "lineHeight": "150%" }),
            Some(text.as_str()),
        ),
        NewObject::Shape {
            text,
            at,
            size,
            color,
            kind,
        } => {
            let (width, height) = sized(size.as_ref(), &ObjectKind::Shape);
            (
                ObjectKind::Shape,
                json!({
                    (POSITION): at, (WIDTH): width, (HEIGHT): height,
                    (SHAPE_TYPE): kind.stored_name(), "fixedSize": true,
                    (FILL_COLOR): fill(color.as_ref(), "white6"),
                    (STROKE_COLOR): { (COLOR): "gray12", (OPACITY): 1 }, (STROKE_WIDTH): 2,
                }),
                Some(text.as_str()),
            )
        }
        NewObject::Frame {
            title: label,
            at,
            size,
            color,
        } => frame_fields(label, at, size.as_ref(), color.as_ref()),
        NewObject::Arrow {
            from,
            to,
            color,
            look,
        } => arrow_fields(states, (from, to), color.as_ref(), look),
        NewObject::Link {
            url,
            at,
            preview,
            embed,
        } => link_fields(url, at, preview.as_ref(), *embed),
        NewObject::Image { upload, at, size } => {
            image_fields(upload, at, size.as_ref())
        }
        NewObject::File { upload, at } => file_fields(upload, at),
    }
}

/// A link card as the web client pastes one; embedded when the preview
/// offers an embed and `embed` is asked for.
fn link_fields<'a>(
    url: &str,
    at: &Point,
    preview: Option<&LinkPreview>,
    embed: bool,
) -> (ObjectKind, Value, Option<&'a str>) {
    let none = LinkPreview::default();
    let preview = preview.unwrap_or(&none);
    let embedded = preview.embed.as_ref().filter(|_| embed);
    let info = json!({
        "description": preview.description, "faviconKey": preview.favicon_key,
        "imageKey": preview.image_key, (LINK): url, "title": preview.title,
    });
    let embed_info =
        embedded.map_or(json!({}), |offered| json!({ "embedInfo": offered }));
    let link_info: serde_json::Map<String, Value> = [info, embed_info]
        .into_iter()
        .filter_map(|part| part.as_object().cloned())
        .flatten()
        .collect();
    (
        ObjectKind::Link,
        json!({
            (POSITION): at, (LINK): url,
            "displayType": if embedded.is_some() { 2 } else { 1 },
            (LINK_INFO): link_info,
        }),
        None,
    )
}

/// An uploaded picture, shown at `size` or at its own pixels.
fn image_fields<'a>(
    upload: &ImageUpload,
    at: &Point,
    size: Option<&Size>,
) -> (ObjectKind, Value, Option<&'a str>) {
    let pixels = upload.pixels;
    let shown = size.copied().unwrap_or(pixels);
    (
        ObjectKind::Image,
        json!({
            (POSITION): at, (NAME): upload.file.name,
            (ORIGINAL_NAME): upload.file.original, "imageType": upload.file.mime,
            (WIDTH): shown.width(), (HEIGHT): shown.height(),
            "naturalWidth": pixels.width(), "naturalHeight": pixels.height(),
            "isLoading": false,
        }),
        None,
    )
}

/// An uploaded file shown as a stamp: the previews of pages, audio and video
/// come from a step the client takes after the upload.
fn file_fields<'a>(
    upload: &Upload,
    at: &Point,
) -> (ObjectKind, Value, Option<&'a str>) {
    (
        ObjectKind::File,
        json!({
            (POSITION): at, (FILE_NAME): upload.name,
            (DISPLAY_FILE_NAME): upload.original, "fileType": upload.mime,
            "fileSize": upload.bytes, "displayType": "stamp",
        }),
        None,
    )
}

fn fill(color: Option<&Color>, default: &str) -> Value {
    json!({ (COLOR): color.map_or(default, Color::as_str), (OPACITY): 1 })
}

fn frame_fields<'a>(
    label: &'a str,
    at: &Point,
    size: Option<&Size>,
    color: Option<&Color>,
) -> (ObjectKind, Value, Option<&'a str>) {
    let (width, height) = sized(size, &ObjectKind::Frame);
    (
        ObjectKind::Frame,
        json!({
            (POSITION): at, (WIDTH): width, (HEIGHT): height,
            (FILL_COLOR): fill(color, "white3"), (LABEL_TEXT): label,
        }),
        None,
    )
}

/// An arrow as the web client draws one by dragging from a side handle of
/// the first object into the second: bound at the right side's middle, and at
/// a point inside the second, which the client routes to its nearest side.
fn arrow_fields<'a>(
    states: &HashMap<String, Value>,
    (from, to): (&str, &str),
    color: Option<&Color>,
    look: &ArrowLook,
) -> (ObjectKind, Value, Option<&'a str>) {
    let rectangle = |object: &str| states.get(object).and_then(rectangle_of);
    let point = |x: f64, y: f64| json!({ (X): x, (Y): y });
    let start = json!({
        (OBJECT_ID): from, (CONTROL): 2, (RELATIVE_POINT): point(1.0, 0.5),
        (POINT): rectangle(from).map_or(
            point(0.0, 0.0),
            |rect| point(rect.right(), rect.y + rect.height / 2.0),
        ),
    });
    let end = json!({
        (OBJECT_ID): to, (CONTROL): null,
        (POINT): rectangle(to).map_or(point(0.0, 0.0), |rect| {
            let middle = rect.centre();
            point(middle.x, middle.y)
        }),
    });
    let mut fields = json!({
        (POSITION): point(0.0, 0.0), (START): start, (END): end,
        (ARROW_TYPE): "elbow",
        (STROKE_COLOR): { (COLOR): color.map_or("gray12", Color::as_str) },
    });
    let looks = arrow_changes(&look.arrow).into_iter().chain(shape_changes(
        &look.outline,
        &json!({}),
        None,
        false,
    ));
    if let Some(map) = fields.as_object_mut() {
        map.extend(looks.map(|(name, value)| (name.to_owned(), value)));
    }
    (ObjectKind::Arrow, fields, None)
}

/// `text` as lines, or as the blocks of the markdown it holds for `kind`.
fn fill_as(
    transaction: &mut TransactionMut<'_>,
    root: &XmlTextRef,
    text: &str,
    (format, kind): (TextFormat, &ObjectKind),
    ids: &dyn IdSource,
) {
    match format {
        TextFormat::Plain => fill_text(transaction, root, text, ids),
        TextFormat::Markdown => fill_blocks(
            transaction,
            root,
            &parse(text, &Editor::of(kind)).blocks,
            ids,
        ),
    }
}

pub(super) fn fill_text(
    transaction: &mut TransactionMut<'_>,
    root: &XmlTextRef,
    text: &str,
    ids: &dyn IdSource,
) {
    text.split('\n').zip(0_u32..).for_each(|(line, at)| {
        let wrapper = root.insert_embed(transaction, at, XmlTextPrelim::new(""));
        wrapper.insert_attribute(transaction, "type", "wrapper");
        wrapper.insert_attribute(transaction, "key", ids.new_id());
        let paragraph = wrapper.insert_embed(transaction, 0, XmlTextPrelim::new(line));
        paragraph.insert_attribute(transaction, "type", "paragraph");
        paragraph.insert_attribute(transaction, "key", ids.new_id());
    });
}

pub(super) fn edit(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    edited: &EditedObject<'_>,
) -> Result<Vec<ChangedObject>> {
    let EditedObject {
        id,
        kind,
        patch,
        also,
    } = *edited;
    let states = scope.states;
    let fields = states.get(id).cloned().unwrap_or_default();
    let map = object_map(transaction, &scope.roots.objects, id, kind)?;
    let content = content_changes(transaction, scope, edited, &fields);
    // A moved object with no parent or a frame for one goes into the deepest frame.
    let held = subtree(states, id);
    let placed = placed_rectangle(patch, &fields);
    let framed = parent_of(&fields)
        .is_none_or(|parent| kind_of(states, parent) == ObjectKind::Frame);
    let parent = placed
        .filter(|_| framed)
        .map(|rect| parent_frame(states, &rect, &held, None));
    let reparented = parent
        .clone()
        .filter(|candidate| candidate.as_deref() != parent_of(&fields));
    let changed: Vec<(&'static str, Value)> = content
        .into_iter()
        .chain(reparented.as_ref().map(|name| (PARENT_ID, json!(name))))
        .chain(also.layer.map(|layer| (Z_INDEX, json!(layer))))
        .chain(also.tag_ids.as_ref().map(|ids| (TAG_IDS, json!(ids))))
        .chain([("updated", scope.stamp.json())])
        .collect();
    insert_changes(transaction, &map, &changed)?;
    let moved = match (patch.at, placed) {
        (Some(to), Some(rect)) => Some(MovedFrame {
            frame: id,
            rectangle: rect,
            parent: parent.flatten(),
            to,
            adopts: *kind == ObjectKind::Frame && shows_content(states, id),
        }),
        _ => None,
    };
    let carried = moved
        .map(|moved| carried_objects(transaction, scope, &moved))
        .transpose()?
        .unwrap_or_default();
    Ok([ChangedObject {
        id: id.to_owned(),
        kind: kind.clone(),
        change: ObjectChangeKind::Changed,
        fields: changed_names(patch, also, reparented.as_ref().map(|_| "parent")),
        link: None,
    }]
    .into_iter()
    .chain(carried)
    .collect())
}

fn content_changes(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    edited: &EditedObject<'_>,
    fields: &Value,
) -> Vec<(&'static str, Value)> {
    let EditedObject { kind, patch, .. } = *edited;
    let text = patch
        .text
        .as_ref()
        .map(|text| {
            if *kind == ObjectKind::Frame {
                vec![(LABEL_TEXT, json!(text))]
            } else {
                write_text(transaction, scope, fields, (text, patch.format, kind))
            }
        })
        .unwrap_or_default();
    text.into_iter()
        .chain(shape_changes(&patch.shape, fields, patch.size, patch.color.is_some()))
        .chain(arrow_changes(&patch.arrow))
        .chain(patch.at.map(|at| (POSITION, json!(at))))
        .chain(
            patch.size.into_iter().flat_map(|size| {
                size_changes(&*transaction, scope, kind, fields, size)
            }),
        )
        .chain(patch.color.as_ref().map(|color| {
            (
                color_field(kind),
                json!({ (COLOR): color.as_str(), (OPACITY): patch.shape.opacity.map_or_else(|| json!(1), |opacity| json!(opacity)) }),
            )
        }))
        .collect()
}

/// A sticker, a stack and a text grow by `textScale` (the web client's corner
/// handle: new height over the height at scale 1); a sticker's width snaps to
/// the nearer of its two sides. Other kinds take free sides.
fn size_changes(
    transaction: &TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    kind: &ObjectKind,
    fields: &Value,
    size: Size,
) -> Vec<(&'static str, Value)> {
    match kind {
        ObjectKind::Sticker | ObjectKind::StickerStack | ObjectKind::Text => {
            scaled_size(transaction, scope, kind, fields, size)
        }
        ObjectKind::Shape
        | ObjectKind::Document
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
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => {
            vec![(WIDTH, json!(size.width())), (HEIGHT, json!(size.height()))]
        }
    }
}

fn scaled_size(
    transaction: &TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    kind: &ObjectKind,
    fields: &Value,
    size: Size,
) -> Vec<(&'static str, Value)> {
    let object = object_with_text(fields.clone(), transaction, &scope.roots.documents);
    let scale = object.as_ref().map_or(1.0, text_scale);
    let base = object
        .as_ref()
        .and_then(|object| {
            Scene::new(std::slice::from_ref(object)).rectangle_of(object)
        })
        .map(|rectangle| rectangle.height / scale)
        .filter(|height| *height > 0.0)
        .unwrap_or(STICKER_SIDE);
    let grown = size.height() / base;
    let seen = size.width() / grown;
    let snapped = if (STICKER_SIDE - seen).abs() > (2.0 * STICKER_SIDE - seen).abs() {
        2.0 * STICKER_SIDE
    } else {
        STICKER_SIDE
    };
    let width = (*kind == ObjectKind::Sticker).then(|| (WIDTH, json!(snapped)));
    [(TEXT_SCALE, json!(grown))]
        .into_iter()
        .chain(width)
        .collect()
}

fn color_field(kind: &ObjectKind) -> &'static str {
    if *kind == ObjectKind::Arrow {
        "strokeColor"
    } else {
        "fillColor"
    }
}

fn placed_rectangle(patch: &ObjectPatch, fields: &Value) -> Option<Rectangle> {
    patch.at.map(|at| {
        let (width, height) = patch.size.map_or_else(
            || rectangle_of(fields).map_or((0.0, 0.0), |r| (r.width, r.height)),
            |size| (size.width(), size.height()),
        );
        Rectangle::new(at.x(), at.y(), width, height)
    })
}

fn changed_names(
    patch: &ObjectPatch,
    also: &Alongside,
    reparented: Option<&'static str>,
) -> Vec<&'static str> {
    [
        patch.text.as_ref().map(|_| "text"),
        patch.at.map(|_| "position"),
        patch.size.map(|_| "size"),
        patch.color.as_ref().map(|_| "color"),
        also.layer.map(|_| "order"),
        also.tag_ids.as_ref().map(|_| "tags"),
        reparented,
    ]
    .into_iter()
    .flatten()
    .chain(patch.shape.fields())
    .chain(patch.arrow.fields())
    .collect()
}

pub(super) fn insert_changes(
    transaction: &mut TransactionMut<'_>,
    map: &MapRef,
    changes: &[(&'static str, Value)],
) -> Result<()> {
    changes.iter().try_for_each(|(name, value)| {
        any(value).map(|value| {
            map.insert(transaction, *name, value);
        })
    })
}

struct MovedFrame<'a> {
    frame: &'a str,
    rectangle: Rectangle,
    parent: Option<String>,
    to: Point,
    /// Only a frame takes in the objects it now covers.
    adopts: bool,
}

fn carried_objects(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    moved: &MovedFrame<'_>,
) -> Result<Vec<ChangedObject>> {
    let moving = carry(transaction, scope, moved)?;
    let adopted = if moved.adopts {
        adopt(transaction, scope, moved)?
    } else {
        Vec::new()
    };
    Ok(moving.into_iter().chain(adopted).collect())
}

fn adopt(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    around: &MovedFrame<'_>,
) -> Result<Vec<ChangedObject>> {
    let WriteTransaction {
        roots,
        states,
        stamp,
        ..
    } = *scope;
    let held = subtree(states, around.frame);
    let beside = |fields: &Value| parent_of(fields) == around.parent.as_deref();
    states
        .iter()
        .filter(|(id, fields)| {
            !held.contains(*id)
                && kind_of(states, id) != ObjectKind::Arrow
                && beside(fields)
                && rectangle_of(fields)
                    .is_some_and(|rect| around.rectangle.contains(&rect))
        })
        .map(|(id, _)| id.clone())
        .sorted_unstable()
        .map(|id| {
            let kind = kind_of(states, &id);
            let map = object_map(transaction, &roots.objects, &id, &kind)?;
            map.insert(transaction, PARENT_ID, around.frame);
            map.insert(transaction, "updated", any(&stamp.json())?);
            Ok(ChangedObject {
                id,
                kind,
                change: ObjectChangeKind::Changed,
                fields: vec!["parent"],
                link: None,
            })
        })
        .collect()
}

pub(crate) fn object_map<T: ReadTxn>(
    transaction: &T,
    objects: &MapRef,
    id: &str,
    kind: &ObjectKind,
) -> Result<MapRef> {
    match objects.get(transaction, id) {
        Some(Out::YMap(map)) => Ok(map),
        _ => Err(Error::BoardChange(BoardChangeError::NotChangeable {
            kind: kind.as_str().to_owned(),
            field: "stored fields",
        })),
    }
}

fn write_text(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    fields: &Value,
    (text, format, kind): (&str, TextFormat, &ObjectKind),
) -> Vec<(&'static str, Value)> {
    let documents = &scope.roots.documents;
    let context = scope.context;
    let current =
        fields
            .get(DOCUMENT_ID)
            .and_then(Value::as_str)
            .and_then(|document| match documents.get(transaction, document) {
                Some(Out::YXmlText(root)) => Some(root),
                _ => None,
            });
    let cleared = current.inspect(|root| {
        let len = root.len(transaction);
        root.remove_range(transaction, 0, len);
    });
    let document = cleared.is_none().then(|| context.ids.new_id());
    let root = cleared.or_else(|| {
        document.as_deref().map(|document| {
            documents.insert(transaction, document, XmlTextPrelim::new(""))
        })
    });
    let filled =
        root.map(|root| fill_as(transaction, &root, text, (format, kind), context.ids));
    filled
        .and(document)
        .map(|document| vec![(DOCUMENT_ID, json!(document))])
        .unwrap_or_default()
}

fn carry(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    moved: &MovedFrame<'_>,
) -> Result<Vec<ChangedObject>> {
    let WriteTransaction {
        roots,
        states,
        stamp,
        ..
    } = *scope;
    let (frame, to) = (moved.frame, moved.to);
    let from = states.get(frame).and_then(|fields| fields.get(POSITION));
    let delta = |axis: &str, target: f64| {
        target
            - from
                .and_then(|position| position.get(axis))
                .and_then(Value::as_f64)
                .unwrap_or(target)
    };
    let shift = (delta(X, to.x()), delta(Y, to.y()));
    let moving = moving_with(states, frame);
    moving
        .iter()
        .filter(|id| *id != frame)
        .filter_map(|id| {
            let stored = states.get(id)?;
            let kind = kind_of(states, id);
            let changes = if kind == ObjectKind::Arrow {
                arrow_moved(stored, &moving, shift)
            } else {
                vec![(POSITION, shifted(stored.get(POSITION)?, shift))]
            };
            Some((id.clone(), kind, changes))
        })
        .map(|(id, kind, changes)| {
            let map = object_map(transaction, &roots.objects, &id, &kind)?;
            insert_changes(transaction, &map, &changes)?;
            map.insert(transaction, "updated", any(&stamp.json())?);
            Ok(ChangedObject {
                id,
                fields: vec![moved_field(&kind)],
                kind,
                change: ObjectChangeKind::Changed,
                link: None,
            })
        })
        .collect()
}

fn moved_field(kind: &ObjectKind) -> &'static str {
    if *kind == ObjectKind::Arrow {
        "ends"
    } else {
        "position"
    }
}

fn moving_with(states: &HashMap<String, Value>, frame: &str) -> BTreeSet<String> {
    let held = subtree(states, frame);
    let arrows: Vec<String> = states
        .iter()
        .filter(|(id, fields)| {
            kind_of(states, id) == ObjectKind::Arrow
                && ends_in(fields, START, &held)
                && ends_in(fields, END, &held)
        })
        .map(|(id, _)| id.clone())
        .collect();
    let stamps: Vec<String> = states
        .iter()
        .filter(|(_, fields)| stuck_to(fields, &held))
        .map(|(id, _)| id.clone())
        .collect();
    held.iter().cloned().chain(arrows).chain(stamps).collect()
}

pub(super) fn shifted(point: &Value, (dx, dy): (f64, f64)) -> Value {
    let axis =
        |name: &str, by: f64| point.get(name).and_then(Value::as_f64).map(|v| v + by);
    let updates = [(X, axis(X, dx)), (Y, axis(Y, dy))]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name.to_owned(), json!(value))));
    point.as_object().map_or_else(
        || point.clone(),
        |fields| Value::Object(fields.clone().into_iter().chain(updates).collect()),
    )
}

pub(super) fn arrow_moved(
    fields: &Value,
    moving: &BTreeSet<String>,
    shift: (f64, f64),
) -> Vec<(&'static str, Value)> {
    let end = |side: &'static str| {
        let end = fields.get(side)?;
        let rides = end
            .get(OBJECT_ID)
            .and_then(Value::as_str)
            .is_none_or(|object| moving.contains(object));
        let point = end.get(POINT).filter(|_| rides)?;
        let moved = end.as_object()?;
        let shifted_point = (POINT.to_owned(), shifted(point, shift));
        Some((
            side,
            Value::Object(moved.clone().into_iter().chain([shifted_point]).collect()),
        ))
    };
    let controls = |name: &'static str| {
        let list = fields
            .get(name)?
            .as_array()
            .filter(|list| !list.is_empty())?;
        Some((
            name,
            Value::Array(list.iter().map(|c| shifted(c, shift)).collect()),
        ))
    };
    [
        end(START),
        end(END),
        controls("elbowMidwayControls"),
        controls("midwayControls"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn remove_all(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    ids: &[String],
) -> Result<Vec<ChangedObject>> {
    let WriteTransaction {
        roots,
        states,
        stamp,
        ..
    } = *scope;
    let gone = removal(states, ids);
    let touched: Vec<ChangedObject> = gone
        .iter()
        .map(|id| {
            roots.objects.remove(transaction, id);
            ChangedObject {
                kind: kind_of(states, id),
                id: id.clone(),
                change: ObjectChangeKind::Removed,
                fields: Vec::new(),
                link: None,
            }
        })
        .collect();
    let removed: Vec<&str> = gone.iter().map(String::as_str).collect();
    journal::write(
        transaction,
        &roots.journal,
        journal::entry(states, &removed, stamp),
    )?;
    let dissolved =
        dissolve_after_removal(transaction, scope, &gone.iter().cloned().collect())?;
    Ok(touched.into_iter().chain(dissolved).collect())
}
