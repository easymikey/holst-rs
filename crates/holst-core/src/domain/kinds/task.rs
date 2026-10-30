//! A task card: a title, a description, a status and a schedule.

use std::str::FromStr;

use serde_json::{Map, Value, json};
use yrs::{Doc, Map as _, MapRef, ReadTxn, Transact as _, types::ToJson as _};

use super::{Detail, kanban};
use crate::{
    domain::{
        board::{Object, ObjectKind, json, text_of},
        change::value::{Made, StoreEntry},
        geometry::{Point, Size},
        time::Timestamp,
    },
    protocol::document::{
        COLOR,
        DOCUMENTS,
        INDEX,
        TAG_IDS,
        TASK_INFO_STORE,
        TITLE,
        WIDTH,
    },
};

pub(crate) const TITLE_DOCUMENT: &str = "titleDocumentId";
pub(crate) const DESCRIPTION_DOCUMENT: &str = "descriptionDocumentId";
pub(crate) const INFO_ID: &str = "taskCardInfoId";
pub(crate) const STATUS_ID: &str = "statusId";
pub(crate) const SCHEDULE: &str = "schedule";
pub(crate) const COLUMN_ID: &str = "columnId";
pub(crate) const SWIMLANE_ID: &str = "swimlaneId";
pub(crate) const DISPLAY_OPTIONS: &str = "displayOptions";
pub(crate) const ASSIGNEE_IDS: &str = "assigneeIds";
pub(crate) const STATUS_STORE: &str = "task-card-statusesStore";

/// A free card is this wide in the web client (main 207756).
const WIDE: f64 = 300.0;
const HIGH: f64 = 120.0;
/// The `index` of a first card, `zH(null, null)` without its random part.
const FIRST_INDEX: u64 = 0x100_0000_0000;
/// What `zH(last, null)` adds to the `index` of the last card.
const INDEX_STEP: f64 = 1_099_511_627_776.0;

/// A kanban cell by id, and the `index` of the last card in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub kanban: String,
    pub column: String,
    pub swimlane: String,
    pub after: Option<f64>,
}

impl Cell {
    /// The `index` of a card put last in the cell.
    #[must_use]
    pub fn index(&self) -> f64 {
        self.after.unwrap_or_default() + INDEX_STEP
    }
}

/// The built-in statuses, each with the title the web client shows.
pub const STATUSES: [(&str, &str); 5] = [
    ("backlog", "Backlog"),
    ("to-do", "To do"),
    ("in-progress", "In progress"),
    ("done", "Done"),
    ("canceled", "Canceled"),
];

/// The colors of the card's palette (main 53334), by name; gray is its default.
pub const COLORS: [(&str, u32); 10] = [
    ("gray", DEFAULT_COLOR),
    ("green", 0x6f_cd98),
    ("yellow", 0xed_cc4d),
    ("orange", 0xf1_a365),
    ("red", 0xe8_726a),
    ("violet", 0x9f_91ee),
    ("blue", 0x6c_9ffd),
    ("sky", 0x82_c3df),
    ("lime", 0x9c_c54c),
    ("pink", 0xda_77bb),
];

const DEFAULT_COLOR: u32 = 0xb3_b4b4;

/// Whether a card of this color is filled with it, as the card's own reader
/// decides when it stores no choice.
#[must_use]
pub fn fills(rgb: u32) -> bool {
    rgb != DEFAULT_COLOR
}

/// The number the card stores for the palette color `name`.
#[must_use]
pub fn tint(name: &str) -> Option<u32> {
    COLORS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, rgb)| *rgb)
}

/// A calendar day, `YYYY-MM-DD` from 1970 on; days order by date.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day(String);

impl Day {
    /// The day as the card stores it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What is not a day.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("a day as YYYY-MM-DD, e.g. 2026-11-06")]
pub struct NotDay;

impl FromStr for Day {
    type Err = NotDay;

