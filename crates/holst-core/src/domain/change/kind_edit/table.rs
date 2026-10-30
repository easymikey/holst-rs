//! A table: a cell's text written, a row or a column inserted or deleted.

use std::collections::HashMap;

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{Any, Map as _, Out, Text as _, TransactionMut, XmlTextRef};

use super::{Written, only};
use crate::{
    domain::{
        area::scene::track_word,
        board::{ObjectKind, any, document_root, text_of},
        change::{
            check::Placement,
            made,
            rich::copy_into,
            value::{ChangedObject, Made},
            write::{
                WriteTransaction,
                changed,
                jitter,
                object_map,
                remove_all,
                write_text,
            },
        },
        geometry::Point,
        kinds::table::{CELL_LIMIT, COLUMN_WIDTH, MIN_HEIGHT, ROW_HEIGHT},
        palette::Color,
        rich_text::TextFormat,
    },
    error::{
        Error,
        Result,
        board_change::{BoardChangeError, TableRefusal},
    },
    protocol::document::{
        COLUMN_ID,
        COLUMN_PREFIX,
        DOCUMENT_ID,
        FILL_COLOR,
        HEIGHT,
        INDEX,
        PARENT_ID,
        ROW_ID,
        ROW_PREFIX,
        TYPE,
        UPDATED,
        WIDTH,
    },
};

const MERGE_PREFIX: &str = "merge-";
const FILL_COLOR_INDEX: &str = "fillColorIndex";
/// The styles a cell carries, each with the key of its precedence index.
const STYLES: [(&str, &str); 6] = [
    ("fillColor", "fillColorIndex"),
    ("fontFamily", "fontFamilyIndex"),
    ("fontSize", "fontSizeIndex"),
    ("verticalAlign", "verticalAlignIndex"),
    ("horizontalAlign", "horizontalAlignIndex"),
    ("textRotation", "textRotationIndex"),
];
const CELL: &str = "table-cell";
/// The step of the web client's `zH` past the last or before the first index.
const STEP: f64 = 1_099_511_627_776.0;

/// Rows or columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    Row,
    Column,
}

/// A block of cells from its first row and column to its last, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub first: (u32, u32),
    pub last: (u32, u32),
}

/// What an edit does to a table; places count from 1 in index order.
#[derive(Debug, Clone, PartialEq)]
pub enum TableEdit {
    /// The text of the cell at `row`, `column`, in the format given.
    Cell {
        row: u32,
        column: u32,
        text: String,
        format: TextFormat,
    },
    /// A new track before the n-th; one past the last appends.
    Insert(Track, u32),
    /// The n-th track with its cells.
    Delete(Track, u32),
    /// The cells of the block joined into one, the leader's text kept.
    Merge(Span),
    /// The merged block led by the cell at `row`, `column` split again.
    Unmerge { row: u32, column: u32 },
    /// The texts of the cells emptied and what the cells hold removed.
    Clear(Span),
    /// The fill colour of the cells.
    Fill(Span, Color),
}

impl Track {
    const fn prefix(self) -> &'static str {
        match self {
            Track::Row => ROW_PREFIX,
            Track::Column => COLUMN_PREFIX,
        }
    }

    const fn word(self) -> &'static str {
        match self {
            Track::Row => "row",
            Track::Column => "column",
        }
    }

    const fn id_field(self) -> &'static str {
        match self {
            Track::Row => ROW_ID,
            Track::Column => COLUMN_ID,
        }
    }
}

impl TableEdit {
    pub(super) const fn field(&self) -> &'static str {
        match self {
            TableEdit::Cell { .. } => "cell text",
            TableEdit::Insert(Track::Row | Track::Column, _)
            | TableEdit::Delete(Track::Row | Track::Column, _) => "rows and columns",
            TableEdit::Merge(_) | TableEdit::Unmerge { .. } => "merged cells",
            TableEdit::Clear(_) => "cell content",
            TableEdit::Fill(..) => "cell color",
        }
    }
}

fn refuse(why: TableRefusal) -> Error {
    Error::BoardChange(BoardChangeError::TableRefused { why })
}

fn text_of_field<'a>(fields: &'a Value, name: &str) -> Option<&'a str> {
    fields.get(name).and_then(Value::as_str)
}

/// The tracks of a table in index order: the number in each key, its index.
fn tracks(fields: &Value, track: Track) -> Vec<(String, f64)> {
    fields
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, stored)| {
            Some((
                key.strip_prefix(track.prefix())?.to_owned(),
                stored.get(INDEX)?.as_f64()?,
            ))
        })
        .sorted_by(|(_, a), (_, b)| a.total_cmp(b))
        .collect()
}

/// The number of the `at`-th track, from 1.
fn number_at(fields: &Value, track: Track, at: u32) -> Result<String> {
    let order = tracks(fields, track);
    usize::try_from(at)
        .ok()
        .and_then(|at| at.checked_sub(1))
        .and_then(|slot| order.get(slot))
        .map(|(number, _)| number.clone())
        .ok_or_else(|| {
            refuse(TableRefusal::OutOfRange {
                track: track.word(),
                at,
                most: order.len(),
            })
        })
}

/// The cell of a table at a row and a column: the first by id.
fn cell_at<'a>(
    states: &'a HashMap<String, Value>,
    table: &str,
    (row, column): (&str, &str),
) -> Option<&'a String> {
    states
        .iter()
        .filter(|(_, fields)| {
            text_of_field(fields, TYPE) == Some(CELL)
                && text_of_field(fields, PARENT_ID) == Some(table)
                && track_word(fields.get(ROW_ID)).as_deref() == Some(row)
                && track_word(fields.get(COLUMN_ID)).as_deref() == Some(column)
        })
        .map(|(id, _)| id)
        .min()
}

/// The cells of a table that stand in the track `number`.
fn cells_in<'a>(
    states: &'a HashMap<String, Value>,
    table: &str,
    (track, number): (Track, &str),
) -> Vec<&'a String> {
    states
        .iter()
        .filter(|(_, fields)| {
            text_of_field(fields, TYPE) == Some(CELL)
                && text_of_field(fields, PARENT_ID) == Some(table)
                && track_word(fields.get(track.id_field())).as_deref() == Some(number)
        })
        .map(|(id, _)| id)
        .sorted_unstable()
        .collect()
}

/// A merged block: the key it is stored under and how far it reaches.
struct Block {
    key: String,
    leader: (String, String),
    reach: (usize, usize),
}

fn blocks(fields: &Value) -> Vec<Block> {
    fields
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, stored)| stored.is_object())
        .filter_map(|(key, stored)| {
            let (row, column) = key.strip_prefix(MERGE_PREFIX)?.split_once('-')?;
            let reach = |name: &str| {
                stored
                    .get(name)
                    .and_then(Value::as_u64)
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(1)
            };
            Some(Block {
                key: key.clone(),
                leader: (
                    track_word(stored.get("row")).unwrap_or_else(|| row.to_owned()),
                    track_word(stored.get("column"))
                        .unwrap_or_else(|| column.to_owned()),
                ),
                reach: (reach("countRow"), reach("countColumn")),
            })
        })
        .collect()
}

