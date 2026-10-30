//! A task card: its status, days and description.

use std::collections::HashMap;

use serde_json::{Map, Value, json};
use yrs::{
    Array as _,
    ArrayPrelim,
    ArrayRef,
    Map as _,
    MapRef,
    ReadTxn as _,
    TransactionMut,
    WriteTxn as _,
};

use super::Written;
use crate::{
    domain::{
        board::ObjectKind,
        change::{
            made,
            write::{WriteTransaction, write_text},
        },
        kinds::task::{
            self,
            ASSIGNEE_IDS,
            DESCRIPTION_DOCUMENT,
            DISPLAY_OPTIONS,
            Day,
            INFO_ID,
            SCHEDULE,
            STATUS_ID,
            STATUS_STORE,
        },
        rich_text::TextFormat,
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{COLOR, TASK_INFO_STORE},
};

const STATUS: &str = "status";
const START: &str = "start";
const DUE: &str = "due";
const DESCRIPTION: &str = "description";
const ASSIGNEES: &str = "assignees";

/// What an edit does to one word of a card: sets it, or takes it away.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting<T> {
    To(T),
    Clear,
}

/// The words of a task card an edit changes; a word left out stays.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TaskEdit {
    /// The status as given: an id or a title.
    pub status: Option<Setting<String>>,
    pub start: Option<Setting<Day>>,
    pub due: Option<Setting<Day>>,
    pub description: Option<Setting<String>>,
    /// The format the description is written in.
    pub format: TextFormat,
    /// The user ids the card is assigned to, already resolved; empty clears.
    pub assignees: Option<Vec<String>>,
    /// The number the card stores for a color of its palette.
    pub color: Option<u32>,
}

impl TaskEdit {
    /// The first word the edit changes.
    pub(super) fn field(&self) -> &'static str {
        [
            (STATUS, self.status.is_some()),
            (START, self.start.is_some()),
            (DUE, self.due.is_some()),
            (DESCRIPTION, self.description.is_some()),
            (ASSIGNEES, self.assignees.is_some()),
            (COLOR, self.color.is_some()),
        ]
        .into_iter()
        .find_map(|(name, given)| given.then_some(name))
        .unwrap_or(STATUS)
    }

    fn schedule_given(&self) -> bool {
        self.start.is_some() || self.due.is_some()
    }

    /// The start and due day the card has after the edit.
    fn days(&self, fields: Option<&Value>) -> (Option<Day>, Option<Day>) {
        let current = |end: &str| {
            fields
                .and_then(|fields| fields.pointer(&format!("/{SCHEDULE}/{end}/date")))
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<Day>().ok())
        };
        let settled = |setting: Option<&Setting<Day>>, end: &str| match setting {
            Some(Setting::To(day)) => Some(day.clone()),
            Some(Setting::Clear) => None,
            None => current(end),
        };
        (
            settled(self.start.as_ref(), "start"),
            settled(self.due.as_ref(), "end"),
        )
    }
}

fn not_changeable(kind: &ObjectKind, field: &'static str) -> Error {
    Error::BoardChange(BoardChangeError::NotChangeable {
        kind: kind.as_str().to_owned(),
        field,
    })
}

pub(super) fn check(
    edit: &TaskEdit,
    kind: &ObjectKind,
    states: &HashMap<String, Value>,
    id: &str,
) -> Result<()> {
    (*kind == ObjectKind::Task)
        .then_some(())
        .ok_or_else(|| not_changeable(kind, edit.field()))?;
    let (start, due) = edit.days(states.get(id));
    start
        .zip(due)
        .filter(|(start, due)| edit.schedule_given() && start > due)
        .map_or(Ok(()), |(_, due)| {
            Err(Error::BoardChange(BoardChangeError::BadValue {
                field: "due day",
                raw: due.as_str().to_owned(),
                expected: "on or after the start day",
            }))
        })
}