    fn from_str(raw: &str) -> Result<Self, NotDay> {
        Timestamp::parse_iso8601(&format!("{raw}T00:00:00Z"))
            .map(|_| Self(raw.to_owned()))
            .ok_or(NotDay)
    }
}

/// A new task of `title` as the web client's toolbar tool places it, with its
/// info entry `info` and the built-in statuses the board may lack; the corner
/// is the caller's to set.
#[must_use]
pub fn made(
    title: &str,
    description: Option<&str>,
    (start, due): (Option<&Day>, Option<&Day>),
    color: Option<u32>,
    info: String,
) -> Made {
    let ends: Map<String, Value> = [("start", start), ("end", due)]
        .into_iter()
        .filter_map(|(end, day)| {
            day.map(|day| (end.to_owned(), json!({ "date": day.as_str() })))
        })
        .collect();
    let filled = color.is_some_and(fills);
    Made {
        kind: ObjectKind::Task,
        at: Point::ORIGIN,
        size: Size::new(WIDE, HIGH).ok(),
        parent: None,
        fields: [
            (WIDTH, json!(WIDE)),
            (INDEX, json!(FIRST_INDEX)),
            (INFO_ID, json!(info)),
            (
                DISPLAY_OPTIONS,
                json!({ "statusLikeCheckbox": false, "fillBackground": filled }),
            ),
        ]
        .into_iter()
        .chain(color.map(|rgb| (COLOR, json!(rgb))))
        .chain((!ends.is_empty()).then_some((SCHEDULE, Value::Object(ends))))
        .collect(),
        texts: [
            (TITLE_DOCUMENT, Some(title)),
            (DESCRIPTION_DOCUMENT, description),
        ]
        .into_iter()
        .filter_map(|(field, text)| text.map(|text| (field, text.to_owned())))
        .collect(),
        code: None,
        parts: Vec::new(),
        entries: std::iter::once(StoreEntry {
            store: TASK_INFO_STORE,
            id: info,
            fields: Vec::new(),
            lists: vec![(TAG_IDS, Vec::new()), (ASSIGNEE_IDS, Vec::new())],
            keep: false,
        })
        .chain(status_entries())
        .collect(),
    }
}

/// The built-in statuses as store entries, each written only when absent.
pub fn status_entries() -> impl Iterator<Item = StoreEntry> {
    STATUSES.iter().map(|(id, name)| StoreEntry {
        store: STATUS_STORE,
        id: (*id).to_owned(),
        fields: vec![(TITLE, json!(name))],
        lists: Vec::new(),
        keep: true,
    })
}

/// `made` put last in the kanban cell `cell`.
#[must_use]
pub fn in_cell(made: Made, cell: &Cell) -> Made {
    Made {
        parent: Some(cell.kanban.clone()),
        fields: made
            .fields
            .into_iter()
            .filter(|(name, _)| ![INDEX, WIDTH].contains(name))
            .chain([
                (INDEX, json!(cell.index())),
                (WIDTH, json!(kanban::CARD_WIDTH)),
                (SWIMLANE_ID, json!(cell.swimlane)),
                (COLUMN_ID, json!(cell.column)),
            ])
            .collect(),
        ..made
    }
}

/// `made` with the status `id`.
#[must_use]
pub fn with_status(made: Made, id: &str) -> Made {
    Made {
        fields: made
            .fields
            .into_iter()
            .chain([(STATUS_ID, json!(id))])
            .collect(),
        ..made
    }
}

/// The id and title of each status the board stores.
#[must_use]
pub fn statuses(document: &Doc) -> Vec<(String, String)> {
    let store = document.get_or_insert_map(STATUS_STORE);
    statuses_in(&document.transact(), &store)
}

/// [`statuses`] read through a transaction already open.
#[must_use]
pub fn statuses_in<T: ReadTxn>(txn: &T, store: &MapRef) -> Vec<(String, String)> {
    store
        .iter(txn)
        .filter_map(|(id, entry)| {
            let entry = json(&entry.to_json(txn))?;
            let title = entry.get(TITLE)?.as_str()?.to_owned();
            Some((id.to_owned(), title))
        })
        .collect()
}

/// The id of the status `given` names: a built-in or stored id, else a title
/// of either, ignoring case.
#[must_use]
pub fn status_id(given: &str, custom: &[(String, String)]) -> Option<String> {
    let known = || {
        STATUSES.iter().map(|(id, title)| (*id, *title)).chain(
            custom
                .iter()
                .map(|(id, title)| (id.as_str(), title.as_str())),
        )
    };
    known()
        .find(|(id, _)| *id == given)
        .or_else(|| known().find(|(_, title)| title.eq_ignore_ascii_case(given)))
        .map(|(id, _)| id.to_owned())
}

/// The title as the text; the status, the start and due days and the number
/// of assignees as words.
pub(super) fn detail(document: &Doc, card: &Object) -> Detail {
    let cell = kanban::cell_words(document, card);
    let documents = document.get_or_insert_map(DOCUMENTS);
    let infos = document.get_or_insert_map(TASK_INFO_STORE);
    let txn = document.transact();
    let text = |field: &str| card.fields.get(field).and_then(Value::as_str);
    let day = |end: &str| {
        card.fields
            .pointer(&format!("/{SCHEDULE}/{end}/date"))
            .cloned()
    };
    let assignees = text(INFO_ID)
        .and_then(|id| infos.get(&txn, id))
        .and_then(|info| json(&info.to_json(&txn)))
        .and_then(|info| {
            info.get(ASSIGNEE_IDS)?
                .as_array()
                .map(|list| json!(list.len()))
        });
    Detail {
        text: text(TITLE_DOCUMENT).map(|id| text_of(&txn, &documents, id)),
        words: cell
            .into_iter()
            .chain([
                (
                    "status",
                    card.fields
                        .get(STATUS_ID)
                        .filter(|id| !id.is_null())
                        .cloned(),
                ),
                ("start", day("start")),
                ("due", day("end")),
                ("assignees", assignees),
            ])
            .filter_map(|(word, value)| value.map(|value| (word, value)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use rstest::rstest;
    use serde_json::json;
    use yrs::{
        Any,
        ArrayPrelim,
        Doc,
        Map as _,
        MapPrelim,
        Transact as _,
        XmlTextPrelim,
    };

    use super::{Cell, Day, STATUSES, in_cell, made, status_id, tint, with_status};
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            change::value::Made,
            geometry::Size,
            kinds::detail,
        },
        protocol::document::{DOCUMENTS, TASK_INFO_STORE},
    };

    fn day(raw: &str) -> Option<Day> {
        raw.parse().ok()
    }

    #[test]
    fn a_made_task_has_the_keys_texts_and_stores_of_the_web_client() -> Result<()> {
        let schedule = (day("2026-11-02"), day("2026-11-06"));
        let full = made(
            "Ship",
            Some("Go live"),
            (schedule.0.as_ref(), schedule.1.as_ref()),
            tint("yellow"),
            "i1".to_owned(),
        );
        let plain = made("Ship", None, (None, None), None, "i2".to_owned());
        assert_eq!(
            (full.kind.clone(), full.size, full.fields, full.texts),
            (
                ObjectKind::Task,
                Some(Size::new(300.0, 120.0)?),
                vec![
                    ("width", json!(300.0)),
                    ("index", json!(0x100_0000_0000_u64)),
                    ("taskCardInfoId", json!("i1")),
                    (
                        "displayOptions",
                        json!({"statusLikeCheckbox": false, "fillBackground": true})
                    ),
                    ("color", json!(0xed_cc4d)),
                    (
                        "schedule",
                        json!({"start": {"date": "2026-11-02"}, "end": {"date": "2026-11-06"}})
                    ),
                ],
                vec![
                    ("titleDocumentId", "Ship".to_owned()),
                    ("descriptionDocumentId", "Go live".to_owned())
                ]
            )
        );
        assert_eq!(
            (plain.fields.len(), plain.texts.len()),
            (4, 1),
            "no color, schedule or description"
        );
        Ok(())
    }

    #[test]
    fn the_info_entry_is_new_and_the_five_statuses_are_kept() {
        let task = made("Ship", None, (None, None), None, "i1".to_owned());
        let entries: Vec<_> = task
            .entries
            .iter()
            .map(|entry| (entry.store, entry.id.as_str(), entry.keep))
            .collect();
        assert_eq!(
            (entries.first().copied(), entries.get(1..)),
            (
                Some(("task-card-infoStore", "i1", false)),
                Some(
                    &[
                        ("task-card-statusesStore", "backlog", true),
                        ("task-card-statusesStore", "to-do", true),
                        ("task-card-statusesStore", "in-progress", true),
                        ("task-card-statusesStore", "done", true),
                        ("task-card-statusesStore", "canceled", true),
                    ][..]
                )
            )
        );
        assert_eq!(
            task.entries.first().map(|entry| entry.lists.clone()),
            Some(vec![("tagIds", vec![]), ("assigneeIds", vec![])])
        );
        assert_eq!(STATUSES.len(), 5);
    }

    #[rstest]
    #[case::plain("2026-11-06", true)]
    #[case::leap_day("2028-02-29", true)]
    #[case::month_13("2026-13-01", false)]
    #[case::no_such_day("2026-02-30", false)]
    #[case::no_zero("2026-1-6", false)]
    #[case::word("tomorrow", false)]
    #[case::with_time("2026-11-06T10:00:00Z", false)]
    #[case::empty("", false)]
    fn a_day_is_a_real_date_as_yyyy_mm_dd(#[case] raw: &str, #[case] real: bool) {
        assert_eq!(raw.parse::<Day>().is_ok(), real);
    }

    #[test]
    fn a_refused_day_says_what_it_expects_and_days_order_by_date() {
        assert_eq!(
            "soon".parse::<Day>().err().map(|error| error.to_string()),
            Some("a day as YYYY-MM-DD, e.g. 2026-11-06".to_owned())
        );
        assert!(day("2026-11-02") < day("2026-11-06"));
    }

    #[test]
    fn a_status_is_named_by_id_or_by_title_among_the_built_in_and_the_boards() {
        let custom = [("s9".to_owned(), "In review".to_owned())];
        assert_eq!(
            ["in-progress", "To do", "in review", "s9", "later"]
                .map(|given| status_id(given, &custom)),
            [
                Some("in-progress".to_owned()),
                Some("to-do".to_owned()),
                Some("s9".to_owned()),
                Some("s9".to_owned()),
                None
            ]
        );
    }

    #[test]
    fn a_task_takes_the_status_asked_for() {
        let task = made("Ship", None, (None, None), None, "i1".to_owned());
        assert_eq!(
            with_status(task, "done").fields.last(),
            Some(&("statusId", json!("done")))
        );
    }

    #[test]
    fn a_task_lists_its_title_status_dates_and_assignees() {
        let document = Doc::new();
        let documents = document.get_or_insert_map(DOCUMENTS);
        let infos = document.get_or_insert_map(TASK_INFO_STORE);
        {
            // imperative: yrs inserts through one `&mut` transaction
            let mut transaction = document.transact_mut();
            documents.insert(&mut transaction, "t1", XmlTextPrelim::new("Ship"));
            let info = infos.insert(
                &mut transaction,
                "i1",
                MapPrelim::from_iter([("tagIds", Any::from(0))]),
            );
            info.insert(
                &mut transaction,
                "assigneeIds",
                ArrayPrelim::from_iter([Any::from("u1"), Any::from("u2")]),
            );
        }
        let card = object_of(json!({
            "id": "k1", "type": "task-card", "position": {"x": 0, "y": 0},
            "titleDocumentId": "t1", "taskCardInfoId": "i1", "statusId": "to-do",
            "schedule": {"start": {"date": "2026-11-02"}, "end": {"date": "2026-11-06"}}
        }));
        let shown = card.iter().map(|object| detail(&document, object));
        assert_eq!(
            shown
                .map(|found| (found.text, found.words))
                .collect::<Vec<_>>(),
            vec![(
                Some("Ship".to_owned()),
                vec![
                    ("status", json!("to-do")),
                    ("start", json!("2026-11-02")),
                    ("due", json!("2026-11-06")),
                    ("assignees", json!(2))
                ]
            )]
        );
    }

    #[test]
    fn a_task_without_dates_or_info_lists_its_status_alone() {
        let card = object_of(json!({
            "id": "k1", "type": "task-card", "position": {"x": 0, "y": 0},
            "statusId": "done"
        }));
        let words = card
            .iter()
            .map(|object| detail(&Doc::new(), object).words)
            .collect::<Vec<_>>();
        assert_eq!(words, vec![vec![("status", json!("done"))]]);
    }

    #[test]
    fn a_task_whose_status_is_cleared_lists_no_status() {
        let card = object_of(json!({
            "id": "k1", "type": "task-card", "position": {"x": 0, "y": 0},
            "statusId": null
        }));
        let words = card
            .iter()
            .map(|object| detail(&Doc::new(), object).words)
            .collect::<Vec<_>>();
        assert_eq!(words, vec![Vec::new()]);
    }

    fn cell(after: Option<f64>) -> Cell {
        Cell {
            kanban: "k1".to_owned(),
            column: "c1".to_owned(),
            swimlane: "l1".to_owned(),
            after,
        }
    }

    #[test]
    fn a_task_in_a_cell_is_held_by_its_kanban_and_placed_after_the_last_card() {
        let plain = made("Ship", None, (None, None), None, "i1".to_owned());
        let first = in_cell(plain.clone(), &cell(None));
        let second = in_cell(plain, &cell(Some(1_099_511_627_776.0)));
        let field = |card: &Made, name: &str| {
            card.fields
                .iter()
                .find(|(field, _)| *field == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            [&first, &second].map(|card| (
                card.parent.clone(),
                (card.at.x(), card.at.y()),
                field(card, "columnId"),
                field(card, "swimlaneId"),
                field(card, "index"),
                field(card, "width"),
                card.fields
                    .iter()
                    .filter(|(name, _)| ["index", "width"].contains(name))
                    .count(),
            )),
            [
                (
                    Some("k1".to_owned()),
                    (0.0, 0.0),
                    Some(json!("c1")),
                    Some(json!("l1")),
                    Some(json!(1_099_511_627_776.0)),
                    Some(json!(338.0)),
                    2
                ),
                (
                    Some("k1".to_owned()),
                    (0.0, 0.0),
                    Some(json!("c1")),
                    Some(json!("l1")),
                    Some(json!(2_199_023_255_552.0)),
                    Some(json!(338.0)),
                    2
                ),
            ]
        );
    }

    #[test]
    fn a_task_in_a_cell_lists_its_column_and_swimlane_first() {
        let document = Doc::new();
        let columns = document.get_or_insert_map("kanban-columnsStore");
        let lanes = document.get_or_insert_map("kanban-swimlanesStore");
        {
            // imperative: yrs inserts through one `&mut` transaction
            let mut transaction = document.transact_mut();
            columns.insert(
                &mut transaction,
                "c1",
                MapPrelim::from_iter([("title", Any::from("To do"))]),
            );
            lanes.insert(
                &mut transaction,
                "l1",
                MapPrelim::from_iter([("title", Any::from("Untitled"))]),
            );
        }
        let card = object_of(json!({
            "id": "k2", "type": "task-card", "position": {"x": 0, "y": 0},
            "columnId": "c1", "swimlaneId": "l1", "statusId": "done"
        }));
        let words = card
            .iter()
            .map(|object| detail(&document, object).words)
            .collect::<Vec<_>>();
        assert_eq!(
            words,
            vec![vec![
                ("column", json!("To do")),
                ("swimlane", json!("Untitled")),
                ("status", json!("done"))
            ]]
        );
    }
}
