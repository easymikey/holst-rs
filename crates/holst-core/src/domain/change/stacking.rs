//! Where an edit puts an object among the others: an end, or one step.

use std::collections::HashMap;

use itertools::Itertools as _;
use serde_json::Value;

use crate::{
    domain::{
        board::ObjectKind,
        change::{
            check::{CheckedEdit, kind_of, parent_of, rectangle_of},
            value::LayerOrder,
            write::{Edge, LAYER_STEP, beyond, jitter},
        },
        geometry::{Rectangle, StoredPoint},
    },
    protocol::document::{END, IGNORE_Z_INDEX, POINT, START, TYPE, X, Y, Z_INDEX},
};

/// The layer an edit gives its object: the end laid for it by [`ends`], or the
/// step; `None` when an object at the end of its overlaps has nowhere to go.
pub(super) fn layer(
    states: &HashMap<String, Value>,
    id: &str,
    order: LayerOrder,
    ends: &HashMap<String, f64>,
) -> Option<f64> {
    match order {
        LayerOrder::Front | LayerOrder::Back => ends.get(id).copied(),
        LayerOrder::Forward => step(states, id, true),
        LayerOrder::Backward => step(states, id, false),
    }
}

/// The layers for the objects brought to the front or sent to the back, each
/// end past every other object, the objects keeping their order among
/// themselves. BUNDLE: main.a33651a4.js 129578-129597.
pub(super) fn ends(
    states: &HashMap<String, Value>,
    edits: &[CheckedEdit<'_>],
) -> HashMap<String, f64> {
    [
        (LayerOrder::Front, Edge::Front),
        (LayerOrder::Back, Edge::Back),
    ]
    .into_iter()
    .flat_map(|(order, edge)| {
        let ranked: Vec<&str> = edits
            .iter()
            .filter(|checked| checked.edit.stacking == Some(order))
            .map(|checked| checked.id.as_str())
            .unique()
            .sorted_by(|a, b| layer_of(states, a).total_cmp(&layer_of(states, b)))
            .collect();
        laid(states, &ranked, edge)
    })
    .collect()
}

/// The ranked objects one layer apart, the first past every other object.
fn laid(
    states: &HashMap<String, Value>,
    ranked: &[&str],
    edge: Edge,
) -> Vec<(String, f64)> {
    let base = ranked
        .first()
        .map_or(0.0, |first| beyond(states, first, edge));
    match edge {
        Edge::Front => ranked
            .iter()
            .zip(0_u32..)
            .map(|(id, step)| ((*id).to_owned(), base + f64::from(step)))
            .collect(),
        Edge::Back => ranked
            .iter()
            .rev()
            .zip(0_u32..)
            .map(|(id, step)| ((*id).to_owned(), base - f64::from(step)))
            .collect(),
    }
}

/// One step past the next object that overlaps, among the objects of the same
/// parent: between that object and the one after it. BUNDLE: the web client
/// does the same for a selection; here it is one object, its box taken as
/// stored (main.a33651a4.js 129607-129651).
fn step(states: &HashMap<String, Value>, id: &str, forward: bool) -> Option<f64> {
    let own = states.get(id)?;
    let area = bounds(own)?;
    let parent = parent_of(own);
    let ignored = parent
        .and_then(|parent| states.get(parent))
        .is_some_and(ignores_layers);
    (!ignored).then_some(())?;
    let mut near: Vec<f64> = states
        .iter()
        .filter(|(other, fields)| {
            other.as_str() != id
                && parent_of(fields) == parent
                && kind_of(states, other) != ObjectKind::TableCell
                && !ignores_layers(fields)
                && bounds(fields).is_some_and(|there| there.intersects(&area))
        })
        .map(|(_, fields)| z_of(fields))
        .collect();
    near.sort_by(f64::total_cmp);
    if !forward {
        near.reverse();
    }
    let held = z_of(own);
    let at = near
        .iter()
        .position(|&z| if forward { z >= held } else { z <= held })?;
    let first = near.get(at).copied()?;
    let lift = LAYER_STEP + LAYER_STEP * jitter(id);
    let second = near.get(at + 1).copied().unwrap_or(if forward {
        first + lift
    } else {
        first - lift
    });
    let (low, high) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    Some(low + (high - low) / 3.0)
}

/// The layer of an object put last among the objects of `parent`, the board
/// for none: past the last by the web client's `zH(last, null)`, which adds
/// 0x10000000000 and a random up to 10 000. BUNDLE: main.a33651a4.js
/// 223458-223472; the last of the others as in 128.8f630629.js 2125-2133.
pub(super) fn appended(
    states: &HashMap<String, Value>,
    id: &str,
    parent: Option<&str>,
) -> f64 {
    let last = states
        .iter()
        .filter(|(other, fields)| {
            other.as_str() != id
                && parent_of(fields) == parent
                && kind_of(states, other) != ObjectKind::TableCell
        })
        .map(|(_, fields)| z_of(fields))
        .max_by(f64::total_cmp);
    last.unwrap_or_default() + 1_099_511_627_776.0 + 10_000.0 * jitter(id)
}

/// A group whose members ignore the layers of the board.
fn ignores_layers(fields: &Value) -> bool {
    fields.get(IGNORE_Z_INDEX).and_then(Value::as_bool) == Some(true)
}

fn layer_of(states: &HashMap<String, Value>, id: &str) -> f64 {
    states.get(id).map_or(0.0, z_of)
}

fn z_of(fields: &Value) -> f64 {
    fields
        .get(Z_INDEX)
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

/// The box an object stands in; an arrow's is round its ends.
fn bounds(fields: &Value) -> Option<Rectangle> {
    let kind =
        ObjectKind::of(fields.get(TYPE).and_then(Value::as_str).unwrap_or_default());
    if kind == ObjectKind::Arrow {
        Rectangle::around([START, END].into_iter().filter_map(|side| {
            let point = fields.get(side)?.get(POINT)?;
            Some(StoredPoint::new(
                point.get(X)?.as_f64()?,
                point.get(Y)?.as_f64()?,
            ))
        }))
    } else {
        rectangle_of(fields)
    }
}

#[cfg(test)] mod tests;