/// The status asked for among the built-in ones and the board's, the built-in
/// ones stored first; none gives `null`.
fn status_field(
    setting: &Setting<String>,
    transaction: &mut TransactionMut<'_>,
) -> Result<Value> {
    match setting {
        Setting::Clear => Ok(Value::Null),
        Setting::To(given) => {
            let store = transaction.get_or_insert_map(STATUS_STORE);
            let id = task::status_id(given, &task::statuses_in(transaction, &store))
                .ok_or_else(|| {
                    Error::BoardChange(BoardChangeError::BadValue {
                        field: "task status",
                        raw: given.clone(),
                        expected: "backlog, to-do, in-progress, done, canceled, or the title of a status of the board",
                    })
                })?;
            task::status_entries()
                .try_for_each(|entry| made::store(transaction, &entry))?;
            Ok(json!(id))
        }
    }
}

/// The schedule with the ends the days leave; none left gives `null`.
fn schedule_field(start: Option<&Day>, due: Option<&Day>) -> Value {
    let ends: Map<String, Value> = [("start", start), ("end", due)]
        .into_iter()
        .filter_map(|(end, day)| {
            day.map(|day| (end.to_owned(), json!({ "date": day.as_str() })))
        })
        .collect();
    Some(ends)
        .filter(|ends| !ends.is_empty())
        .map_or(Value::Null, Value::Object)
}

/// The card's `displayOptions` with `fillBackground` set for the new color,
/// its other options kept.
fn display_options(fields: &Value, rgb: u32) -> Value {
    let kept = fields
        .get(DISPLAY_OPTIONS)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Value::Object(
        kept.into_iter()
            .chain([("fillBackground".to_owned(), json!(task::fills(rgb)))])
            .collect(),
    )
}

/// The description rewritten, in a new document when the card has none; taking
/// it away empties a document there is and makes none.
fn description_fields(
    setting: &Setting<String>,
    (transaction, scope): (&mut TransactionMut<'_>, &WriteTransaction<'_>),
    fields: &Value,
    format: TextFormat,
) -> Vec<(&'static str, Value)> {
    let held = fields
        .get(DESCRIPTION_DOCUMENT)
        .and_then(Value::as_str)
        .is_some();
    let text = match (setting, held) {
        (Setting::To(text), _) => Some(text.as_str()),
        (Setting::Clear, true) => Some(""),
        (Setting::Clear, false) => None,
    };
    text.map(|text| {
        write_text(
            transaction,
            scope,
            fields,
            DESCRIPTION_DOCUMENT,
            (text, format, &ObjectKind::Task),
        )
    })
    .unwrap_or_default()
}

/// The assignee ids of a task card, which its info entry holds in a list of
/// its own, replaced in place (order kept, duplicates dropped) so the entry's
/// other lists stay.
fn card_assignees(
    transaction: &mut TransactionMut<'_>,
    fields: &Value,
    ids: &[String],
) -> Result<()> {
    let info = fields
        .get(INFO_ID)
        .and_then(Value::as_str)
        .and_then(|info| {
            transaction
                .get_map(TASK_INFO_STORE)?
                .get(&*transaction, info)?
                .cast::<MapRef>()
                .ok()
        })
        .ok_or_else(|| not_changeable(&ObjectKind::Task, ASSIGNEES))?;
    let list = info
        .get(&*transaction, ASSIGNEE_IDS)
        .and_then(|held| held.cast::<ArrayRef>().ok())
        .unwrap_or_else(|| {
            info.insert(transaction, ASSIGNEE_IDS, ArrayPrelim::default())
        });
    let length = list.len(&*transaction);
    list.remove_range(transaction, 0, length);
    let mut seen = std::collections::HashSet::new();
    list.insert_range(
        transaction,
        0,
        ids.iter().filter(|id| seen.insert(id.as_str())).cloned(),
    );
    Ok(())
}

