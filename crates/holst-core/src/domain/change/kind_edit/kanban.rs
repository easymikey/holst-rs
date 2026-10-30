//! A task card moved between the cells of a kanban, into one or out of it.

use std::collections::HashMap;

use serde_json::{Value, json};
use yrs::TransactionMut;

use super::{Written, only};
use crate::{
    domain::{
        board::ObjectKind,
        change::{
            check::{find, kind_of, parent_of},
            write::WriteTransaction,
        },
        kinds::{
            kanban::{self, CARD_WIDTH, COLUMN_WIDTH, columns_in, swimlanes_in},
            task::{COLUMN_ID, Cell, SWIMLANE_ID},
        },
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{INDEX, PARENT_ID, POSITION, SCALE, WIDTH, X, Y},
};

pub(super) const KANBAN: &str = "kanban";
const COLUMN: &str = "column";
const SWIMLANE: &str = "swimlane";

/// Where an edit puts a task card.
#[derive(Debug, Clone, PartialEq)]
pub enum CardMove {
    /// Out of its kanban, onto the board where it is drawn.
    Out,
    /// Into a cell; what is left out is the card's own, else the first.
    Into {
        kanban: Option<String>,
        column: Option<String>,
        swimlane: Option<String>,
    },
}

impl CardMove {
    /// The first word the move says.
    pub(super) fn field(&self) -> &'static str {
        match self {
            Self::Out
            | Self::Into {
                kanban: Some(_), ..
            } => KANBAN,
            Self::Into {
                column: Some(_), ..
            } => COLUMN,
            Self::Into { .. } => SWIMLANE,
        }
    }

    /// The first value the move gives, for a refusal.
    fn raw(&self) -> String {
        match self {
            Self::Out => "none".to_owned(),
            Self::Into {
                kanban,
                column,
                swimlane,
            } => kanban
                .as_ref()
                .or(column.as_ref())
                .or(swimlane.as_ref())
                .cloned()
                .unwrap_or_default(),
        }
    }
}

fn refused(field: &'static str, raw: String, expected: &'static str) -> Error {
    Error::BoardChange(BoardChangeError::BadValue {
        field,
        raw,
        expected,
    })
}

/// The id of `holder` when it is a kanban.
fn kanban_in<'a>(
    states: &HashMap<String, Value>,
    holder: Option<&'a str>,
) -> Option<&'a str> {
    holder.filter(|up| kind_of(states, up).as_str() == kanban::STORED)
}

/// The id of the kanban the card is in.
fn held_by<'a>(states: &HashMap<String, Value>, card: &'a Value) -> Option<&'a str> {
    kanban_in(states, parent_of(card))
}

/// The id of the kanban `reference` names.
fn named(states: &HashMap<String, Value>, reference: &str) -> Result<String> {
    let id = find(states, reference)?;
    kanban_in(states, Some(&id))
        .map(str::to_owned)
        .ok_or_else(|| refused(KANBAN, reference.to_owned(), "a kanban"))
}

pub(super) fn check(
    edit: &CardMove,
    kind: &ObjectKind,
    states: &HashMap<String, Value>,
    id: &str,
) -> Result<()> {
    only(kind, &ObjectKind::Task, edit.field())?;
    let card = states.get(id).unwrap_or(&Value::Null);
    let inside = held_by(states, card)
        .map(|_| ())
        .ok_or_else(|| refused(edit.field(), edit.raw(), "a card that is in a kanban"));
    match edit {
        CardMove::Into {
            kanban: Some(reference),
            ..
        } => named(states, reference).map(drop),
        CardMove::Out | CardMove::Into { kanban: None, .. } => inside,
    }
}

/// Cell `given` titles (ignoring case), else `kept`, else the first.
fn picked(
    (name, expected): (&'static str, &'static str),
    given: Option<&str>,
    (kept, cells): (Option<&str>, &[(String, String)]),
) -> Result<String> {
    let found = given.map_or_else(
        || {
            cells
                .iter()
                .find(|(id, _)| Some(id.as_str()) == kept)
                .or_else(|| cells.first())
        },
        |title| {
            cells
                .iter()
                .find(|(_, known)| known.to_lowercase() == title.to_lowercase())
        },
    );
    found
        .map(|(id, _)| id.clone())
        .ok_or_else(|| refused(name, given.unwrap_or_default().to_owned(), expected))
}

