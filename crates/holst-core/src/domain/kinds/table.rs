//! A table: how many rows and columns it has.

use std::sync::LazyLock;

use serde_json::{Value, json};

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
    },
    protocol::document::{
        COLOR,
        COLUMN_PREFIX,
        FILL_COLOR,
        HEIGHT,
        INDEX,
        OPACITY,
        ROW_PREFIX,
        STROKE_COLOR,
        WIDTH,
    },
};

/// The most cells a table holds in the web client (main 205145).
pub const CELL_LIMIT: u32 = 2500;

/// The rows and columns of a table the web client places.
pub const DEFAULT_ROWS: u32 = 2;
pub const DEFAULT_COLUMNS: u32 = 3;

pub(crate) const ROW_HEIGHT: u32 = 60;
pub(crate) const COLUMN_WIDTH: u32 = 240;
/// The step between the `index` of one track and the next (main 223456).
const INDEX_STEP: u64 = 0x100_0000_0000;
pub(crate) const MIN_HEIGHT: &str = "minHeight";
const FILL_COLOR_INDEX: &str = "fillColorIndex";

/// The keys a table can hold, which `Made` borrows for good.
static ROW_KEYS: LazyLock<Vec<String>> = LazyLock::new(|| keys(ROW_PREFIX));
static COLUMN_KEYS: LazyLock<Vec<String>> = LazyLock::new(|| keys(COLUMN_PREFIX));

fn keys(prefix: &str) -> Vec<String> {
    (1..=CELL_LIMIT).map(|n| format!("{prefix}{n}")).collect()
}

/// Table of `rows` by `columns` as the web's tool places it; no cell objects.
#[must_use]
pub fn made(rows: u32, columns: u32) -> Made {
    let paint = |color| json!({ (COLOR): color, (OPACITY): 1 });
    let index = |n: u32| INDEX_STEP * u64::from(n);
    let row = |n| json!({ (INDEX): index(n), (HEIGHT): ROW_HEIGHT, (MIN_HEIGHT): ROW_HEIGHT });
    let column = |n| json!({ (INDEX): index(n), (WIDTH): COLUMN_WIDTH });
    Made {
        kind: ObjectKind::Table,
        at: Point::ORIGIN,
        size: Size::new(
            f64::from(columns) * f64::from(COLUMN_WIDTH),
            f64::from(rows) * f64::from(ROW_HEIGHT),
        )
        .ok(),
        parent: None,
        fields: [
            (STROKE_COLOR, paint("gray7")),
            (FILL_COLOR, paint("white6")),
            (FILL_COLOR_INDEX, json!(1)),
        ]
        .into_iter()
        .chain(
            ROW_KEYS
                .iter()
                .zip(1..=rows)
                .map(|(key, n)| (key.as_str(), row(n))),
        )
        .chain(
            COLUMN_KEYS
                .iter()
                .zip(1..=columns)
                .map(|(key, n)| (key.as_str(), column(n))),
        )
        .collect(),
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// Row and column counts: fields keyed `row-` or `column-` that hold an object.
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
    use serde_json::{Value, json};
    use yrs::{Any, Doc, Map as _, Transact as _};

    use super::{CELL_LIMIT, made};
    use crate::domain::{
        board::{ObjectKind, objects},
        kinds::detail,
    };

    #[test]
    fn a_made_table_has_the_tracks_and_colours_of_the_toolbar_tool() {
        let table = made(2, 3);
        let stored = Value::Object(
            table
                .fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
        );
        assert_eq!(
            (
                table.kind,
                stored,
                table.size.map(|size| (size.width(), size.height()))
            ),
            (
                ObjectKind::Table,
                json!({
                    "strokeColor": {"color": "gray7", "opacity": 1},
                    "fillColor": {"color": "white6", "opacity": 1},
                    "fillColorIndex": 1,
                    "row-1": {"index": 1_099_511_627_776_u64, "height": 60, "minHeight": 60},
                    "row-2": {"index": 2_199_023_255_552_u64, "height": 60, "minHeight": 60},
                    "column-1": {"index": 1_099_511_627_776_u64, "width": 240},
                    "column-2": {"index": 2_199_023_255_552_u64, "width": 240},
                    "column-3": {"index": 3_298_534_883_328_u64, "width": 240},
                }),
                Some((720.0, 120.0))
            )
        );
    }

    #[test]
    fn a_table_of_the_cell_limit_in_one_row_keeps_every_column() {
        let columns = made(1, CELL_LIMIT).fields.len();
        assert_eq!(
            columns,
            3 + 1 + usize::try_from(CELL_LIMIT).unwrap_or_default()
        );
    }

    fn words_of(table: &Value) -> Vec<String> {
        let document = Doc::new();
        let store = document.get_or_insert_map("objects");
        {
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