/// The track number a block starts at and how many tracks of that kind it
/// covers.
const fn reach_on(block: &Block, track: Track) -> (&String, usize) {
    match track {
        Track::Row => (&block.leader.0, block.reach.0),
        Track::Column => (&block.leader.1, block.reach.1),
    }
}

/// The merged blocks that cover the track `number`, with how many tracks of
/// that kind each covers.
fn merged_over(fields: &Value, track: Track, number: &str) -> Vec<(String, usize)> {
    let order: Vec<String> = tracks(fields, track)
        .into_iter()
        .map(|(own, _)| own)
        .collect();
    let place = order.iter().position(|other| other == number);
    blocks(fields)
        .into_iter()
        .filter_map(|block| {
            let (leader, span) = reach_on(&block, track);
            let start = order.iter().position(|other| other == leader)?;
            place
                .filter(|place| (start..start + span).contains(place))
                .map(|_| (block.key.clone(), span))
        })
        .collect()
}

pub(super) fn check(
    edit: &TableEdit,
    kind: &ObjectKind,
    states: &HashMap<String, Value>,
    id: &str,
) -> Result<()> {
    let fields = states.get(id).unwrap_or(&Value::Null);
    only(kind, &ObjectKind::Table, edit.field())?;
    match edit {
        TableEdit::Cell { row, column, .. } => number_at(fields, Track::Row, *row)
            .and(number_at(fields, Track::Column, *column))
            .map(drop),
        TableEdit::Insert(track, at) => inserting(fields, *track, *at),
        TableEdit::Delete(track, at) => deleting(fields, *track, *at),
        TableEdit::Merge(span) => merging(fields, *span),
        TableEdit::Unmerge { row, column } => unmerging(fields, (*row, *column)),
        TableEdit::Clear(span) | TableEdit::Fill(span, _) => {
            grid(fields, *span).map(drop)
        }
    }
}

/// The places of a block, row by row.
fn grid(fields: &Value, span: Span) -> Result<Vec<(String, String)>> {
    let pick = |track: Track, from: u32, to: u32| {
        (from..=to)
            .map(|at| number_at(fields, track, at))
            .collect::<Result<Vec<_>>>()
    };
    let rows = pick(Track::Row, span.first.0, span.last.0)?;
    let columns = pick(Track::Column, span.first.1, span.last.1)?;
    Ok(rows
        .iter()
        .cartesian_product(&columns)
        .map(|(row, column)| (row.clone(), column.clone()))
        .collect())
}

/// The places a merged block covers.
fn block_cells(fields: &Value, block: &Block) -> Vec<(String, String)> {
    let reach = |track: Track, leader: &str, count: usize| -> Vec<String> {
        tracks(fields, track)
            .into_iter()
            .map(|(number, _)| number)
            .skip_while(|number| number != leader)
            .take(count)
            .collect()
    };
    let rows = reach(Track::Row, &block.leader.0, block.reach.0);
    let columns = reach(Track::Column, &block.leader.1, block.reach.1);
    rows.iter()
        .cartesian_product(&columns)
        .map(|(row, column)| (row.clone(), column.clone()))
        .collect()
}

fn merging(fields: &Value, span: Span) -> Result<()> {
    let cells = grid(fields, span)?;
    (cells.len() > 1)
        .then_some(())
        .ok_or_else(|| refuse(TableRefusal::OneCell))?;
    blocks(fields).iter().try_for_each(|block| {
        let held = block_cells(fields, block);
        let inside = held.iter().filter(|place| cells.contains(place)).count();
        (inside == 0 || inside == held.len())
            .then_some(())
            .ok_or_else(|| {
                refuse(TableRefusal::Ragged {
                    first: span.first,
                    last: span.last,
                })
            })
    })
}

/// The merged block whose leader stands at the place.
fn block_led_by(fields: &Value, (row, column): &(String, String)) -> Option<Block> {
    let key = format!("{MERGE_PREFIX}{row}-{column}");
    blocks(fields)
        .into_iter()
        .find(|block| block.key == key && (block.reach.0 > 1 || block.reach.1 > 1))
}

fn unmerging(fields: &Value, (row, column): (u32, u32)) -> Result<()> {
    let place = (
        number_at(fields, Track::Row, row)?,
        number_at(fields, Track::Column, column)?,
    );
    block_led_by(fields, &place)
        .map(drop)
        .ok_or_else(|| refuse(TableRefusal::NotMerged { row, column }))
}

/// The index that puts a style above every other the table, its tracks and
/// its cells hold (main.a33651a4.js 220625).
pub(in crate::domain::change) fn next_index(
    states: &HashMap<String, Value>,
    table: &str,
    index_key: &str,
) -> f64 {
    let held = |stored: &Value| stored.get(index_key).and_then(Value::as_f64);
    let own = states.get(table);
    let tracks = own
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(name, _)| {
            name.starts_with(ROW_PREFIX) || name.starts_with(COLUMN_PREFIX)
        })
        .map(|(_, stored)| stored);
    let cells = states.values().filter(|stored| {
        text_of_field(stored, TYPE) == Some(CELL)
            && text_of_field(stored, PARENT_ID) == Some(table)
    });
    own.into_iter()
        .chain(tracks)
        .chain(cells)
        .filter_map(held)
        .fold(0.0, f64::max)
        + 1.0
}

fn inserting(fields: &Value, track: Track, at: u32) -> Result<()> {
    let count = tracks(fields, track).len();
    let (rows, columns) = (
        tracks(fields, Track::Row).len(),
        tracks(fields, Track::Column).len(),
    );
    let after = match track {
        Track::Row => (rows + 1) * columns,
        Track::Column => rows * (columns + 1),
    };
    let limit = usize::try_from(CELL_LIMIT).unwrap_or(usize::MAX);
    let slot = usize::try_from(at).unwrap_or_default().saturating_sub(1);
    let order: Vec<String> = tracks(fields, track)
        .into_iter()
        .map(|(number, _)| number)
        .collect();
    let split = blocks(fields).iter().any(|block| {
        let (leader, span) = reach_on(block, track);
        order
            .iter()
            .position(|number| number == leader)
            .is_some_and(|start| start < slot && slot < start + span)
    });
    usize::try_from(at)
        .is_ok_and(|at| (1..=count + 1).contains(&at))
        .then_some(())
        .ok_or_else(|| {
            refuse(TableRefusal::OutOfRange {
                track: track.word(),
                at,
                most: count + 1,
            })
        })?;
    (after <= limit)
        .then_some(())
        .ok_or_else(|| refuse(TableRefusal::TooBig))?;
    (!split).then_some(()).ok_or_else(|| {
        refuse(TableRefusal::MergedBetween {
            track: track.word(),
            at,
        })
    })
}

