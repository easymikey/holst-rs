//! The web client's deletion journal, `delete-events-metaStore`; pure.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map as Fields, Value, json};
use yrs::{Any, Map as _, MapPrelim, MapRef, TransactionMut};

use crate::{
    domain::{board::any, change::write::AuthorStamp},
    error::Result,
    protocol::document::TYPE,
};

pub(crate) const JOURNAL: &str = "delete-events-metaStore";

const ID_DIGITS: usize = 5;

const HASH_MODULUS: u64 = 0x7fff_ffff;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemovedObject {
    pub(super) id: String,
    pub(crate) fields: Fields<String, Value>,
}

pub(crate) fn entry_id<'a>(ids: impl IntoIterator<Item = &'a str>) -> String {
    let sorted: BTreeSet<&str> = ids.into_iter().collect();
    let hash = sorted
        .into_iter()
        .flat_map(str::encode_utf16)
        .fold(0_u64, |hash, unit| {
            (hash * 31 + u64::from(unit)) % HASH_MODULUS
        });
    base36(hash).chars().take(ID_DIGITS).collect()
}

fn base36(number: u64) -> String {
    let digits: Vec<char> =
        std::iter::successors(Some(number), |rest| (*rest >= 36).then_some(rest / 36))
            .filter_map(|rest| char::from_digit(u32::try_from(rest % 36).ok()?, 36))
            .collect();
    digits.into_iter().rev().collect()
}

pub(crate) fn entry(
    states: &HashMap<String, Value>,
    gone: &[&str],
    stamp: AuthorStamp,
) -> RemovedObject {
    let snapshots: Vec<Value> = gone
        .iter()
        .filter_map(|id| states.get(*id).cloned())
        .collect();
    let kinds: BTreeSet<&str> = snapshots
        .iter()
        .filter_map(|state| state.get(TYPE).and_then(Value::as_str))
        .collect();
    let single = match kinds.into_iter().collect::<Vec<_>>().as_slice() {
        [kind] => Some((*kind).to_owned()),
        _ => None,
    };
    let locked = snapshots
        .iter()
        .any(|state| state.get("locked") == Some(&Value::Bool(true)));
    let unlock = single.as_deref() == Some("group") && locked;
    let id = entry_id(gone.iter().copied());
    let common = json!({
        "id": id, "a": stamp.author, "t": stamp.at.as_millis(),
        "cnt": snapshots.len(), "s": snapshots,
    });
    let optional = [
        single.map(|kind| ("type", Value::String(kind))),
        unlock.then_some(("unlock", Value::Bool(true))),
    ];
    let fields = common
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .chain(
            optional
                .into_iter()
                .flatten()
                .map(|(name, value)| (name.to_owned(), value)),
        )
        .collect();
    RemovedObject { id, fields }
}

pub(super) fn write(
    transaction: &mut TransactionMut<'_>,
    journal: &MapRef,
    entry: RemovedObject,
) -> Result<()> {
    let fields = entry
        .fields
        .into_iter()
        .map(|(name, value)| any(&value).map(|value| (name, value)))
        .collect::<Result<Vec<(String, Any)>>>()?;
    journal.insert(transaction, entry.id.as_str(), MapPrelim::from_iter(fields));
    Ok(())
}