pub(super) fn write(
    edit: &TaskEdit,
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    fields: &Value,
) -> Result<Written> {
    let status = edit
        .status
        .as_ref()
        .map(|setting| status_field(setting, transaction).map(|id| (STATUS_ID, id)))
        .transpose()?;
    let schedule = edit.schedule_given().then(|| {
        let (start, due) = edit.days(Some(fields));
        (SCHEDULE, schedule_field(start.as_ref(), due.as_ref()))
    });
    let description = edit.description.as_ref().map(|setting| {
        description_fields(setting, (transaction, scope), fields, edit.format)
    });
    edit.assignees
        .as_deref()
        .map_or(Ok(()), |ids| card_assignees(transaction, fields, ids))?;
    let color = edit.color.into_iter().flat_map(|rgb| {
        [
            (COLOR, json!(rgb)),
            (DISPLAY_OPTIONS, display_options(fields, rgb)),
        ]
    });
    Ok(Written {
        fields: status
            .into_iter()
            .chain(schedule)
            .chain(color)
            .chain(description.into_iter().flatten())
            .collect(),
        touched: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use super::{Setting, TaskEdit};
    use crate::{
        domain::{
            board::{self, text_of},
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
            },
            geometry::Point,
            id_source::SequentialIds,
            kinds::task::{self, Day},
            rich_text::TextFormat,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    fn board_with(new: NewObject) -> Result<Doc> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new,
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn day(raw: &str) -> Option<Day> {
        raw.parse().ok()
    }

    /// A task on a board that stores no statuses yet.
    fn a_task(
        (start, due): (Option<&str>, Option<&str>),
        description: Option<&str>,
    ) -> Result<Doc> {
        let mut made = task::made(
            "Ship",
            description,
            (start.and_then(day).as_ref(), due.and_then(day).as_ref()),
            None,
            "i1".to_owned(),
        );
        made.entries.truncate(1);
        made.at = Point::new(10.0, 20.0)?;
        board_with(NewObject::Made(made))
    }

    fn card_id(document: &Doc) -> String {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.keys(&transaction).next().map(str::to_owned))
            .unwrap_or_default()
    }

    fn edit(document: &Doc, task_edit: TaskEdit) -> Result<()> {
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: card_id(document),
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Task(task_edit)],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new("r"), &change).map(drop)
    }

    fn stored(document: &Doc) -> Value {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, &card_id(document)))
            .and_then(|value| board::json(&value.to_json(&transaction)))
            .unwrap_or_default()
    }

    fn field(document: &Doc, name: &str) -> Value {
        stored(document).get(name).cloned().unwrap_or_default()
    }

    fn status_ids(document: &Doc) -> Vec<String> {
        let transaction = document.transact();
        let mut ids: Vec<String> = transaction
            .get_map("task-card-statusesStore")
            .map(|store| store.keys(&transaction).map(str::to_owned).collect())
            .unwrap_or_default();
        ids.sort();
        ids
    }

    fn description(document: &Doc) -> Option<String> {
        let id = field(document, "descriptionDocumentId");
        let transaction = document.transact();
        let documents = transaction.get_map("documents")?;
        id.as_str().map(|id| text_of(&transaction, &documents, id))
    }

    fn status(raw: &str) -> TaskEdit {
        TaskEdit {
            status: Some(Setting::To(raw.to_owned())),
            ..TaskEdit::default()
        }
    }

    #[test]
    fn a_status_is_written_and_the_five_statuses_are_ensured_once() -> Result<()> {
        let document = a_task((None, None), None)?;
        edit(&document, status("done"))?;
        let first = (field(&document, "statusId"), status_ids(&document));
        edit(&document, status("Canceled"))?;
        assert_eq!(
            (
                first,
                field(&document, "statusId"),
                status_ids(&document).len()
            ),
            (
                (
                    json!("done"),
                    ["backlog", "canceled", "done", "in-progress", "to-do"]
                        .map(str::to_owned)
                        .to_vec()
                ),
                json!("canceled"),
                5
            )
        );
        Ok(())
    }

    #[test]
    fn a_status_the_board_lacks_is_refused_and_none_takes_it_away() -> Result<()> {
        let document = a_task((None, None), None)?;
        edit(&document, status("done"))?;
        let refused = edit(&document, status("later")).err();
        edit(
            &document,
            TaskEdit {
                status: Some(Setting::Clear),
                ..TaskEdit::default()
            },
        )?;
        assert_eq!(
            refused.map(|error| error.to_string()),
            Some("\"later\" is not a valid task status: expected backlog, to-do, in-progress, done, canceled, or the title of a status of the board".to_owned())
        );
        assert_eq!(field(&document, "statusId"), Value::Null);
        Ok(())
    }

    #[test]
    fn a_due_day_before_the_start_is_refused_and_the_card_stays() -> Result<()> {
        let document = a_task((Some("2026-11-10"), None), None)?;
        let refused = edit(
            &document,
            TaskEdit {
                due: day("2026-11-01").map(Setting::To),
                ..TaskEdit::default()
            },
        )
        .err();
        assert_eq!(
            refused.map(|error| error.to_string()),
            Some("\"2026-11-01\" is not a valid due day: expected on or after the start day".to_owned())
        );
        assert_eq!(
            field(&document, "schedule"),
            json!({"start": {"date": "2026-11-10"}})
        );
        Ok(())
    }

    #[test]
    fn due_none_clears_only_the_end_and_a_later_day_replaces_it() -> Result<()> {
        let document = a_task((Some("2026-11-02"), Some("2026-11-06")), None)?;
        edit(
            &document,
            TaskEdit {
                due: Some(Setting::Clear),
                ..TaskEdit::default()
            },
        )?;
        let cleared = field(&document, "schedule");
        edit(
            &document,
            TaskEdit {
                due: day("2026-12-01").map(Setting::To),
                ..TaskEdit::default()
            },
        )?;
        assert_eq!(
            (cleared, field(&document, "schedule")),
            (
                json!({"start": {"date": "2026-11-02"}}),
                json!({"start": {"date": "2026-11-02"}, "end": {"date": "2026-12-01"}})
            )
        );
        Ok(())
    }

    #[test]
    fn the_description_is_made_on_first_use_and_rewritten_after() -> Result<()> {
        let document = a_task((None, None), None)?;
        let before = description(&document);
        let write = |text: &str| TaskEdit {
            description: Some(Setting::To(text.to_owned())),
            ..TaskEdit::default()
        };
        edit(&document, write("Go live"))?;
        let made = (
            description(&document),
            field(&document, "descriptionDocumentId"),
        );
        edit(&document, write("Go later"))?;
        assert_eq!(
            (
                before,
                made.0,
                description(&document),
                field(&document, "descriptionDocumentId") == made.1
            ),
            (
                None,
                Some("Go live".to_owned()),
                Some("Go later".to_owned()),
                true
            )
        );
        Ok(())
    }

    #[test]
    fn description_none_empties_the_text_and_makes_none_when_there_is_none()
    -> Result<()> {
        let bare = a_task((None, None), None)?;
        let described = a_task((None, None), Some("Go live"))?;
        let clear = || TaskEdit {
            description: Some(Setting::Clear),
            ..TaskEdit::default()
        };
        edit(&bare, clear())?;
        edit(&described, clear())?;
        assert_eq!(
            (description(&bare), description(&described)),
            (None, Some(String::new()))
        );
        Ok(())
    }

    fn assignees(document: &Doc) -> Value {
        let info = field(document, "taskCardInfoId");
        let transaction = document.transact();
        transaction
            .get_map("task-card-infoStore")
            .and_then(|store| {
                store.get(&transaction, info.as_str().unwrap_or_default())
            })
            .and_then(|value| board::json(&value.to_json(&transaction)))
            .and_then(|info| info.get("assigneeIds").cloned())
            .unwrap_or_default()
    }

    fn assign(ids: &[&str]) -> TaskEdit {
        TaskEdit {
            assignees: Some(ids.iter().map(|id| (*id).to_owned()).collect()),
            ..TaskEdit::default()
        }
    }

    #[test]
    fn assignees_are_written_replaced_deduplicated_and_cleared() -> Result<()> {
        let document = a_task((None, None), None)?;
        edit(&document, assign(&["u1", "u2", "u1"]))?;
        let first = assignees(&document);
        edit(&document, assign(&["u3"]))?;
        let second = assignees(&document);
        edit(&document, assign(&[]))?;
        assert_eq!(
            (first, second, assignees(&document)),
            (json!(["u1", "u2"]), json!(["u3"]), json!([]))
        );
        Ok(())
    }

    #[test]
    fn a_sticker_takes_no_assignees() -> Result<()> {
        let document = board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })?;
        let refused = edit(&document, assign(&["u1"])).err();
        assert!(matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "assignees",
                ..
            }))
        ));
        Ok(())
    }

    #[test]
    fn a_sticker_takes_none_of_it() -> Result<()> {
        let document = board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })?;
        let refused = edit(&document, status("done")).err();
        assert!(matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "status",
                ..
            }))
        ));
        Ok(())
    }
}
