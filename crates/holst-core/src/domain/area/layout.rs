//! The cells of a table and the columns of a kanban, laid out for drawing.

use std::collections::{BTreeMap, HashMap};

use itertools::Itertools;
use serde_json::Value;

use crate::{
    domain::{
        area::scene::{number, track_word},
        board::{KANBAN_COLUMNS, KANBAN_SWIMLANES, Object, ObjectKind},
        geometry::{Rectangle, StoredSize},
        kinds::kanban::{CARD_WIDTH, COLUMN_WIDTH},
    },
    protocol::document,
};

const MERGE_PREFIX: &str = "merge-";

/// One row or column of a table: its stored number and its length.
type Line = (String, f64);

fn lines(fields: &Value, prefix: &str, default: f64, side: &str) -> Vec<Line> {
    fields
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, track)| {
            let id = key.strip_prefix(prefix)?;
            Some((
                number(track, document::INDEX)?,
                id.to_owned(),
                number(track, side).unwrap_or(default),
            ))
        })
        .sorted_by(|(first, id, _), (second, other, _)| {
            first.total_cmp(second).then_with(|| id.cmp(other))
        })
        .map(|(_, id, length)| (id, length))
        .collect()
}

/// A merged block: its leader cell and how many rows and columns it covers.
#[derive(Debug, Clone, Copy)]
struct Merge {
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
}

impl Merge {
    const fn covers(self, row: usize, column: usize) -> bool {
        row >= self.row
            && row < self.row + self.rows
            && column >= self.column
            && column < self.column + self.columns
    }
}

/// A table's rows, columns and merged blocks in the order they are drawn.
#[derive(Debug)]
pub(crate) struct Grid {
    rows: Vec<Line>,
    columns: Vec<Line>,
    merges: Vec<Merge>,
}

impl Grid {
    pub(crate) fn of(fields: &Value) -> Self {
        let rows = lines(fields, document::ROW_PREFIX, 60.0, document::HEIGHT);
        let columns = lines(fields, document::COLUMN_PREFIX, 240.0, document::WIDTH);
        let place =
            |lines: &[Line], id: &str| lines.iter().position(|(own, _)| own == id);
        let merges = fields
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, stored)| {
                let (row, column) = key.strip_prefix(MERGE_PREFIX)?.split_once('-')?;
                let leader = |name: &str, fallback: &str| {
                    track_word(stored.get(name)).unwrap_or_else(|| fallback.to_owned())
                };
                let count = |name: &str| {
                    track_word(stored.get(name))
                        .and_then(|word| word.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1)
                };
                Some(Merge {
                    row: place(&rows, &leader("row", row))?,
                    column: place(&columns, &leader("column", column))?,
                    rows: count("countRow"),
                    columns: count("countColumn"),
                })
            })
            .filter(|merge| merge.rows > 1 || merge.columns > 1)
            .collect();
        Self {
            rows,
            columns,
            merges,
        }
    }

    /// The width and the height of the whole table.
    pub(crate) fn size(&self) -> StoredSize {
        let total =
            |lines: &[Line]| lines.iter().map(|(_, length)| length).sum::<f64>();
        StoredSize::new(total(&self.columns), total(&self.rows))
    }

    /// The row and the column index a cell stands in.
    pub(crate) fn place(&self, cell: &Object) -> Option<(usize, usize)> {
        let find = |lines: &[Line], field: &str| {
            let id = track_word(cell.fields.get(field))?;
            lines.iter().position(|(own, _)| *own == id)
        };
        Some((
            find(&self.rows, document::ROW_ID)?,
            find(&self.columns, document::COLUMN_ID)?,
        ))
    }

    /// Whether a merged block led by another cell holds this one.
    pub(crate) fn is_covered(&self, row: usize, column: usize) -> bool {
        self.merges.iter().any(|merge| {
            merge.covers(row, column) && (merge.row, merge.column) != (row, column)
        })
    }

    /// The place a cell takes inside the table; a block's leader takes the
    /// whole block.
    pub(crate) fn rectangle(&self, row: usize, column: usize) -> Rectangle {
        let (rows, columns) = self
            .merges
            .iter()
            .find(|merge| (merge.row, merge.column) == (row, column))
            .map_or((1, 1), |merge| (merge.rows, merge.columns));
        let before = |lines: &[Line], at: usize| {
            lines.iter().take(at).map(|(_, length)| length).sum::<f64>()
        };
        let across = |lines: &[Line], at: usize, count: usize| {
            lines
                .iter()
                .skip(at)
                .take(count)
                .map(|(_, length)| length)
                .sum::<f64>()
        };
        Rectangle::new(
            before(&self.columns, column),
            before(&self.rows, row),
            across(&self.columns, column, columns),
            across(&self.rows, row, rows),
        )
    }

    /// The outline of every place of the table, a merged block as one.
    pub(crate) fn blocks(&self) -> Vec<Rectangle> {
        (0..self.rows.len())
            .cartesian_product(0..self.columns.len())
            .filter(|&(row, column)| !self.is_covered(row, column))
            .map(|(row, column)| self.rectangle(row, column))
            .collect()
    }
}

