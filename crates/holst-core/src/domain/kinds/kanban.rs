//! A kanban: columns and swimlanes in their own stores and a first empty card.

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{Doc, Map as _, ReadTxn, Transact as _, types::ToJson as _};

use super::{Detail, task};
use crate::{
    domain::{
        board::{Object, ObjectKind, json},
        change::value::{Made, StoreEntry},
        geometry::{Point, Size},
    },
    protocol::document::{INDEX, TITLE, WIDTH},
};

pub(crate) const STORED: &str = "kanban";
const INFO_STORE: &str = "kanban-infoStore";
const COLUMN_STORE: &str = "kanban-columnsStore";
const SWIMLANE_STORE: &str = "kanban-swimlanesStore";
const INFO_ID: &str = "kanbanInfoId";
const COLUMNS: &str = "columns";
const SWIMLANES: &str = "swimlanes";

/// The step between the `index` of one column or swimlane and the next.
const STEP: usize = 10_000;
/// The room a column and a kanban take in the free-corner search.
pub(crate) const COLUMN_WIDTH: f64 = 340.0;
/// A card in a cell is as wide as the column; a free card is `task::WIDE`.
pub(crate) const CARD_WIDTH: f64 = 338.0;
const HEIGHT: f64 = 400.0;

/// The columns of a new kanban in the web client, in English.
const DEFAULT_COLUMNS: [&str; 3] = ["To do", "In progress", "Done"];
/// The swimlane of a new kanban.
const DEFAULT_SWIMLANES: [&str; 1] = ["Untitled"];

/// Whether the object is a kanban.
#[must_use]
pub fn is_kanban(object: &Object) -> bool {
    object.kind.as_str() == STORED
}

/// Kanban of the given column and swimlane titles, else defaults, with an
/// empty first card in the first cell; ids from `new_id`, corner set by caller.
#[must_use]
pub fn made(
    columns: &[String],
    swimlanes: &[String],
    mut new_id: impl FnMut() -> String,
) -> Made {
    let columns = cells(columns, &DEFAULT_COLUMNS, &mut new_id);
    let swimlanes = cells(swimlanes, &DEFAULT_SWIMLANES, &mut new_id);
    let info = new_id();
    let ids = |cells: &[(String, String)]| {
        cells.iter().map(|(id, _)| json!(id)).collect::<Vec<_>>()
    };
    let first = columns.first().zip(swimlanes.first());
    let card = task::made("", None, (None, None), None, new_id());
    let entries = stored(COLUMN_STORE, &columns)
        .chain(stored(SWIMLANE_STORE, &swimlanes))
        .chain([StoreEntry {
            store: INFO_STORE,
            id: info.clone(),
            fields: Vec::new(),
            lists: vec![(COLUMNS, ids(&columns)), (SWIMLANES, ids(&swimlanes))],
            keep: false,
        }])
        .collect();
    let wide = u32::try_from(columns.len()).map_or(f64::MAX, f64::from);
    Made {
        kind: ObjectKind::Other(STORED.to_owned()),
        at: Point::ORIGIN,
        size: Size::new(wide * COLUMN_WIDTH, HEIGHT).ok(),
        parent: None,
        fields: vec![(INFO_ID, json!(info))],
        texts: Vec::new(),
        code: None,
        parts: first
            .map(|((column, _), (swimlane, _))| Made {
                fields: card
                    .fields
                    .into_iter()
                    .filter(|(name, _)| ![WIDTH, task::DISPLAY_OPTIONS].contains(name))
                    .chain([
                        (WIDTH, json!(CARD_WIDTH)),
                        // The web client's first card has no display options.
                        (task::DISPLAY_OPTIONS, Value::Null),
                        (task::SWIMLANE_ID, json!(swimlane)),
                        (task::COLUMN_ID, json!(column)),
                    ])
                    .collect(),
                ..card
            })
            .into_iter()
            .collect(),
        entries,
    }
}

/// An id and the title for each of `given`, or of `default` when none is.
fn cells(
    given: &[String],
    default: &[&str],
    new_id: &mut impl FnMut() -> String,
) -> Vec<(String, String)> {
    let titles: Vec<String> = match given {
        [] => default.iter().map(|title| (*title).to_owned()).collect(),
        _ => given.to_vec(),
    };
    titles.into_iter().map(|title| (new_id(), title)).collect()
}

/// A store entry for each cell, indexed in steps from 0.
fn stored(
    store: &'static str,
    cells: &[(String, String)],
) -> impl Iterator<Item = StoreEntry> {
    cells
        .iter()
        .zip((0_u64..).step_by(STEP))
        .map(move |((id, title), index)| StoreEntry {
            store,
            id: id.clone(),
            fields: vec![(TITLE, json!(title)), (INDEX, json!(index))],
            lists: Vec::new(),
            keep: false,
        })
}

/// The id and title of each column of `kanban`, by index.
#[must_use]
pub fn columns(document: &Doc, kanban: &Object) -> Vec<(String, String)> {
    listed(document, kanban, (COLUMNS, COLUMN_STORE))
}