fn deleting(fields: &Value, track: Track, at: u32) -> Result<()> {
    let number = number_at(fields, track, at)?;
    (tracks(fields, track).len() > 1)
        .then_some(())
        .ok_or_else(|| {
            refuse(TableRefusal::LastTrack {
                track: track.word(),
                at,
            })
        })?;
    merged_over(fields, track, &number)
        .iter()
        .all(|(_, span)| *span <= 1)
        .then_some(())
        .ok_or_else(|| {
            refuse(TableRefusal::MergedAcross {
                track: track.word(),
                at,
            })
        })
}

pub(super) fn write(
    edit: &TableEdit,
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    id: &str,
    fields: &Value,
) -> Result<Written> {
    match edit {
        TableEdit::Cell {
            row,
            column,
            text,
            format,
        } => {
            let place = (
                number_at(fields, Track::Row, *row)?,
                number_at(fields, Track::Column, *column)?,
            );
            cell(transaction, scope, id, &place, (text, *format))
        }
        TableEdit::Insert(track, at) => {
            insert(transaction, scope, id, fields, (*track, *at))
        }
        TableEdit::Delete(track, at) => {
            delete(transaction, scope, id, fields, (*track, *at))
        }
        TableEdit::Merge(span) => merge(transaction, scope, id, fields, *span),
        TableEdit::Unmerge { row, column } => {
            unmerge(transaction, scope, id, fields, (*row, *column))
        }
        TableEdit::Clear(span) => clear(transaction, scope, id, fields, *span),
        TableEdit::Fill(span, color) => {
            fill(transaction, scope, id, fields, (*span, color))
        }
    }
}

/// The text of the cell at a row and a column, in the cell that is there or a
/// new one.
fn cell(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    place: &(String, String),
    (text, format): (&str, TextFormat),
) -> Result<Written> {
    let rewritten = cell_at(scope.states, table, (&place.0, &place.1))
        .map(|cell| rewrite(transaction, scope, cell, (text, format), Vec::new()))
        .transpose()?;
    let touched = rewritten.map_or_else(
        || {
            create(
                transaction,
                scope,
                (table, place),
                (text, format),
                Vec::new(),
            )
        },
        Ok,
    )?;
    Ok(Written {
        fields: Vec::new(),
        touched: vec![touched],
    })
}

fn rewrite(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    cell: &str,
    (text, format): (&str, TextFormat),
    styles: Vec<(&'static str, Value)>,
) -> Result<ChangedObject> {
    let stored = scope.states.get(cell).unwrap_or(&Value::Null);
    let fields: Vec<_> = write_text(
        transaction,
        scope,
        stored,
        DOCUMENT_ID,
        (text, format, &ObjectKind::TableCell),
    )
    .into_iter()
    .chain(styles)
    .collect();
    changed(transaction, scope, cell.to_owned(), &fields, vec!["text"])
}

/// A track number as the web client stores it: a number.
fn track_value(word: &str) -> Value {
    word.parse::<u64>()
        .map_or_else(|_| json!(word), |number| json!(number))
}

/// `styles` and, once there are any, the styles the cell leaves unset as
/// `null` with an index of their own, as the web client creates a styled cell.
fn all_styles(
    scope: &WriteTransaction<'_>,
    table: &str,
    styles: Vec<(&'static str, Value)>,
) -> Vec<(&'static str, Value)> {
    let unset: Vec<_> = STYLES
        .iter()
        .filter(|(name, _)| {
            !styles.is_empty() && !styles.iter().any(|(set, _)| set == name)
        })
        .flat_map(|&(name, index)| {
            [
                (name, Value::Null),
                (index, json!(next_index(scope.states, table, index))),
            ]
        })
        .collect();
    styles.into_iter().chain(unset).collect()
}

/// A cell as the web client makes it when a cell gets text.
fn create(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    (table, (row, column)): (&str, &(String, String)),
    (text, format): (&str, TextFormat),
    styles: Vec<(&'static str, Value)>,
) -> Result<ChangedObject> {
    let made = Made {
        kind: ObjectKind::TableCell,
        at: Point::ORIGIN,
        size: None,
        parent: None,
        fields: [(ROW_ID, track_value(row)), (COLUMN_ID, track_value(column))]
            .into_iter()
            .chain(all_styles(scope, table, styles))
            .collect(),
        texts: vec![(DOCUMENT_ID, text.to_owned())],
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    };
    let placement = Placement {
        parent: Some(table.to_owned()),
        format,
    };
    made::write(transaction, scope, &made, &placement).map(|written| written.changed)
}

/// The `zH` of the web client: an index between the neighbours, or a step
/// past the one there is.
fn index_between(before: Option<f64>, after: Option<f64>, spread: f64) -> f64 {
    match (before, after) {
        (Some(low), Some(high)) => low + (high - low) * (0.1 + 0.8 * spread),
        (Some(low), None) => low + STEP + 10_000.0 * spread,
        (None, Some(high)) => high - STEP - 10_000.0 * spread,
        (None, None) => STEP,
    }
}

/// Entries of the table's own map, and its `updated` stamp.
fn put(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    entries: Vec<(String, Value)>,
) -> Result<()> {
    let map = object_map(transaction, &scope.roots.objects, table, &ObjectKind::Table)?;
    entries
        .into_iter()
        .chain([(UPDATED.to_owned(), scope.stamp.json())])
        .try_for_each(|(name, value)| {
            any(&value).map(|value| {
                map.insert(transaction, name, value);
            })
        })
}

fn insert(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    fields: &Value,
    (track, at): (Track, u32),
) -> Result<Written> {
    let order = tracks(fields, track);
    let slot = usize::try_from(at).unwrap_or(usize::MAX);
    let neighbour = |place: Option<usize>| {
        place
            .and_then(|place| order.get(place))
            .map(|(_, index)| *index)
    };
    let newest = fields
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, _)| key.strip_prefix(track.prefix())?.parse::<u32>().ok())
        .max()
        .unwrap_or_default();
    let key = format!("{}{}", track.prefix(), newest + 1);
    let index = index_between(
        neighbour(slot.checked_sub(2)),
        neighbour(slot.checked_sub(1)),
        jitter(&key),
    );
    let stored = match track {
        Track::Row => {
            json!({ (INDEX): index, (HEIGHT): ROW_HEIGHT, (MIN_HEIGHT): ROW_HEIGHT })
        }
        Track::Column => json!({ (INDEX): index, (WIDTH): COLUMN_WIDTH }),
    };
    put(transaction, scope, table, vec![(key, stored)])?;
    Ok(Written {
        fields: Vec::new(),
        touched: Vec::new(),
    })
}

/// The track, the merged blocks over it and the cells in it gone; a removed
/// cell that held text says so.
fn delete(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    fields: &Value,
    (track, at): (Track, u32),
) -> Result<Written> {
    let number = number_at(fields, track, at)?;
    let cells: Vec<String> = cells_in(scope.states, table, (track, &number))
        .into_iter()
        .cloned()
        .collect();
    let lettered: Vec<String> = cells
        .iter()
        .filter(|cell| holds_text(transaction, scope, cell))
        .cloned()
        .collect();
    let gone = std::iter::once(format!("{}{number}", track.prefix()))
        .chain(
            merged_over(fields, track, &number)
                .into_iter()
                .map(|(key, _)| key),
        )
        .map(|key| (key, Value::Null))
        .collect();
    put(transaction, scope, table, gone)?;
    let removed = Some(cells)
        .filter(|cells| !cells.is_empty())
        .map(|cells| remove_all(transaction, scope, &cells))
        .transpose()?
        .unwrap_or_default();
    Ok(Written {
        fields: Vec::new(),
        touched: removed
            .into_iter()
            .map(|object| ChangedObject {
                fields: lettered
                    .contains(&object.id)
                    .then_some("text")
                    .into_iter()
                    .collect(),
                ..object
            })
            .collect(),
    })
}

fn words_of(
    transaction: &TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    cell: &str,
) -> String {
    scope
        .states
        .get(cell)
        .and_then(|stored| text_of_field(stored, DOCUMENT_ID))
        .map(|document| text_of(transaction, &scope.roots.documents, document))
        .unwrap_or_default()
}

/// The text document a cell holds now.
fn document_of(
    transaction: &TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    cell: &str,
) -> Option<XmlTextRef> {
    let map = object_map(
        transaction,
        &scope.roots.objects,
        cell,
        &ObjectKind::TableCell,
    )
    .ok()?;
    match map.get(transaction, DOCUMENT_ID)? {
        Out::Any(Any::String(document)) => {
            document_root(transaction, &scope.roots.documents, &document)
        }
        Out::Any(_)
        | Out::YText(_)
        | Out::YArray(_)
        | Out::YMap(_)
        | Out::YXmlElement(_)
        | Out::YXmlFragment(_)
        | Out::YXmlText(_)
        | Out::YDoc(_)
        | Out::UndefinedRef(_) => None,
    }
}

fn holds_text(
    transaction: &TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    cell: &str,
) -> bool {
    !words_of(transaction, scope, cell).trim().is_empty()
}

/// The styles a cell sets, each with a fresh index above all others.
fn restyled(
    scope: &WriteTransaction<'_>,
    table: &str,
    source: &Value,
) -> Vec<(&'static str, Value)> {
    STYLES
        .iter()
        .filter_map(|&(name, index)| {
            let value = source.get(name).filter(|value| !value.is_null())?;
            Some([
                (name, value.clone()),
                (index, json!(next_index(scope.states, table, index))),
            ])
        })
        .flatten()
        .collect()
}

/// The cell at a place if the table has an object for it.
fn present<'a>(
    scope: &'a WriteTransaction<'_>,
    table: &str,
    (row, column): &(String, String),
) -> Option<&'a String> {
    cell_at(scope.states, table, (row, column))
}