/// The height of the strip with the column titles.
const HEADER: f64 = 48.0;
/// The height of the strip with a swimlane title.
const LANE_BAR: f64 = 32.0;
/// The room around a card and between two.
const GAP: f64 = 8.0;
/// The height of a card with none stored.
const CARD_HEIGHT: f64 = 120.0;

/// A swimlane's strip: its title and where it stands in the kanban.
#[derive(Debug)]
pub(crate) struct Lane {
    pub title: String,
    pub top: f64,
}

/// Kanban columns, swimlanes and cell cards, in kanban coordinates.
#[derive(Debug)]
pub(crate) struct Kanban {
    pub columns: Vec<String>,
    pub lanes: Vec<Lane>,
    pub width: f64,
    pub height: f64,
    cards: HashMap<String, Rectangle>,
}

impl Kanban {
    /// The layout of `kanban`, `None` when it lists no column.
    pub(crate) fn of(kanban: &Object, objects: &[Object]) -> Option<Self> {
        let named = |key: &str| -> Vec<(String, String)> {
            kanban
                .fields
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|cell| {
                    Some((
                        cell.get("id")?.as_str()?.to_owned(),
                        cell.get("title")?.as_str()?.to_owned(),
                    ))
                })
                .collect()
        };
        let columns =
            Some(named(KANBAN_COLUMNS)).filter(|columns| !columns.is_empty())?;
        let lanes = named(KANBAN_SWIMLANES);
        let cell_of = |card: &Object| {
            let at = |cells: &[(String, String)], field: &str| {
                card.fields
                    .get(field)
                    .and_then(Value::as_str)
                    .and_then(|id| cells.iter().position(|(own, _)| own == id))
                    .unwrap_or_default()
            };
            (at(&lanes, "swimlaneId"), at(&columns, "columnId"))
        };
        let by_cell: Cells<'_> = objects
            .iter()
            .filter(|card| {
                card.kind == ObjectKind::Task
                    && card.parent.as_deref() == Some(kanban.id.as_str())
            })
            .sorted_by(|a, b| {
                let index = |card: &Object| {
                    number(&card.fields, document::INDEX).unwrap_or_default()
                };
                index(a).total_cmp(&index(b)).then_with(|| a.id.cmp(&b.id))
            })
            .fold(BTreeMap::new(), |mut cells, card| {
                cells.entry(cell_of(card)).or_default().push(card);
                cells
            });
        let lane_heights = lane_heights(&by_cell, lanes.len().max(1), columns.len());
        let tops: Vec<f64> = lane_heights
            .iter()
            .scan(HEADER, |top, height| {
                let at = *top;
                *top += height;
                Some(at)
            })
            .collect();
        let cards = placed_cards(&by_cell, &tops);
        let titles = lanes
            .iter()
            .map(|(_, title)| title.clone())
            .chain(std::iter::repeat(String::new()));
        Some(Self {
            width: f64::from(u32::try_from(columns.len()).unwrap_or_default())
                * COLUMN_WIDTH,
            height: HEADER + lane_heights.iter().sum::<f64>(),
            lanes: titles
                .zip(tops.iter())
                .map(|(title, &top)| Lane { title, top })
                .collect(),
            columns: columns.into_iter().map(|(_, title)| title).collect(),
            cards,
        })
    }

    /// The place of a card inside the kanban.
    pub(crate) fn card(&self, id: &str) -> Option<Rectangle> {
        self.cards.get(id).copied()
    }
}

