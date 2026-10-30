//! A table: how many rows and columns it has.

use serde_json::{Value, json};

use super::Detail;
use crate::{
    domain::board::Object,
    protocol::document::{COLUMN_PREFIX, ROW_PREFIX},
};

/// The `rows` and `columns` of a table: the keys of its fields that hold an
/// object under `row-` and `column-`.
pub(super) fn detail(table: &Object) -> Detail {
    let count = |prefix: &str| {
        table
            .fields
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, track)| key.starts_with(prefix) && track.is_object())
            .count()
    };
    Detail {
        text: None,
        words: [
            ("rows", count(ROW_PREFIX)),
            ("columns", count(COLUMN_PREFIX)),
        ]
        .into_iter()
        .filter(|(_, tracks)| *tracks > 0)
        .map(|(word, tracks)| (word, json!(tracks)))
        .collect::<Vec<(&'static str, Value)>>(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yrs::{Any, Doc, Map as _, Transact as _};

    use crate::domain::{board::objects, kinds::detail};

    fn words_of(table: &serde_json::Value) -> Vec<String> {
        let document = Doc::new();
        let store = document.get_or_insert_map("objects");
        {
            // imperative: yrs inserts through one `&mut` transaction
            let mut txn = document.transact_mut();
            store.insert(
                &mut txn,
                "t1",
                Any::from_json(&table.to_string()).unwrap_or(Any::Null),
            );
        }
        objects(&document)
            .iter()
            .flat_map(|object| detail(&document, object).words)
            .map(|(word, value)| format!("{word}={value}"))
            .collect()
    }

    #[test]
    fn a_table_counts_its_rows_and_columns_not_its_cells() {
        let words = words_of(&json!({
            "id": "t1", "type": "table", "position": {"x": 0, "y": 0},
            "row-1": {"index": 0, "height": 60}, "row-2": {"index": 1},
            "column-1": {"index": 0}, "column-2": {"index": 1}, "column-3": {"index": 2},
            "rowCount": 9
        }));
        assert_eq!(words, ["rows=2", "columns=3"]);
    }

    #[test]
    fn a_table_without_tracks_has_no_words() {
        let words = words_of(&json!({
            "id": "t1", "type": "table", "position": {"x": 0, "y": 0}, "row-1": 5
        }));
        assert_eq!(words, Vec::<String>::new());
    }
}