/// Card put last in `kanban`'s cell: holder, place, card scale and width.
fn into_fields(
    transaction: &TransactionMut<'_>,
    states: &HashMap<String, Value>,
    (id, card): (&str, &Value),
    (edit_kanban, column, swimlane): (
        &Option<String>,
        &Option<String>,
        &Option<String>,
    ),
) -> Result<Vec<(&'static str, Value)>> {
    let kanban = match edit_kanban {
        Some(reference) => named(states, reference)?,
        None => held_by(states, card).map(str::to_owned).unwrap_or_default(),
    };
    let holder = states.get(&kanban).unwrap_or(&Value::Null);
    // Left out, a card already in the kanban keeps its own column and swimlane.
    let own = held_by(states, card) == Some(kanban.as_str());
    let kept = |field: &str| card.get(field).and_then(Value::as_str).filter(|_| own);
    let column = picked(
        (COLUMN, "the title of a column of the kanban"),
        column.as_deref(),
        (kept(COLUMN_ID), &columns_in(transaction, holder)),
    )?;
    let swimlane = picked(
        (SWIMLANE, "the title of a swimlane of the kanban"),
        swimlane.as_deref(),
        (kept(SWIMLANE_ID), &swimlanes_in(transaction, holder)),
    )?;
    let sits = |fields: &Value, key: &str, wanted: &str| {
        fields.get(key).and_then(Value::as_str) == Some(wanted)
    };
    let after = states
        .iter()
        .filter(|(other, fields)| {
            other.as_str() != id
                && parent_of(fields) == Some(kanban.as_str())
                && sits(fields, COLUMN_ID, &column)
                && sits(fields, SWIMLANE_ID, &swimlane)
        })
        .filter_map(|(_, fields)| fields.get(INDEX).and_then(Value::as_f64))
        .max_by(f64::total_cmp);
    let index = Cell {
        kanban: kanban.clone(),
        column: column.clone(),
        swimlane: swimlane.clone(),
        after,
    }
    .index();
    Ok(vec![
        (PARENT_ID, json!(kanban)),
        (SWIMLANE_ID, json!(swimlane)),
        (COLUMN_ID, json!(column)),
        (INDEX, json!(index)),
        (WIDTH, json!(CARD_WIDTH)),
        (SCALE, json!(scale_of(holder))),
    ])
}

fn scale_of(kanban: &Value) -> f64 {
    kanban.get(SCALE).and_then(Value::as_f64).unwrap_or(1.0)
}

/// Card out of its kanban: the kanban's corner moved right by earlier columns,
/// in the kanban's parent.
fn out_fields(
    transaction: &TransactionMut<'_>,
    states: &HashMap<String, Value>,
    card: &Value,
) -> Vec<(&'static str, Value)> {
    let holder = held_by(states, card)
        .and_then(|kanban| states.get(kanban))
        .unwrap_or(&Value::Null);
    let scale = scale_of(holder);
    let before = card
        .get(COLUMN_ID)
        .and_then(Value::as_str)
        .and_then(|column| {
            columns_in(transaction, holder)
                .iter()
                .position(|(id, _)| id == column)
        })
        .and_then(|columns| u32::try_from(columns).ok())
        .map_or(0.0, f64::from);
    let corner = |axis: &str| {
        holder
            .pointer(&format!("/{POSITION}/{axis}"))
            .and_then(Value::as_f64)
            .unwrap_or_default()
    };
    vec![
        (
            PARENT_ID,
            holder.get(PARENT_ID).cloned().unwrap_or_default(),
        ),
        (SWIMLANE_ID, Value::Null),
        (COLUMN_ID, Value::Null),
        (
            POSITION,
            json!({ (X): corner(X) + before * COLUMN_WIDTH * scale, (Y): corner(Y) }),
        ),
        (SCALE, json!(scale)),
    ]
}

