//! Objects aligned and spread: each moves with what it holds.

use std::collections::HashMap;

use serde_json::{Value, json};
use yrs::TransactionMut;

use crate::{
    domain::{
        area::scene::Scene,
        board::{Object, ObjectKind, object_with_text},
        change::{
            arrange::{aligned, distributed},
            check::{kind_of, parent_frame, parent_of, subtree},
            value::{Arrangement, ChangedObject},
            write::{
                WriteTransaction,
                insert_changes,
                moving_with,
                object_map,
                shift_objects,
                stored_now,
            },
        },
        geometry::Rectangle,
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::PARENT_ID,
};

/// Moves the objects against each other as the web client's align and
/// distribute buttons do: each with what it holds, then into the frame it now
/// lies in.
///
/// # Errors
/// Too few objects with a place to arrange, and the write.
// BUNDLE: main.a33651a4.js 209337-209358, 209416-209460.
pub(super) fn arrange(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    ids: &[String],
    how: Arrangement,
) -> Result<Vec<ChangedObject>> {
    let objects: Vec<Object> = scope
        .states
        .values()
        .filter_map(|fields| {
            object_with_text(fields.clone(), &*transaction, &scope.roots.documents)
        })
        .collect();
    let scene = Scene::new(&objects);
    let placed: Vec<(&String, Rectangle)> = ids
        .iter()
        .filter_map(|id| {
            let object = scene.get(id)?;
            Some((id, scene.bounds(object, 0)?))
        })
        .collect();
    let rectangles: Vec<Rectangle> = placed.iter().map(|(_, own)| *own).collect();
    let (needs, shifts) = match how {
        Arrangement::Align(to) => {
            (2, (rectangles.len() >= 2).then(|| aligned(&rectangles, to)))
        }
        Arrangement::Distribute(axis) => (3, distributed(&rectangles, axis)),
    };
    let shifts =
        shifts.ok_or(Error::BoardChange(BoardChangeError::TooFewToArrange {
            needs,
            given: rectangles.len(),
        }))?;
    let steps: Vec<(&String, (f64, f64), Rectangle)> = placed
        .iter()
        .zip(shifts)
        .filter(|(_, (across, down))| across.abs() > 0.0 || down.abs() > 0.0)
        .map(|((id, own), shift)| (*id, shift, *own))
        .collect();
    let moved: Vec<ChangedObject> = steps
        .iter()
        .map(|(id, shift, _)| {
            shift_objects(
                transaction,
                scope,
                &moving_with(scope.states, id),
                None,
                *shift,
            )
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let after: HashMap<String, Value> =
        moved
            .iter()
            .fold(scope.states.clone(), |mut states, changed| {
                if let Some(fields) = stored_now(&*transaction, scope, &changed.id) {
                    states.insert(changed.id.clone(), fields);
                }
                states
            });
    let framed: Vec<(&String, Option<String>)> = steps
        .iter()
        .filter_map(|(id, (across, down), own)| {
            let fields = after.get(*id)?;
            let there =
                Rectangle::new(own.x + across, own.y + down, own.width, own.height);
            let frame = parent_frame(&after, &there, &subtree(&after, id), None);
            let free = parent_of(fields)
                .is_none_or(|parent| kind_of(&after, parent) == ObjectKind::Frame);
            (free && frame.as_deref() != parent_of(fields)).then_some((*id, frame))
        })
        .collect();
    framed.iter().try_for_each(|(id, frame)| {
        let map =
            object_map(transaction, &scope.roots.objects, id, &kind_of(&after, id))?;
        insert_changes(transaction, &map, &[(PARENT_ID, json!(frame))])
    })?;
    Ok(moved
        .into_iter()
        .map(|changed| {
            let reparented = framed.iter().any(|(id, _)| **id == changed.id);
            ChangedObject {
                fields: changed
                    .fields
                    .iter()
                    .copied()
                    .chain(reparented.then_some("parent"))
                    .collect(),
                ..changed
            }
        })
        .collect())
}
