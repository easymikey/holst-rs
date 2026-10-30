//! Edits of many objects and copies of objects, written as one change.

use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
};

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{Map as _, TransactionMut, XmlTextPrelim, types::ToJson as _};

use crate::{
    domain::{
        board::{Object, ObjectKind, insert_map, json as stored_json},
        change::{
            check::{
                CheckedEdit,
                corner_of,
                fitted_in,
                frame_above,
                on_board,
                parent_frame,
                rectangle_of,
            },
            stacking,
            value::{ChangedObject, ObjectPatch, Pasted, Reparent},
            write::{
                Alongside,
                Edge,
                EditedObject,
                WriteTransaction,
                arrow_moved,
                beyond,
                edit,
                fill_text,
                new_header,
                shifted,
                stored_now,
            },
        },
        geometry::{Point, Rectangle},
        tree::name_index::build::normalized,
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{
        COLOR,
        DOCUMENT_ID,
        END,
        ID,
        OBJECT_ID,
        PARENT_ID,
        POSITION,
        START,
        TAG_IDS,
        TITLE,
    },
};

// PROTOCOL: the colour a new tag gets.
const NEW_TAG_COLOR: &str = "indigo5";

/// The board's objects as the change has left them so far, and what it touched.
struct Running {
    states: HashMap<String, Value>,
    touched: Vec<ChangedObject>,
}

impl Running {
    fn start(scope: &WriteTransaction<'_>) -> Running {
        Running {
            states: scope.states.clone(),
            touched: Vec::new(),
        }
    }

    fn after(
        mut self,
        read: Vec<(String, Value)>,
        touched: Vec<ChangedObject>,
    ) -> Running {
        self.states.extend(read);
        self.touched.extend(touched);
        self
    }
}

/// `fields` with the field `name` set to `value`.
fn with_field(fields: &Value, name: &str, value: Value) -> Value {
    Value::Object(
        fields
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain([(name.to_owned(), value)])
            .collect(),
    )
}

/// Edits one object after another, each against the board the ones before left.
pub(super) fn edit_all(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    edits: &[CheckedEdit<'_>],
) -> Result<Vec<ChangedObject>> {
    let ends = stacking::ends(scope.states, edits);
    edits
        .iter()
        .try_fold(Running::start(scope), |running, checked| {
            edited(transaction, scope, running, (checked, &ends))
        })
        .map(|done| done.touched)
}

fn edited(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    running: Running,
    (checked, ends): (&CheckedEdit<'_>, &HashMap<String, f64>),
) -> Result<Running> {
    let CheckedEdit {
        id,
        kind,
        edit: asked,
        link_to,
        reparent,
    } = checked;
    let layer = match (asked.stacking, reparent) {
        (Some(order), _) => stacking::layer(&running.states, id, order, ends),
        (None, Some(into)) => {
            Some(stacking::appended(&running.states, id, into.frame()))
        }
        (None, None) => None,
    };
    let patch = match reparent {
        Some(into) => {
            Cow::Owned(kept_in_frame(&running.states, id, into, &asked.patch)?)
        }
        None => Cow::Borrowed(&asked.patch),
    };
    let tag_ids = asked
        .tags
        .as_ref()
        .map(|titles| tag_ids(transaction, scope, titles))
        .transpose()?;
    let also = Alongside {
        layer,
        reparent: reparent.clone(),
        tag_ids,
        link_to: link_to.clone(),
    };
    let now = WriteTransaction {
        roots: scope.roots,
        states: &running.states,
        stamp: scope.stamp,
        context: scope.context,
        user: scope.user,
    };
    let touched = edit(
        transaction,
        &now,
        &EditedObject {
            id,
            kind,
            patch: &patch,
            also: &also,
        },
    )?;
    let read = touched
        .iter()
        .filter_map(|changed| {
            Some((
                changed.id.clone(),
                stored_now(&*transaction, scope, &changed.id)?,
            ))
        })
        .collect();
    Ok(running.after(read, touched))
}

/// The patch with the corner the object keeps relative to its frame: from one
/// frame to another it moves by the difference of their corners, an explicit
/// corner stays as given. BUNDLE: 128.8f630629.js 2123-2137, where a shift
/// of 0.5 or less is not made (2027).
fn kept_in_frame(
    states: &HashMap<String, Value>,
    id: &str,
    into: &Reparent,
    patch: &ObjectPatch,
) -> Result<ObjectPatch> {
    let from = corner_of(states, frame_above(states, id).as_deref());
    let to = corner_of(states, into.frame());
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let shifted = states
        .get(id)
        .and_then(rectangle_of)
        .filter(|_| patch.at.is_none())
        .map(|area| {
            if dx.abs() > 0.5 || dy.abs() > 0.5 {
                Rectangle::new(area.x + dx, area.y + dy, area.width, area.height)
            } else {
                area
            }
        });
    let fitted = shifted
        .zip(into.frame())
        .map(|(moved, frame)| fitted_in(states, id, frame, moved))
        .transpose()?
        .flatten();
    let at = shifted
        .filter(|_| fitted.is_some() || dx.abs() > 0.5 || dy.abs() > 0.5)
        .map(|moved| fitted.map_or_else(|| Point::new(moved.x, moved.y), Ok))
        .transpose()?;
    on_board(at.as_ref(), None)?;
    Ok(ObjectPatch {
        at: patch.at.or(at),
        ..patch.clone()
    })
}

/// The ids of the tags with these titles, in order: a title nobody has yet
/// makes a new tag.
fn tag_ids(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    titles: &[String],
) -> Result<Vec<String>> {
    titles
        .iter()
        .map(|title| title.trim())
        .filter(|title| !title.is_empty())
        .map(|title| tag_id(transaction, scope, title))
        .collect::<Result<Vec<_>>>()
        .map(|ids| ids.into_iter().unique().collect())
}

fn tag_id(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    title: &str,
) -> Result<String> {
    let wanted = normalized(title);
    let defined = scope
        .roots
        .tags
        .iter(&*transaction)
        .filter_map(|(id, definition)| {
            let definition = stored_json(&definition.to_json(&*transaction))?;
            let name = definition.get(TITLE)?.as_str()?.to_owned();
            (normalized(&name) == wanted).then(|| id.to_owned())
        })
        .min();
    defined.map_or_else(|| new_tag(transaction, scope, title), Ok)
}

fn new_tag(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    title: &str,
) -> Result<String> {
    let id = scope.context.ids.new_id();
    let pairs = [(ID, id.as_str()), (TITLE, title), (COLOR, NEW_TAG_COLOR)]
        .map(|(name, value)| (name.to_owned(), json!(value)));
    insert_map(transaction, &scope.roots.tags, id.as_str(), pairs)?;
    Ok(id)
}

/// What a copy is made of: the objects, and the ids the copies get.
struct Pasting<'a> {
    scope: &'a WriteTransaction<'a>,
    pasted: &'a Pasted,
    made: HashMap<&'a str, String>,
}

/// Puts copies of the objects on the board, each past the layers before it.
pub(super) fn paste(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    pasted: &Pasted,
) -> Result<Vec<ChangedObject>> {
    let pasting = Pasting {
        scope,
        pasted,
        made: pasted
            .objects
            .iter()
            .map(|object| (object.id.as_str(), scope.context.ids.new_id()))
            .collect(),
    };
    pasted
        .objects
        .iter()
        .try_fold(Running::start(scope), |running, object| {
            pasting.one(transaction, running, object)
        })
        .map(|done| done.touched)
}

impl Pasting<'_> {
    fn one(
        &self,
        transaction: &mut TransactionMut<'_>,
        running: Running,
        object: &Object,
    ) -> Result<Running> {
        let id = self.made.get(object.id.as_str()).cloned().ok_or_else(|| {
            Error::BoardChange(BoardChangeError::NoObject {
                reference: object.id.clone(),
            })
        })?;
        let laid = self.laid(object, &id, &running.states);
        let stored = self.with_content(transaction, object, laid)?;
        let pairs = stored
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.clone()));
        insert_map(transaction, &self.scope.roots.objects, id.as_str(), pairs)?;
        let made = ChangedObject::created(id.clone(), object.kind.clone());
        Ok(running.after(vec![(id.clone(), stored)], vec![made]))
    }

    /// The copy's fields but its text and tags: new id, layer, author, place
    /// and parent.
    fn laid(
        &self,
        object: &Object,
        id: &str,
        states: &HashMap<String, Value>,
    ) -> Value {
        let base = object.fields.as_object().cloned().unwrap_or_default();
        let moved = self.moved(object, &base);
        let placed =
            Value::Object(base.clone().into_iter().chain(moved.clone()).collect());
        let stacking = if object.kind == ObjectKind::Frame {
            Edge::Back
        } else {
            Edge::Front
        };
        let own = new_header(
            id,
            &object.kind,
            beyond(states, id, stacking),
            self.scope.stamp,
        )
        .map(|(name, value)| (name.to_owned(), value));
        let parent = (
            PARENT_ID.to_owned(),
            json!(self.parent(object, &placed, states)),
        );
        Value::Object(
            base.into_iter()
                .filter(|(name, _)| name != PARENT_ID)
                .chain(moved)
                .chain(own)
                .chain([parent])
                .collect(),
        )
    }

    /// The fields the shift changes: a corner, or an arrow's ends.
    fn moved(
        &self,
        object: &Object,
        base: &serde_json::Map<String, Value>,
    ) -> Vec<(String, Value)> {
        let shift = (self.pasted.shift.x(), self.pasted.shift.y());
        if object.kind == ObjectKind::Arrow {
            self.arrow_moved(object, base, shift)
        } else {
            base.get(POSITION)
                .map(|corner| (POSITION.to_owned(), shifted(corner, shift)))
                .into_iter()
                .collect()
        }
    }

    /// An arrow's ends and bends shifted, the ends bound to the copies.
    fn arrow_moved(
        &self,
        object: &Object,
        base: &serde_json::Map<String, Value>,
        shift: (f64, f64),
    ) -> Vec<(String, Value)> {
        let moving: BTreeSet<String> =
            self.made.keys().map(|old| (*old).to_owned()).collect();
        let parts = arrow_moved(&object.fields, &moving, shift);
        let bends = parts
            .iter()
            .filter(|(name, _)| *name != START && *name != END)
            .map(|(name, value)| ((*name).to_owned(), value.clone()));
        let ends = [START, END].into_iter().filter_map(|side| {
            let end = parts
                .iter()
                .find(|(name, _)| *name == side)
                .map(|(_, value)| value)
                .or_else(|| base.get(side))?;
            Some((side.to_owned(), self.bound(end)))
        });
        bends.chain(ends).collect()
    }

    /// An arrow end bound to the copy of the object it was bound to.
    fn bound(&self, end: &Value) -> Value {
        end.get(OBJECT_ID)
            .and_then(Value::as_str)
            .and_then(|old| self.made.get(old))
            .map_or_else(|| end.clone(), |new| with_field(end, OBJECT_ID, json!(new)))
    }

    /// The copy of its parent, else the frame it lands in; arrows have none.
    fn parent(
        &self,
        object: &Object,
        placed: &Value,
        states: &HashMap<String, Value>,
    ) -> Option<String> {
        let copied = object
            .parent
            .as_deref()
            .and_then(|parent| self.made.get(parent))
            .cloned();
        copied.or_else(|| {
            (object.kind != ObjectKind::Arrow)
                .then(|| {
                    rectangle_of(placed).and_then(|area| {
                        parent_frame(states, &area, &BTreeSet::new(), None)
                    })
                })
                .flatten()
        })
    }

    /// `laid` with a new document for the text and the tags found by title.
    fn with_content(
        &self,
        transaction: &mut TransactionMut<'_>,
        object: &Object,
        laid: Value,
    ) -> Result<Value> {
        let context = self.scope.context;
        let document =
            object
                .fields
                .get(DOCUMENT_ID)
                .and_then(Value::as_str)
                .map(|_| {
                    let document = context.ids.new_id();
                    let root = self.scope.roots.documents.insert(
                        transaction,
                        document.as_str(),
                        XmlTextPrelim::new(""),
                    );
                    fill_text(transaction, &root, &object.text, context.ids);
                    document
                });
        let titles: Option<Vec<String>> = object
            .fields
            .get(TAG_IDS)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|old| self.pasted.tags.get(old).cloned())
                    .collect()
            });
        let tags = titles
            .map(|titles| tag_ids(transaction, self.scope, &titles))
            .transpose()?;
        Ok([
            document.map(|document| (DOCUMENT_ID, json!(document))),
            tags.map(|ids| (TAG_IDS, json!(ids))),
        ]
        .into_iter()
        .flatten()
        .fold(laid, |fields, (name, value)| {
            with_field(&fields, name, value)
        }))
    }
}
