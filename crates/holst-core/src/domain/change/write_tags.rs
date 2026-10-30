//! A tag definition renamed, recolored or removed, and the references to it.

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{
    Array as _,
    ArrayRef,
    Map as _,
    MapRef,
    ReadTxn,
    TransactionMut,
    types::ToJson as _,
};

use crate::{
    domain::{
        board::{ObjectKind, json as stored_json},
        change::{
            check::asked_name,
            value::{ChangedObject, ObjectChangeKind, TagChange},
            write::{WriteTransaction, changed, insert_changes},
        },
        tree::name_index::build::normalized,
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{COLOR, TAG_IDS, TASK_INFO_STORE, TITLE},
};

/// A tag is not an object; a report names it by this kind.
fn tag_kind() -> ObjectKind {
    ObjectKind::Other("tag".to_owned())
}

/// The tag `reference` names, by id or else by title, changed as asked.
pub(super) fn tag_change(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    reference: &str,
    change: &TagChange,
) -> Result<Vec<ChangedObject>> {
    let defined = definitions(&*transaction, scope);
    let wanted = normalized(reference);
    let id = defined
        .iter()
        .find(|(id, _)| id == reference)
        .or_else(|| {
            defined
                .iter()
                .filter(|(_, title)| normalized(title) == wanted)
                .min_by_key(|(id, _)| id)
        })
        .map(|(id, _)| id.as_str())
        .ok_or_else(|| {
            Error::BoardChange(BoardChangeError::NoTag {
                reference: reference.to_owned(),
            })
        })?;
    match change {
        TagChange { remove: true, .. } => removed(transaction, scope, id),
        TagChange { .. } => updated(transaction, scope, (&defined, id), change),
    }
}

/// The id and title of each tag the board defines.
fn definitions<T: ReadTxn>(
    transaction: &T,
    scope: &WriteTransaction<'_>,
) -> Vec<(String, String)> {
    scope
        .roots
        .tags
        .iter(transaction)
        .filter_map(|(id, definition)| {
            let definition = stored_json(&definition.to_json(transaction))?;
            let title = definition.get(TITLE)?.as_str()?.to_owned();
            Some((id.to_owned(), title))
        })
        .collect()
}

/// The title and colour set, unless another tag has the title.
fn updated(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    (defined, id): (&[(String, String)], &str),
    change: &TagChange,
) -> Result<Vec<ChangedObject>> {
    let name = asked_name(change);
    let taken = name.filter(|asked| {
        defined
            .iter()
            .any(|(other, title)| other != id && normalized(title) == normalized(asked))
    });
    taken.map_or(Ok(()), |title| {
        Err(Error::BoardChange(BoardChangeError::TagTaken {
            title: title.to_owned(),
        }))
    })?;
    let map = scope
        .roots
        .tags
        .get(&*transaction, id)
        .and_then(|definition| definition.cast::<MapRef>().ok())
        .ok_or_else(|| {
            Error::BoardChange(BoardChangeError::NoTag {
                reference: id.to_owned(),
            })
        })?;
    let color = change.color.as_ref();
    let written: Vec<(&'static str, Value)> = [
        name.map(|name| (TITLE, json!(name))),
        color.map(|color| (COLOR, json!(color.as_str()))),
    ]
    .into_iter()
    .flatten()
    .collect();
    insert_changes(transaction, &map, &written)?;
    Ok(vec![ChangedObject {
        change: ObjectChangeKind::Changed,
        fields: [name.map(|_| "name"), color.map(|_| "color")]
            .into_iter()
            .flatten()
            .collect(),
        ..ChangedObject::created(id.to_owned(), tag_kind())
    }])
}

/// The definition deleted, and its id off every object and every task card.
fn removed(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    id: &str,
) -> Result<Vec<ChangedObject>> {
    scope.roots.tags.remove(transaction, id);
    unlisted(transaction, id);
    let objects = scope
        .states
        .iter()
        .filter_map(|(object, fields)| {
            without(fields, id).map(|rest| (object.clone(), rest))
        })
        .sorted_unstable_by(|(first, _), (second, _)| first.cmp(second))
        .map(|(object, rest)| {
            changed(
                transaction,
                scope,
                object,
                &[(TAG_IDS, json!(rest))],
                vec!["tags"],
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok([ChangedObject::removed(id.to_owned(), tag_kind())]
        .into_iter()
        .chain(objects)
        .collect())
}

/// The object's `tagIds` without `id`, if it holds it.
fn without(fields: &Value, id: &str) -> Option<Vec<Value>> {
    let held = fields.get(TAG_IDS)?.as_array()?;
    held.iter().any(|held| held.as_str() == Some(id)).then(|| {
        held.iter()
            .filter(|held| held.as_str() != Some(id))
            .cloned()
            .collect()
    })
}

/// `id` taken out of the `tagIds` list of every task card info.
fn unlisted(transaction: &mut TransactionMut<'_>, id: &str) {
    let store = transaction.get_map(TASK_INFO_STORE);
    let places: Vec<(ArrayRef, u32)> = store
        .iter()
        .flat_map(|store| {
            store
                .iter(&*transaction)
                .map(|(_, info)| info)
                .collect::<Vec<_>>()
        })
        .filter_map(|info| info.cast::<MapRef>().ok())
        .filter_map(|info| info.get(&*transaction, TAG_IDS)?.cast::<ArrayRef>().ok())
        .flat_map(|list| {
            let held = stored_json(&list.to_json(&*transaction))
                .and_then(|held| held.as_array().cloned())
                .unwrap_or_default();
            held.iter()
                .zip(0_u32..)
                .filter(|(held, _)| held.as_str() == Some(id))
                .map(|(_, at)| (list.clone(), at))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
        })
        .collect();
    // imperative: each removal writes to the transaction
    for (list, at) in places {
        list.remove(transaction, at);
    }
}
