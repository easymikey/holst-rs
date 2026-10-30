//! A new object with its texts, parts and store entries.

use std::collections::HashMap;

use serde_json::{Value, json};
use yrs::{
    ArrayPrelim,
    Map as _,
    MapPrelim,
    MapRef,
    Text as _,
    TransactionMut,
    WriteTxn as _,
    XmlTextPrelim,
};

use crate::{
    domain::{
        board::{any, insert_map},
        change::{
            check::Placement,
            value::{ChangedObject, Made, StoreEntry},
            write::{Edge, WriteTransaction, Written, beyond, fill_as, new_header},
        },
        rich_text::TextFormat,
    },
    error::Result,
    protocol::document::{DOCUMENT_ID, PARENT_ID, POSITION},
};

/// What `made` stores in its own map beside the header, the ids of its texts
/// and its parent: `position` and the fields it names.
pub(super) fn own_fields(made: &Made) -> Vec<(String, Value)> {
    std::iter::once((POSITION.to_owned(), json!(made.at)))
        .chain(
            made.fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone())),
        )
        .collect()
}

/// The objects written so far, which the layer of the next one goes past.
struct Writing<'a> {
    scope: &'a WriteTransaction<'a>,
    format: TextFormat,
    states: HashMap<String, Value>,
    /// The parts, at any depth, with the fields each was written with.
    held: Vec<(String, Value)>,
}

/// `made` written in front of the board, its parts and entries with it.
pub(super) fn write(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    made: &Made,
    placement: &Placement,
) -> Result<Written> {
    let mut writing = Writing {
        scope,
        format: placement.format,
        states: scope.states.clone(),
        held: Vec::new(),
    };
    let (id, fields) =
        put(transaction, &mut writing, made, placement.parent.as_deref())?;
    Ok(Written {
        changed: ChangedObject::created(id, made.kind.clone()),
        fields,
        parts: writing.held,
    })
}

fn put(
    transaction: &mut TransactionMut<'_>,
    writing: &mut Writing<'_>,
    made: &Made,
    parent: Option<&str>,
) -> Result<(String, Value)> {
    let scope = writing.scope;
    let ids = scope.context.ids;
    let id = ids.new_id();
    let documents = made
        .texts
        .iter()
        .map(|(field, text)| {
            let document = ids.new_id();
            let root = scope.roots.documents.insert(
                transaction,
                document.as_str(),
                XmlTextPrelim::new(""),
            );
            fill_as(transaction, &root, text, (writing.format, &made.kind), ids);
            ((*field).to_owned(), json!(document))
        })
        .collect::<Vec<_>>();
    let code = made.code.as_ref().map(|code| {
        let document = ids.new_id();
        let root = transaction.get_or_insert_text(format!("code-mirror-{document}"));
        root.insert(transaction, 0, code);
        (DOCUMENT_ID.to_owned(), json!(document))
    });
    let layer = beyond(&writing.states, &id, Edge::Front);
    let header = new_header(&id, &made.kind, layer, scope.stamp)
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value));
    let pairs: Vec<(String, Value)> = header
        .chain(own_fields(made))
        .chain(documents)
        .chain(code)
        .chain([(PARENT_ID.to_owned(), json!(parent))])
        .collect();
    insert_map(transaction, &scope.roots.objects, &id, pairs.clone())?;
    let fields = Value::Object(pairs.into_iter().collect());
    writing.states.insert(id.clone(), fields.clone());
    made.parts.iter().try_for_each(|part| {
        let written = put(transaction, writing, part, Some(&id))?;
        writing.held.push(written);
        Ok::<_, crate::error::Error>(())
    })?;
    made.entries
        .iter()
        .try_for_each(|entry| store(transaction, entry))?;
    Ok((id, fields))
}

/// `entry` in its store, unless it is to be kept and the store has one.
pub(super) fn store(
    transaction: &mut TransactionMut<'_>,
    entry: &StoreEntry,
) -> Result<()> {
    let stored = transaction.get_or_insert_map(entry.store);
    let present = stored.get(&*transaction, entry.id.as_str()).is_some();
    Some(entry)
        .filter(|entry| !(entry.keep && present))
        .map_or(Ok(()), |entry| insert_entry(transaction, &stored, entry))
}

fn insert_entry(
    transaction: &mut TransactionMut<'_>,
    stored: &MapRef,
    entry: &StoreEntry,
) -> Result<()> {
    let fields = entry
        .fields
        .iter()
        .map(|(name, value)| any(value).map(|value| ((*name).to_owned(), value)))
        .collect::<Result<Vec<_>>>()?;
    let map =
        stored.insert(transaction, entry.id.as_str(), MapPrelim::from_iter(fields));
    entry.lists.iter().try_for_each(|(name, items)| {
        let items = items.iter().map(any).collect::<Result<Vec<_>>>()?;
        map.insert(transaction, *name, ArrayPrelim::from_iter(items));
        Ok(())
    })
}
