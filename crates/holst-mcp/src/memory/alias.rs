use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use serde_json::Value;

use crate::memory::lock;

const ALIAS_CAPACITY: usize = 1 << 17;

/// Id number `first + k + 1` sits at `order[k]`; the oldest fall off the front.
#[derive(Default)]
struct AliasTable {
    by_id: HashMap<Arc<str>, usize>,
    order: VecDeque<Arc<str>>,
    first: usize,
}

pub(crate) struct Aliases {
    table: Mutex<AliasTable>,
    capacity: usize,
}

impl Default for Aliases {
    fn default() -> Self {
        Self::with_capacity(ALIAS_CAPACITY)
    }
}

impl Aliases {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            table: Mutex::default(),
            capacity: capacity.max(1),
        }
    }

    pub(crate) fn intern(&self, id: &str) -> usize {
        // imperative: the guard is mutated in several steps.
        let mut table = lock(&self.table);
        let known = table.by_id.get(id).copied();
        known.unwrap_or_else(|| {
            let id: Arc<str> = id.into();
            table.order.push_back(Arc::clone(&id));
            let number = table.first.saturating_add(table.order.len());
            table.by_id.insert(id, number);
            if table.order.len() > self.capacity {
                table.first = table.first.saturating_add(1);
                if let Some(oldest) = table.order.pop_front() {
                    table.by_id.remove(&oldest);
                }
            }
            number
        })
    }

    pub(crate) fn id(&self, number: usize) -> Option<String> {
        let table = lock(&self.table);
        table
            .order
            .get(number.checked_sub(table.first)?.checked_sub(1)?)
            .map(ToString::to_string)
    }

    #[cfg(test)]
    pub(crate) fn held(&self) -> (usize, usize) {
        let table = lock(&self.table);
        (table.order.len(), table.by_id.len())
    }
}

pub(crate) fn alias_text(text: &str, mut alias: impl FnMut(&str) -> String) -> String {
    let (out, copied) = holst_core::domain::id::uuid_spans(text).fold(
        (String::with_capacity(text.len()), 0),
        |(mut out, from), (start, end)| {
            out.push_str(text.get(from..start).unwrap_or_default());
            out.push_str(&alias(text.get(start..end).unwrap_or_default()));
            (out, end)
        },
    );
    out + text.get(copied..).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{alias} is not from this session (did the server restart?): take an @N from a fresh \
     search or list"
)]
pub(crate) struct UnknownAlias {
    pub(crate) alias: String,
}

pub(crate) fn expand_path<E>(
    path: &str,
    expand: impl Fn(&str) -> Result<String, E>,
) -> Result<String, E> {
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(path, query)| (path, Some(query)));
    // One buffer for the whole path: segments are appended in place.
    path.split('/')
        .enumerate()
        .try_fold(
            String::with_capacity(path.len()),
            |mut out, (i, segment)| {
                out.extend((i > 0).then_some('/'));
                out.push_str(&expand(segment)?);
                Ok(out)
            },
        )
        .map(|mut out| {
            out.extend(query.into_iter().flat_map(|query| ["?", query]));
            out
        })
}

pub(crate) fn expand_json<E>(
    value: &mut Value,
    expand: &impl Fn(&str) -> Result<String, E>,
) -> Result<(), E> {
    match value {
        Value::String(s) if s.starts_with('@') => expand(s).map(|alias| *s = alias),
        Value::Array(items) => items
            .iter_mut()
            .try_for_each(|item| expand_json(item, expand)),
        Value::Object(map) => map
            .values_mut()
            .try_for_each(|item| expand_json(item, expand)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}