type Cells<'a> = BTreeMap<(usize, usize), Vec<&'a Object>>;

fn card_height(card: &Object) -> f64 {
    card.height.unwrap_or(CARD_HEIGHT)
}

/// The height of each swimlane: its tallest cell under the strip with its
/// title.
fn lane_heights(by_cell: &Cells<'_>, lanes: usize, columns: usize) -> Vec<f64> {
    (0..lanes)
        .map(|lane| {
            (0..columns)
                .map(|column| {
                    by_cell
                        .get(&(lane, column))
                        .into_iter()
                        .flatten()
                        .map(|card| card_height(card) + GAP)
                        .sum::<f64>()
                        + GAP
                })
                .fold(CARD_HEIGHT + 2.0 * GAP, f64::max)
                + LANE_BAR
        })
        .collect()
}

/// The place of each card, stacked down its cell, by the card's id.
fn placed_cards(by_cell: &Cells<'_>, tops: &[f64]) -> HashMap<String, Rectangle> {
    by_cell
        .iter()
        .flat_map(|(&(lane, column), cards)| {
            let top = tops.get(lane).copied().unwrap_or(HEADER) + LANE_BAR + GAP;
            let left = f64::from(u32::try_from(column).unwrap_or_default())
                * COLUMN_WIDTH
                + (COLUMN_WIDTH - CARD_WIDTH) / 2.0;
            cards.iter().scan(top, move |y, card| {
                let rectangle = Rectangle::new(
                    left,
                    *y,
                    card.width.unwrap_or(CARD_WIDTH),
                    card_height(card),
                );
                *y += rectangle.height + GAP;
                Some((card.id.clone(), rectangle))
            })
        })
        .collect()
}

/// The facing sides of two `(low, high)` spans along one axis, as (start, end).
fn facing(forward: bool, from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    if forward {
        (from.1, to.0)
    } else {
        (from.0, to.1)
    }
}

/// The strip between a mind-map node and the node it hangs from, as a curve
/// from the nearest side of one to the nearest side of the other.
pub(crate) fn connector(from: &Rectangle, to: &Rectangle) -> [(f64, f64); 4] {
    let centre = |r: &Rectangle| (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let ((from_x, from_y), (to_x, to_y)) = (centre(from), centre(to));
    let (across, down) = (to_x - from_x, to_y - from_y);
    let (left, right) =
        facing(across >= 0.0, (from.x, from.right()), (to.x, to.right()));
    let (top, bottom) =
        facing(down >= 0.0, (from.y, from.bottom()), (to.y, to.bottom()));
    let (bend_x, bend_y) = (f64::midpoint(left, right), f64::midpoint(top, bottom));
    let sideways = [
        (left, from_y),
        (bend_x, from_y),
        (bend_x, to_y),
        (right, to_y),
    ];
    let lengthways = [
        (from_x, top),
        (from_x, bend_y),
        (to_x, bend_y),
        (to_x, bottom),
    ];
    if across.abs() >= down.abs() {
        sideways
    } else {
        lengthways
    }
}