pub(super) fn write(
    edit: &CardMove,
    transaction: &TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    (id, card): (&str, &Value),
) -> Result<Written> {
    let fields = match edit {
        CardMove::Out => out_fields(transaction, scope.states, card),
        CardMove::Into {
            kanban,
            column,
            swimlane,
        } => into_fields(
            transaction,
            scope.states,
            (id, card),
            (kanban, column, swimlane),
        )?,
    };
    Ok(Written {
        fields,
        touched: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Doc, Map as _, MapRef, ReadTxn as _, Transact as _, types::ToJson as _};

    use super::CardMove;
    use crate::{
        domain::{
            board,
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
            },
            geometry::Point,
            id_source::SequentialIds,
            kinds::{kanban, task},
            rich_text::TextFormat,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    const STEP: u64 = 1_099_511_627_776;

    fn entry(new: NewObject) -> NewEntry {
        NewEntry {
            new,
            frame: None,
            format: TextFormat::Plain,
        }
    }

    /// A kanban at 100,50 with default cells; cards at 10,20 and 30,40.
    fn board() -> Result<Doc> {
        let document = Doc::new();
        let mut issued = 0_u32;
        let mut kanban = kanban::made(&[], &[], || {
            issued += 1;
            format!("k{issued}")
        });
        kanban.at = Point::new(100.0, 50.0)?;
        let card = |id: &str, at: Point| {
            let mut made = task::made("Ship", None, (None, None), None, id.to_owned());
            made.entries.truncate(1);
            made.at = at;
            NewObject::Made(made)
        };
        let add = BoardChange::AddAll(vec![
            entry(NewObject::Made(kanban)),
            entry(card("i1", Point::new(10.0, 20.0)?)),
            entry(card("i2", Point::new(30.0, 40.0)?)),
        ]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn objects(document: &Doc) -> Vec<(String, Value)> {
        let transaction = document.transact();
        let mut all: Vec<(String, Value)> = transaction
            .get_map("objects")
            .map(|objects| {
                objects
                    .iter(&transaction)
                    .filter_map(|(id, value)| {
                        Some((
                            id.to_owned(),
                            board::json(&value.to_json(&transaction))?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        all.sort_by(|(first, _), (second, _)| first.cmp(second));
        all
    }

    /// The id of the object of `kind` that `keep` accepts.
    fn the(document: &Doc, kind: &str, keep: impl Fn(&Value) -> bool) -> String {
        objects(document)
            .into_iter()
            .find(|(_, fields)| word(fields, "type") == kind && keep(fields))
            .map(|(id, _)| id)
            .unwrap_or_default()
    }

    fn word(fields: &Value, name: &str) -> Value {
        fields.get(name).cloned().unwrap_or_default()
    }

    fn at(x: f64) -> impl Fn(&Value) -> bool {
        move |fields| {
            fields.pointer("/position/x").and_then(Value::as_f64) == Some(x)
                && word(fields, "parentId").is_null()
        }
    }

    fn fields(document: &Doc, id: &str) -> Value {
        objects(document)
            .into_iter()
            .find(|(each, _)| each == id)
            .map(|(_, fields)| fields)
            .unwrap_or_default()
    }

    fn into(column: Option<&str>) -> CardMove {
        CardMove::Into {
            kanban: Some("kanban".to_owned()),
            column: column.map(str::to_owned),
            swimlane: None,
        }
    }

    fn moved(document: &Doc, id: &str, to: &CardMove) -> Result<()> {
        let kanban = the(document, "kanban", |_| true);
        let to = match to {
            CardMove::Into {
                column, swimlane, ..
            } => CardMove::Into {
                kanban: Some(kanban),
                column: column.clone(),
                swimlane: swimlane.clone(),
            },
            CardMove::Out => CardMove::Out,
        };
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: id.to_owned(),
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Card(to)],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new("r"), &change).map(drop)
    }

    #[test]
    fn a_card_moved_into_done_gets_its_column_and_an_index_after_the_last_card()
    -> Result<()> {
        let document = board()?;
        let (first, second) = (
            the(&document, "task-card", at(10.0)),
            the(&document, "task-card", at(30.0)),
        );
        moved(&document, &first, &into(Some("done")))?;
        moved(&document, &second, &into(Some("Done")))?;
        let kanban = the(&document, "kanban", |_| true);
        let [one, two] = [&first, &second].map(|id| fields(&document, id));
        assert_eq!(
            (
                word(&one, "parentId"),
                word(&one, "columnId"),
                word(&one, "swimlaneId"),
                word(&one, "index"),
                word(&one, "width"),
                word(&two, "columnId"),
                word(&two, "index"),
            ),
            (
                json!(kanban),
                json!("k3"),
                json!("k4"),
                json!(STEP),
                json!(338),
                json!("k3"),
                json!(2 * STEP),
            )
        );
        Ok(())
    }

    #[test]
    fn a_card_moved_out_clears_the_cell_and_keeps_the_place_it_was_drawn_at()
    -> Result<()> {
        let document = board()?;
        let card = the(&document, "task-card", at(10.0));
        moved(&document, &card, &into(Some("done")))?;
        moved(&document, &card, &CardMove::Out)?;
        let out = fields(&document, &card);
        assert_eq!(
            (
                word(&out, "parentId"),
                word(&out, "swimlaneId"),
                word(&out, "columnId"),
                word(&out, "position"),
            ),
            (
                Value::Null,
                Value::Null,
                Value::Null,
                json!({"x": 780, "y": 50}),
            )
        );
        Ok(())
    }

    #[test]
    fn a_card_moved_out_of_a_kanban_in_a_frame_stays_in_the_frame() -> Result<()> {
        let document = board()?;
        let (kanban, card) = (
            the(&document, "kanban", |_| true),
            the(&document, "task-card", at(10.0)),
        );
        let mut transaction = document.transact_mut();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, &kanban))
            .and_then(|held| held.cast::<MapRef>().ok())
            .into_iter()
            .for_each(|map| {
                map.insert(&mut transaction, "parentId", "f1");
            });
        drop(transaction);
        moved(&document, &card, &into(Some("done")))?;
        moved(&document, &card, &CardMove::Out)?;
        assert_eq!(word(&fields(&document, &card), "parentId"), json!("f1"));
        Ok(())
    }

    #[test]
    fn an_unknown_column_is_refused_and_the_card_stays() -> Result<()> {
        let document = board()?;
        let card = the(&document, "task-card", at(10.0));
        let refused = moved(&document, &card, &into(Some("later"))).err();
        assert_eq!(
            refused.map(|error| error.to_string()),
            Some("\"later\" is not a valid column: expected the title of a column of the kanban".to_owned())
        );
        assert_eq!(word(&fields(&document, &card), "columnId"), Value::Null);
        Ok(())
    }

    #[test]
    fn an_object_that_is_no_task_card_is_refused() -> Result<()> {
        let document = board()?;
        let kanban = the(&document, "kanban", |_| true);
        let refused = moved(&document, &kanban, &into(None)).err();
        assert!(matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "kanban",
                ..
            }))
        ));
        Ok(())
    }
}