/// The id and title of each swimlane of `kanban`, by index.
#[must_use]
pub fn swimlanes(document: &Doc, kanban: &Object) -> Vec<(String, String)> {
    listed(document, kanban, (SWIMLANES, SWIMLANE_STORE))
}

/// The columns of the kanban stored as `kanban`, read through `txn`.
pub(crate) fn columns_in<T: ReadTxn>(txn: &T, kanban: &Value) -> Vec<(String, String)> {
    cells_in(txn, kanban, (COLUMNS, COLUMN_STORE))
}

/// The swimlanes of the kanban stored as `kanban`, read through `txn`.
pub(crate) fn swimlanes_in<T: ReadTxn>(
    txn: &T,
    kanban: &Value,
) -> Vec<(String, String)> {
    cells_in(txn, kanban, (SWIMLANES, SWIMLANE_STORE))
}

/// The cells the info entry lists under `list`, titled by `store`.
fn listed(
    document: &Doc,
    kanban: &Object,
    which: (&str, &str),
) -> Vec<(String, String)> {
    // The roots exist before a read-only transaction looks for them.
    let _roots = [INFO_STORE, which.1].map(|name| document.get_or_insert_map(name));
    cells_in(&document.transact(), &kanban.fields, which)
}

fn cells_in<T: ReadTxn>(
    txn: &T,
    kanban: &Value,
    (list, store): (&str, &str),
) -> Vec<(String, String)> {
    let ids = kanban
        .get(INFO_ID)
        .and_then(Value::as_str)
        .and_then(|id| txn.get_map(INFO_STORE)?.get(txn, id))
        .and_then(|info| json(&info.to_json(txn)))
        .and_then(|info| info.get(list).cloned());
    let stored = txn
        .get_map(store)
        .and_then(|entries| json(&entries.to_json(txn)))
        .unwrap_or_default();
    ids.iter()
        .flat_map(|ids| ids.as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .filter_map(|id| {
            let entry = stored.get(id)?;
            let index = entry.get(INDEX).and_then(Value::as_f64).unwrap_or_default();
            let title = entry.get(TITLE)?.as_str()?;
            Some((index, id.to_owned(), title.to_owned()))
        })
        .sorted_by(|(first, ..), (second, ..)| first.total_cmp(second))
        .map(|(_, id, title)| (id, title))
        .collect()
}

/// The title of the column and of the swimlane a task card sits in, as words.
pub(super) fn cell_words(
    document: &Doc,
    card: &Object,
) -> [(&'static str, Option<Value>); 2] {
    let titled = |(field, store): (&str, &str)| {
        let id = card.fields.get(field).and_then(Value::as_str)?;
        let entries = document.get_or_insert_map(store);
        let txn = document.transact();
        let entry = json(&entries.get(&txn, id)?.to_json(&txn))?;
        entry.get(TITLE).cloned()
    };
    [
        ("column", titled((task::COLUMN_ID, COLUMN_STORE))),
        ("swimlane", titled((task::SWIMLANE_ID, SWIMLANE_STORE))),
    ]
}

/// The titles of the columns and of the swimlanes as words.
pub(super) fn detail(document: &Doc, kanban: &Object) -> Detail {
    let joined = |cells: Vec<(String, String)>| {
        Some(cells.into_iter().map(|(_, title)| title).join(","))
            .filter(|titles| !titles.is_empty())
            .map(Value::from)
    };
    Detail {
        text: None,
        words: [
            (COLUMNS, joined(columns(document, kanban))),
            (SWIMLANES, joined(swimlanes(document, kanban))),
        ]
        .into_iter()
        .filter_map(|(word, value)| value.map(|value| (word, value)))
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use yrs::{Any, ArrayPrelim, Doc, Map as _, MapPrelim, Transact as _};

    use super::{columns, detail, made, swimlanes};
    use crate::domain::{board::object_of, change::value::StoreEntry};

    fn counter() -> impl FnMut() -> String {
        let mut next = 0;
        move || {
            next += 1;
            format!("k{next}")
        }
    }

    fn titles(given: &[&str]) -> Vec<String> {
        given.iter().map(|title| (*title).to_owned()).collect()
    }

    fn entry<'a>(
        entries: &'a [StoreEntry],
        store: &str,
        id: &str,
    ) -> Option<&'a StoreEntry> {
        entries
            .iter()
            .find(|entry| entry.store == store && entry.id == id)
    }

    #[test]
    fn a_kanban_stores_columns_and_swimlanes_by_id_with_indexes_in_steps() {
        let kanban = made(&titles(&["Now", "Next"]), &titles(&["Team"]), counter());
        let columns = |id: &str| entry(&kanban.entries, "kanban-columnsStore", id);
        let info = entry(&kanban.entries, "kanban-infoStore", "k4");
        assert_eq!(
            (
                columns("k1").map(|entry| entry.fields.clone()),
                columns("k2").map(|entry| entry.fields.clone()),
                entry(&kanban.entries, "kanban-swimlanesStore", "k3")
                    .map(|entry| entry.fields.clone()),
                info.map(|entry| entry.lists.clone()),
                kanban.fields.clone(),
            ),
            (
                Some(vec![("title", json!("Now")), ("index", json!(0))]),
                Some(vec![("title", json!("Next")), ("index", json!(10_000))]),
                Some(vec![("title", json!("Team")), ("index", json!(0))]),
                Some(vec![
                    ("columns", vec![json!("k1"), json!("k2")]),
                    ("swimlanes", vec![json!("k3")]),
                ]),
                vec![("kanbanInfoId", json!("k4"))],
            )
        );
    }

    #[test]
    fn a_kanban_without_titles_has_the_web_columns_and_one_untitled_swimlane() {
        let kanban = made(&[], &[], counter());
        let stored = |store: &str| {
            kanban
                .entries
                .iter()
                .filter(|entry| entry.store == store)
                .map(|entry| entry.fields.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            (
                stored("kanban-columnsStore"),
                stored("kanban-swimlanesStore")
            ),
            (
                vec![
                    vec![("title", json!("To do")), ("index", json!(0))],
                    vec![("title", json!("In progress")), ("index", json!(10_000))],
                    vec![("title", json!("Done")), ("index", json!(20_000))],
                ],
                vec![vec![("title", json!("Untitled")), ("index", json!(0))]]
            )
        );
    }

    #[test]
    fn the_first_card_is_an_empty_task_in_the_first_cell() {
        let kanban = made(&titles(&["Now", "Next"]), &[], counter());
        let card = kanban.parts.first();
        let field = |name: &str| {
            card.and_then(|card| {
                card.fields
                    .iter()
                    .find(|(known, _)| *known == name)
                    .map(|(_, value)| value.clone())
            })
        };
        assert_eq!(
            (
                kanban.parts.len(),
                card.map(|card| card.texts.clone()),
                field("columnId"),
                field("swimlaneId"),
                field("index"),
                field("width"),
                field("displayOptions"),
                card.map(|card| card.entries.len()),
            ),
            (
                1,
                Some(vec![("titleDocumentId", String::new())]),
                Some(json!("k1")),
                Some(json!("k3")),
                Some(json!(0x100_0000_0000_u64)),
                Some(json!(338.0)),
                Some(Value::Null),
                Some(6),
            )
        );
    }

    /// A board with a kanban `K` whose columns are stored out of order.
    fn board() -> Doc {
        let document = Doc::new();
        let infos = document.get_or_insert_map("kanban-infoStore");
        let stores = ["kanban-columnsStore", "kanban-swimlanesStore"]
            .map(|name| document.get_or_insert_map(name));
        let mut transaction = document.transact_mut();
        let info = infos.insert(
            &mut transaction,
            "i1",
            MapPrelim::from_iter([("unused", Any::from(0))]),
        );
        info.insert(
            &mut transaction,
            "columns",
            ArrayPrelim::from_iter([Any::from("c2"), Any::from("c1")]),
        );
        info.insert(
            &mut transaction,
            "swimlanes",
            ArrayPrelim::from_iter([Any::from("s1")]),
        );
        let [columns, swimlanes] = stores;
        [(&columns, "c1", "Now", 0), (&columns, "c2", "Next", 10_000)]
            .into_iter()
            .chain([(&swimlanes, "s1", "Team", 0)])
            .for_each(|(store, id, title, index)| {
                store.insert(
                    &mut transaction,
                    id,
                    MapPrelim::from_iter([
                        ("title", Any::from(title)),
                        ("index", Any::from(index)),
                    ]),
                );
            });
        drop(transaction);
        document
    }

    #[test]
    fn columns_and_swimlanes_are_read_back_in_index_order() {
        let document = board();
        let kanban = object_of(json!({
            "id": "K", "type": "kanban", "position": {"x": 0, "y": 0},
            "kanbanInfoId": "i1"
        }));
        let shown = kanban.iter().map(|object| {
            (
                columns(&document, object),
                swimlanes(&document, object),
                detail(&document, object).words,
            )
        });
        assert_eq!(
            shown.collect::<Vec<_>>(),
            vec![(
                vec![
                    ("c1".to_owned(), "Now".to_owned()),
                    ("c2".to_owned(), "Next".to_owned())
                ],
                vec![("s1".to_owned(), "Team".to_owned())],
                vec![("columns", json!("Now,Next")), ("swimlanes", json!("Team"))]
            )]
        );
    }

    #[test]
    fn a_kanban_without_an_info_entry_lists_no_words() {
        let kanban = object_of(json!({
            "id": "K", "type": "kanban", "position": {"x": 0, "y": 0}
        }));
        let words = kanban
            .iter()
            .map(|object| detail(&Doc::new(), object).words)
            .collect::<Vec<_>>();
        assert_eq!(words, vec![Vec::new()]);
    }
}