/// The cell blank, or already there and emptied, with the styles given.
fn set_up(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    place: &(String, String),
    (text, styles): (&str, Vec<(&'static str, Value)>),
) -> Result<ChangedObject> {
    match present(scope, table, place) {
        Some(cell) => {
            rewrite(transaction, scope, cell, (text, TextFormat::Plain), styles)
        }
        None => create(
            transaction,
            scope,
            (table, place),
            (text, TextFormat::Plain),
            styles,
        ),
    }
}

/// The block joined: its top left cell holds the text and styles of the first
/// cell with text, as the web client moves them (main.a33651a4.js
/// 221136-221150).
fn merge(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    fields: &Value,
    span: Span,
) -> Result<Written> {
    let cells = grid(fields, span)?;
    let corner = cells.first().cloned().unwrap_or_default();
    let leader = cells
        .iter()
        .find(|place| {
            present(scope, table, place)
                .is_some_and(|cell| holds_text(transaction, scope, cell))
        })
        .unwrap_or(&corner)
        .clone();
    let carried = (leader != corner)
        .then(|| {
            let source = present(scope, table, &leader)
                .and_then(|cell| scope.states.get(cell))
                .unwrap_or(&Value::Null);
            let styles = restyled(scope, table, source);
            let led = present(scope, table, &leader)
                .and_then(|cell| document_of(transaction, scope, cell));
            let made = set_up(transaction, scope, table, &corner, ("", styles))?;
            let into = document_of(transaction, scope, &made.id);
            led.zip(into).iter().for_each(|(from, to)| {
                let len = to.len(transaction);
                to.remove_range(transaction, 0, len);
                copy_into(transaction, (from, to), scope.context.ids);
            });
            Ok::<_, Error>(made)
        })
        .transpose()?;
    let counts = (
        span.last.0 - span.first.0 + 1,
        span.last.1 - span.first.1 + 1,
    );
    let block = json!({
        "countRow": counts.0, "countColumn": counts.1,
        "row": track_value(&corner.0), "column": track_value(&corner.1),
    });
    let inside = blocks(fields)
        .into_iter()
        .filter(|held| {
            block_cells(fields, held)
                .iter()
                .all(|place| cells.contains(place))
        })
        .map(|held| (held.key, Value::Null));
    let key = format!("{MERGE_PREFIX}{}-{}", corner.0, corner.1);
    put(
        transaction,
        scope,
        table,
        inside.chain([(key, block)]).collect(),
    )?;
    Ok(Written {
        fields: Vec::new(),
        touched: carried.into_iter().collect(),
    })
}

/// The block split: the cells it covered are emptied and take the leader's
/// styles, as the web client does (main.a33651a4.js 221084-221110).
fn unmerge(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    fields: &Value,
    (row, column): (u32, u32),
) -> Result<Written> {
    let place = (
        number_at(fields, Track::Row, row)?,
        number_at(fields, Track::Column, column)?,
    );
    let key = format!("{MERGE_PREFIX}{}-{}", place.0, place.1);
    let covered: Vec<(String, String)> = block_led_by(fields, &place)
        .into_iter()
        .flat_map(|block| block_cells(fields, &block))
        .filter(|other| *other != place)
        .collect();
    let source = present(scope, table, &place)
        .and_then(|cell| scope.states.get(cell))
        .unwrap_or(&Value::Null);
    let styles = restyled(scope, table, source);
    put(transaction, scope, table, vec![(key, Value::Null)])?;
    let touched = covered
        .iter()
        .filter(|other| present(scope, table, other).is_some() || !styles.is_empty())
        .map(|other| set_up(transaction, scope, table, other, ("", styles.clone())))
        .collect::<Result<Vec<_>>>()?;
    Ok(Written {
        fields: Vec::new(),
        touched,
    })
}

/// The texts of the cells emptied, what they hold and what the cells they
/// lead cover hold removed (main.a33651a4.js 221470-221487); a cell that had
/// text says so.
fn clear(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    fields: &Value,
    span: Span,
) -> Result<Written> {
    let cells = grid(fields, span)?;
    let own: Vec<String> = cells
        .iter()
        .filter_map(|place| present(scope, table, place).cloned())
        .collect();
    let covered = blocks(fields)
        .into_iter()
        .filter(|block| cells.contains(&block.leader))
        .flat_map(|block| block_cells(fields, &block))
        .filter_map(|place| present(scope, table, &place).cloned());
    let holders: Vec<String> = own.iter().cloned().chain(covered).unique().collect();
    let held: Vec<String> = scope
        .states
        .iter()
        .filter(|(_, stored)| {
            text_of_field(stored, PARENT_ID)
                .is_some_and(|parent| holders.iter().any(|holder| holder == parent))
        })
        .map(|(id, _)| id.clone())
        .sorted_unstable()
        .collect();
    let lettered: Vec<String> = own
        .into_iter()
        .filter(|cell| holds_text(transaction, scope, cell))
        .collect();
    let emptied = lettered
        .iter()
        .map(|cell| {
            rewrite(
                transaction,
                scope,
                cell,
                ("", TextFormat::Plain),
                Vec::new(),
            )
            .map(|object| ChangedObject {
                fields: vec!["text cleared"],
                ..object
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let removed = Some(held)
        .filter(|held| !held.is_empty())
        .map(|held| remove_all(transaction, scope, &held))
        .transpose()?
        .unwrap_or_default();
    Ok(Written {
        fields: Vec::new(),
        touched: emptied.into_iter().chain(removed).collect(),
    })
}

/// The colour on the table when every cell is chosen, on the rows or columns
/// when whole ones are, else on the cells, made first if they are not there
/// (main.a33651a4.js 220650-220695).
fn fill(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    table: &str,
    fields: &Value,
    (span, color): (Span, &Color),
) -> Result<Written> {
    let cells = grid(fields, span)?;
    let style = [
        (FILL_COLOR, json!({ "color": color.as_str(), "opacity": 1 })),
        (
            FILL_COLOR_INDEX,
            json!(next_index(scope.states, table, FILL_COLOR_INDEX)),
        ),
    ];
    let reaches = |track: Track, last: u32| {
        usize::try_from(last) == Ok(tracks(fields, track).len())
    };
    let whole_columns = span.first.0 == 1 && reaches(Track::Row, span.last.0);
    let whole_rows = span.first.1 == 1 && reaches(Track::Column, span.last.1);
    let mut band = |track: Track, numbers: Vec<&String>| {
        let entries = numbers
            .into_iter()
            .unique()
            .map(|number| {
                let key = format!("{}{number}", track.prefix());
                let mut stored = fields
                    .get(&key)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                stored.extend(
                    style
                        .iter()
                        .map(|(name, value)| ((*name).to_owned(), value.clone())),
                );
                (key, Value::Object(stored))
            })
            .collect();
        put(transaction, scope, table, entries).map(|()| Written {
            fields: Vec::new(),
            touched: Vec::new(),
        })
    };
    match (whole_columns, whole_rows) {
        (true, true) => Ok(Written {
            fields: style.to_vec(),
            touched: Vec::new(),
        }),
        (false, true) => band(Track::Row, cells.iter().map(|place| &place.0).collect()),
        (true, false) => {
            band(Track::Column, cells.iter().map(|place| &place.1).collect())
        }
        (false, false) => {
            let touched = cells
                .iter()
                .map(|place| match present(scope, table, place) {
                    Some(cell) => {
                        changed(transaction, scope, cell.clone(), &style, vec!["color"])
                    }
                    None => create(
                        transaction,
                        scope,
                        (table, place),
                        ("", TextFormat::Plain),
                        style.to_vec(),
                    ),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Written {
                fields: Vec::new(),
                touched,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::collections::BTreeMap;

    use serde_json::{Value, json};
    use yrs::{
        Doc,
        Map as _,
        ReadTxn as _,
        Transact as _,
        WriteTxn as _,
        types::ToJson as _,
    };

    use super::{Span, TableEdit, Track, track_word};
    use crate::{
        domain::{
            board::{self, any, insert_map, text_of},
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{
                    BoardChange,
                    ChangeUpdate,
                    NewEntry,
                    NewObject,
                    ObjectChangeKind,
                    ObjectEdit,
                    ObjectPatch,
                },
            },
            id_source::SequentialIds,
            kinds::table,
            palette::Color,
            rich_text::TextFormat,
        },
        error::{
            Error,
            Result,
            board_change::{BoardChangeError, TableRefusal},
        },
    };

    fn table_of(rows: u32, columns: u32) -> Result<Doc> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new: NewObject::Made(table::made(rows, columns)),
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn objects(document: &Doc) -> BTreeMap<String, Value> {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .into_iter()
            .flat_map(|objects| {
                objects
                    .iter(&transaction)
                    .filter_map(|(id, value)| {
                        Some((
                            id.to_owned(),
                            board::json(&value.to_json(&transaction))?,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn of_type(document: &Doc, kind: &str) -> Vec<(String, Value)> {
        objects(document)
            .into_iter()
            .filter(|(_, fields)| at(fields, &["type"]) == kind)
            .collect()
    }

    fn the_table(document: &Doc) -> (String, Value) {
        of_type(document, "table")
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
        path.iter()
            .fold(value, |held, name| held.get(*name).unwrap_or(&Value::Null))
    }

    fn edit(document: &Doc, table_edit: TableEdit) -> Result<ChangeUpdate> {
        edit_as(document, "r", table_edit)
    }

    fn edit_as(
        document: &Doc,
        prefix: &'static str,
        table_edit: TableEdit,
    ) -> Result<ChangeUpdate> {
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: the_table(document).0,
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Table(table_edit)],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new(prefix), &change)
    }

    fn cell(row: u32, column: u32, text: &str) -> TableEdit {
        TableEdit::Cell {
            row,
            column,
            text: text.to_owned(),
            format: TextFormat::Plain,
        }
    }

    fn cell_text(document: &Doc, cell: &Value) -> String {
        let transaction = document.transact();
        let documents = transaction.get_map("documents");
        documents.map_or_else(String::new, |documents| {
            text_of(
                &transaction,
                &documents,
                at(cell, &["documentId"]).as_str().unwrap_or_default(),
            )
        })
    }

    fn put(document: &Doc, id: &str, fields: &Value) -> Result<()> {
        let mut transaction = document.transact_mut();
        let objects = transaction.get_or_insert_map("objects");
        let pairs = fields
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>();
        insert_map(&mut transaction, &objects, id, pairs)
    }

    fn merge(document: &Doc, key: &str, block: &Value) -> Result<()> {
        let (id, _) = the_table(document);
        let mut transaction = document.transact_mut();
        let objects = transaction.get_or_insert_map("objects");
        let map = match objects.get(&transaction, id.as_str()) {
            Some(yrs::Out::YMap(map)) => Some(map),
            _ => None,
        };
        map.map_or(Ok(()), |map| {
            any(block).map(|block| {
                map.insert(&mut transaction, key, block);
            })
        })
    }

    fn a_cell_at(document: &Doc, id: &str, (row, column): (u32, u32)) -> Result<()> {
        put(
            document,
            id,
            &json!({
                "id": id, "type": "table-cell", "parentId": the_table(document).0,
                "rowId": row.to_string(), "columnId": column.to_string(),
                "position": { "x": 0, "y": 0 }, "zIndex": 1,
            }),
        )
    }

    fn refusal(result: Result<ChangeUpdate>) -> Option<String> {
        match result.err() {
            Some(Error::BoardChange(BoardChangeError::TableRefused { why })) => {
                Some(why.to_string())
            }
            _ => None,
        }
    }

    fn index_of(document: &Doc, key: &str) -> f64 {
        at(&the_table(document).1, &[key, "index"])
            .as_f64()
            .unwrap_or(f64::NAN)
    }

    #[test]
    fn text_in_an_empty_cell_makes_one_cell_object() -> Result<()> {
        let document = table_of(2, 3)?;
        let update = edit(&document, cell(1, 2, "Hi"))?;
        let cells = of_type(&document, "table-cell");
        let made = cells
            .first()
            .map(|(_, fields)| fields.clone())
            .unwrap_or_default();
        assert_eq!(
            (
                cells.len(),
                at(&made, &["parentId"]),
                at(&made, &["rowId"]),
                at(&made, &["columnId"]),
                at(&made, &["position"]),
                cell_text(&document, &made),
                update
                    .objects
                    .iter()
                    .map(|object| (object.kind.as_str(), object.change))
                    .collect::<Vec<_>>(),
            ),
            (
                1,
                &json!(the_table(&document).0),
                &json!(1),
                &json!(2),
                &json!({"x": 0, "y": 0}),
                "Hi".to_owned(),
                vec![
                    ("table", ObjectChangeKind::Changed),
                    ("table-cell", ObjectChangeKind::Created)
                ],
            )
        );
        Ok(())
    }

    #[test]
    fn text_in_a_filled_cell_rewrites_it() -> Result<()> {
        let document = table_of(2, 3)?;
        edit(&document, cell(2, 3, "first"))?;
        edit(&document, cell(2, 3, "second"))?;
        let cells = of_type(&document, "table-cell");
        let texts: Vec<String> = cells
            .iter()
            .map(|(_, made)| cell_text(&document, made))
            .collect();
        assert_eq!(texts, vec!["second".to_owned()]);
        Ok(())
    }

    #[test]
    fn markdown_in_a_cell_may_be_a_heading() -> Result<()> {
        let document = table_of(1, 1)?;
        edit(
            &document,
            TableEdit::Cell {
                row: 1,
                column: 1,
                text: "# Title".to_owned(),
                format: TextFormat::Markdown,
            },
        )?;
        let cells = of_type(&document, "table-cell");
        let kept: Vec<String> = cells
            .iter()
            .map(|(_, made)| cell_text(&document, made))
            .collect();
        assert_eq!(kept, vec!["Title".to_owned()]);
        Ok(())
    }

    #[test]
    fn a_row_inserted_between_rows_gets_an_index_between_theirs() -> Result<()> {
        let document = table_of(2, 3)?;
        edit(&document, TableEdit::Insert(Track::Row, 2))?;
        let table = the_table(&document).1;
        let (first, second) =
            (index_of(&document, "row-1"), index_of(&document, "row-2"));
        let fresh = index_of(&document, "row-3");
        assert_eq!(
            (
                first < fresh && fresh < second,
                at(&table, &["row-3", "height"]),
                at(&table, &["row-3", "minHeight"])
            ),
            (true, &json!(60), &json!(60))
        );
        Ok(())
    }

    #[test]
    fn rows_and_columns_insert_at_both_ends() -> Result<()> {
        let document = table_of(2, 3)?;
        edit(&document, TableEdit::Insert(Track::Row, 1))?;
        edit(&document, TableEdit::Insert(Track::Row, 4))?;
        edit(&document, TableEdit::Insert(Track::Column, 4))?;
        let table = the_table(&document).1;
        assert_eq!(
            (
                index_of(&document, "row-3") < index_of(&document, "row-1"),
                index_of(&document, "row-4") > index_of(&document, "row-2"),
                index_of(&document, "column-4") > index_of(&document, "column-3"),
                at(&table, &["column-4", "width"]),
            ),
            (true, true, true, &json!(240))
        );
        Ok(())
    }

    #[test]
    fn deleting_a_row_removes_its_cells_and_what_they_hold() -> Result<()> {
        let document = table_of(2, 2)?;
        a_cell_at(&document, "c11", (1, 1))?;
        a_cell_at(&document, "c12", (1, 2))?;
        a_cell_at(&document, "c21", (2, 1))?;
        put(
            &document,
            "inside",
            &json!({"id": "inside", "type": "sticker", "parentId": "c11",
                    "position": {"x": 1, "y": 1}, "zIndex": 2}),
        )?;
        edit(&document, TableEdit::Delete(Track::Row, 1))?;
        let left: Vec<String> = objects(&document).into_keys().collect();
        let table = the_table(&document).1;
        assert_eq!(
            (
                at(&table, &["row-1"]),
                at(&table, &["row-2", "index"]) != &Value::Null,
                left.contains(&"c21".to_owned()),
                left.len()
            ),
            (&Value::Null, true, true, 2)
        );
        Ok(())
    }

    #[test]
    fn a_merge_inside_the_band_goes_and_one_across_it_refuses() -> Result<()> {
        let document = table_of(2, 2)?;
        merge(
            &document,
            "merge-1-1",
            &json!({"countRow": 2, "countColumn": 1, "row": "1", "column": "1"}),
        )?;
        let across = refusal(edit(&document, TableEdit::Delete(Track::Row, 2)));
        edit(&document, TableEdit::Delete(Track::Column, 1))?;
        let table = the_table(&document).1;
        assert_eq!(
            (
                across.as_deref(),
                at(&table, &["merge-1-1"]),
                at(&table, &["column-1"])
            ),
            (
                Some(
                    "a merged block crosses row 2: unmerge it in the web client first"
                ),
                &Value::Null,
                &Value::Null
            )
        );
        Ok(())
    }

    #[test]
    fn a_place_outside_the_table_is_refused() -> Result<()> {
        let document = table_of(2, 3)?;
        let refused: Vec<Option<String>> = [
            cell(3, 1, "x"),
            cell(1, 4, "x"),
            TableEdit::Insert(Track::Row, 4),
            TableEdit::Insert(Track::Column, 0),
            TableEdit::Delete(Track::Column, 4),
        ]
        .into_iter()
        .map(|table_edit| refusal(edit(&document, table_edit)))
        .collect();
        assert_eq!(
            refused,
            [
                "row 3 is not in the table: use 1 to 2",
                "column 4 is not in the table: use 1 to 3",
                "row 4 is not in the table: use 1 to 3",
                "column 0 is not in the table: use 1 to 4",
                "column 4 is not in the table: use 1 to 3",
            ]
            .map(|text| Some(text.to_owned()))
        );
        Ok(())
    }

    #[test]
    fn the_last_track_stays_and_a_full_table_takes_no_more() -> Result<()> {
        let one = table_of(1, 1)?;
        let full = table_of(1, 2500)?;
        assert_eq!(
            (
                refusal(edit(&one, TableEdit::Delete(Track::Row, 1))),
                refusal(edit(&full, TableEdit::Insert(Track::Row, 1))),
            ),
            (
                Some("row 1 is the last row: remove the table instead".to_owned()),
                Some("a table holds at most 2500 cells".to_owned()),
            )
        );
        Ok(())
    }

    #[test]
    fn a_sticker_takes_no_table_edit() -> Result<()> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new: NewObject::Text {
                text: "t".into(),
                at: crate::domain::geometry::Point::ORIGIN,
            },
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        let id = objects(&document).into_keys().next().unwrap_or_default();
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: id,
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Table(TableEdit::Insert(Track::Row, 1))],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        let refused =
            built_with_ids(&document, &SequentialIds::new("r"), &change).err();
        assert!(
            matches!(
                refused,
                Some(Error::BoardChange(BoardChangeError::NotChangeable { .. }))
            ),
            "{refused:?}"
        );
        Ok(())
    }

    #[test]
    fn a_removed_cell_with_text_says_so() -> Result<()> {
        let document = table_of(2, 2)?;
        edit(&document, cell(1, 1, "kept words"))?;
        a_cell_at(&document, "empty", (1, 2))?;
        let update = edit(&document, TableEdit::Delete(Track::Row, 1))?;
        let removed: Vec<(bool, Vec<&str>)> = update
            .objects
            .iter()
            .filter(|object| object.change == ObjectChangeKind::Removed)
            .map(|object| (object.id == "empty", object.fields.clone()))
            .collect();
        assert_eq!(removed.len(), 2);
        assert_eq!(
            removed
                .into_iter()
                .filter(|(_, fields)| !fields.is_empty())
                .count(),
            1
        );
        Ok(())
    }

    fn span(first: (u32, u32), last: (u32, u32)) -> Span {
        Span { first, last }
    }

    fn blue() -> Result<Color> {
        "blue3".parse()
    }

    fn cell_at_place(document: &Doc, (row, column): (&str, &str)) -> Value {
        of_type(document, "table-cell")
            .into_iter()
            .map(|(_, fields)| fields)
            .find(|fields| {
                track_word(fields.get("rowId")).as_deref() == Some(row)
                    && track_word(fields.get("columnId")).as_deref() == Some(column)
            })
            .unwrap_or_default()
    }

    #[test]
    fn merging_two_cells_writes_the_key_and_keeps_the_leaders_text() -> Result<()> {
        let document = table_of(2, 2)?;
        edit(&document, cell(1, 2, "lead"))?;
        edit_as(&document, "m", TableEdit::Merge(span((1, 1), (1, 2))))?;
        let table = the_table(&document).1;
        let first = cell_at_place(&document, ("1", "1"));
        assert_eq!(
            (
                at(&table, &["merge-1-1"]).clone(),
                cell_text(&document, &first)
            ),
            (
                json!({"countRow": 1, "countColumn": 2, "row": 1, "column": 1}),
                "lead".to_owned()
            )
        );
        Ok(())
    }

    #[test]
    fn a_ragged_or_single_merge_is_refused() -> Result<()> {
        let document = table_of(2, 2)?;
        merge(
            &document,
            "merge-1-1",
            &json!({"countRow": 2, "countColumn": 1, "row": "1", "column": "1"}),
        )?;
        assert_eq!(
            (
                refusal(edit(&document, TableEdit::Merge(span((1, 1), (1, 2))))),
                refusal(edit(&document, TableEdit::Merge(span((2, 2), (2, 2))))),
            ),
            (
                Some("a merged block reaches out of cells 1,1 to 1,2: take all of it or none".to_owned()),
                Some("merging takes at least two cells".to_owned()),
            )
        );
        Ok(())
    }

    #[test]
    fn unmerging_removes_the_key_and_a_cell_that_leads_none_is_refused() -> Result<()> {
        let document = table_of(2, 2)?;
        merge(
            &document,
            "merge-1-1",
            &json!({"countRow": 1, "countColumn": 2, "row": "1", "column": "1"}),
        )?;
        let alone = refusal(edit(&document, TableEdit::Unmerge { row: 2, column: 2 }));
        edit(&document, TableEdit::Unmerge { row: 1, column: 1 })?;
        assert_eq!(
            (alone, at(&the_table(&document).1, &["merge-1-1"]).clone()),
            (
                Some("cell 2,2 does not lead a merged block".to_owned()),
                Value::Null
            )
        );
        Ok(())
    }

    #[test]
    fn clearing_empties_the_texts_and_removes_what_the_cells_hold() -> Result<()> {
        let document = table_of(2, 2)?;
        a_cell_at(&document, "c11", (1, 1))?;
        a_cell_at(&document, "c12", (1, 2))?;
        edit(&document, cell(1, 1, "gone"))?;
        edit_as(&document, "q", cell(1, 2, "kept"))?;
        put(
            &document,
            "inside",
            &json!({"id": "inside", "type": "sticker",
                    "parentId": "c11",
                    "position": {"x": 1, "y": 1}, "zIndex": 2}),
        )?;
        let update = edit(&document, TableEdit::Clear(span((1, 1), (1, 1))))?;
        let (first, second) = (
            cell_at_place(&document, ("1", "1")),
            cell_at_place(&document, ("1", "2")),
        );
        assert_eq!(
            (
                cell_text(&document, &first),
                cell_text(&document, &second),
                objects(&document).contains_key("inside"),
                update
                    .objects
                    .iter()
                    .any(|object| object.fields.contains(&"text cleared")),
            ),
            (String::new(), "kept".to_owned(), false, true)
        );
        Ok(())
    }

    #[test]
    fn a_cell_colour_gets_an_index_above_all_the_others() -> Result<()> {
        let document = table_of(2, 2)?;
        merge(
            &document,
            "row-1",
            &json!({"index": 1, "fillColor": {"color": "red4", "opacity": 1}, "fillColorIndex": 5}),
        )?;
        edit(&document, TableEdit::Fill(span((1, 1), (1, 1)), blue()?))?;
        let made = cell_at_place(&document, ("1", "1"));
        assert_eq!(
            (
                at(&made, &["fillColor"]).clone(),
                at(&made, &["fillColorIndex"]).clone()
            ),
            (json!({"color": "blue3", "opacity": 1}), json!(6))
        );
        Ok(())
    }

    #[test]
    fn a_cell_made_by_a_colour_has_every_style_and_index_at_layer_zero() -> Result<()> {
        let document = table_of(2, 2)?;
        edit(&document, TableEdit::Fill(span((1, 1), (1, 1)), blue()?))?;
        let made = cell_at_place(&document, ("1", "1"));
        let styles = [
            "fillColor",
            "fontFamily",
            "fontSize",
            "verticalAlign",
            "horizontalAlign",
            "textRotation",
        ];
        assert_eq!(
            (
                styles.map(|name| at(&made, &[name]).is_null()),
                styles.map(|name| at(&made, &[&format!("{name}Index")]).clone()),
                at(&made, &["zIndex"]).clone(),
            ),
            (
                [false, true, true, true, true, true],
                [json!(2), json!(1), json!(1), json!(1), json!(1), json!(1)],
                json!(0)
            )
        );
        Ok(())
    }

    #[test]
    fn a_colour_on_every_cell_goes_to_the_table_and_on_a_band_to_its_tracks()
    -> Result<()> {
        let document = table_of(2, 2)?;
        edit(&document, TableEdit::Fill(span((1, 1), (2, 2)), blue()?))?;
        edit(&document, TableEdit::Fill(span((2, 1), (2, 2)), blue()?))?;
        edit(&document, TableEdit::Fill(span((1, 2), (2, 2)), blue()?))?;
        let table = the_table(&document).1;
        assert_eq!(
            (
                at(&table, &["fillColor", "color"]).clone(),
                at(&table, &["fillColorIndex"]).clone(),
                at(&table, &["row-2", "fillColorIndex"]).clone(),
                at(&table, &["column-2", "fillColorIndex"]).clone(),
                at(&table, &["row-2", "index"]) != &Value::Null,
                of_type(&document, "table-cell").len(),
            ),
            (json!("blue3"), json!(2), json!(3), json!(4), true, 0)
        );
        Ok(())
    }

    #[test]
    fn a_colour_on_a_whole_table_writes_the_index_beside_it() -> Result<()> {
        let document = table_of(2, 2)?;
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: the_table(&document).0,
            patch: ObjectPatch {
                color: Some(blue()?),
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(&document, &SequentialIds::new("r"), &change)?;
        let table = the_table(&document).1;
        assert_eq!(
            (
                at(&table, &["fillColor", "color"]).clone(),
                at(&table, &["fillColorIndex"]).clone()
            ),
            (json!("blue3"), json!(2))
        );
        Ok(())
    }

    fn a_web_cell_at(
        document: &Doc,
        id: &str,
        (row, column): (u32, u32),
    ) -> Result<()> {
        put(
            document,
            id,
            &json!({
                "id": id, "type": "table-cell", "parentId": the_table(document).0,
                "rowId": row, "columnId": column,
                "position": { "x": 0, "y": 0 }, "zIndex": 1,
            }),
        )
    }

    fn markdown_at(document: &Doc, place: (&str, &str)) -> String {
        board::object_of(cell_at_place(document, place))
            .and_then(|cell| board::markdown_text(document, &cell))
            .unwrap_or_default()
    }

    #[test]
    fn cells_with_number_ids_or_text_ids_are_found() -> Result<()> {
        let document = table_of(2, 2)?;
        a_web_cell_at(&document, "web", (1, 1))?;
        a_cell_at(&document, "beta", (2, 2))?;
        edit(&document, cell(1, 1, "Bye"))?;
        edit_as(&document, "m", cell(2, 2, "Yo"))?;
        let texts: Vec<String> = of_type(&document, "table-cell")
            .iter()
            .map(|(id, made)| format!("{id}={}", cell_text(&document, made)))
            .collect();
        assert_eq!(texts, ["beta=Yo", "web=Bye"]);
        Ok(())
    }

    #[test]
    fn deleting_a_row_removes_the_cells_the_web_made() -> Result<()> {
        let document = table_of(2, 2)?;
        a_web_cell_at(&document, "web", (1, 1))?;
        a_web_cell_at(&document, "kept", (2, 1))?;
        edit(&document, TableEdit::Delete(Track::Row, 1))?;
        let left: Vec<String> = of_type(&document, "table-cell")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(left, ["kept"]);
        Ok(())
    }

    #[test]
    fn two_deletes_of_one_table_in_one_call_are_checked_one_after_the_other()
    -> Result<()> {
        let document = table_of(2, 1)?;
        let object = the_table(&document).0;
        let delete = || ObjectEdit {
            object: object.clone(),
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Table(TableEdit::Delete(Track::Row, 1))],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        };
        let change = BoardChange::EditAll(vec![delete(), delete()]);
        let refused =
            refusal(built_with_ids(&document, &SequentialIds::new("r"), &change));
        assert_eq!(
            refused.as_deref(),
            Some("row 1 is the last row: remove the table instead")
        );
        Ok(())
    }

    fn tracks_of(document: &Doc, prefix: &str) -> usize {
        the_table(document)
            .1
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, stored)| key.starts_with(prefix) && stored.is_object())
            .count()
    }

    #[test]
    fn a_merge_keeps_the_formatted_text_of_its_leader() -> Result<()> {
        let document = table_of(1, 2)?;
        edit(
            &document,
            TableEdit::Cell {
                row: 1,
                column: 2,
                text: "**Total** [now](https://example.com)".to_owned(),
                format: TextFormat::Markdown,
            },
        )?;
        let leader = markdown_at(&document, ("1", "2"));
        edit_as(&document, "m", TableEdit::Merge(span((1, 1), (1, 2))))?;
        assert_eq!(
            (markdown_at(&document, ("1", "1")), leader.is_empty()),
            (leader, false)
        );
        Ok(())
    }

    #[test]
    fn a_row_cannot_be_inserted_inside_a_merged_block() -> Result<()> {
        let document = table_of(3, 2)?;
        merge(
            &document,
            "merge-1-1",
            &json!({"countRow": 2, "countColumn": 1, "row": "1", "column": "1"}),
        )?;
        let inside = refusal(edit(&document, TableEdit::Insert(Track::Row, 2)));
        edit(&document, TableEdit::Insert(Track::Row, 1))?;
        edit(&document, TableEdit::Insert(Track::Row, 4))?;
        assert_eq!(
            (inside.as_deref(), tracks_of(&document, "row-")),
            (
                Some(
                    "a merged block crosses the place of new row 2: unmerge it in the web client first"
                ),
                5
            )
        );
        Ok(())
    }

    #[test]
    fn the_refusals_are_words() {
        assert_eq!(
            TableRefusal::MergedAcross {
                track: "row",
                at: 2
            }
            .to_string(),
            "a merged block crosses row 2: unmerge it in the web client first"
        );
    }
}
