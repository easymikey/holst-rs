#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::collections::HashMap;

use rstest::rstest;
use serde_json::{Value, json};

use super::{edit, remove};
use crate::{
    domain::{
        change::{
            check::check,
            value::{BoardChange, LayerOrder, ObjectEdit, ObjectPatch, RemoveRight},
        },
        geometry::{Point, Size},
    },
    error::{Error, Result, board_change::BoardChangeError},
};

const GROUP: &str = "00000000-0000-4000-8000-000000000001";
const IN_GROUP: &str = "00000000-0000-4000-8000-000000000002";
const LOCKED_FRAME: &str = "00000000-0000-4000-8000-000000000003";
const FRAME: &str = "00000000-0000-4000-8000-000000000004";
const IN_FRAME: &str = "00000000-0000-4000-8000-000000000005";
const INNER_GROUP: &str = "00000000-0000-4000-8000-000000000006";
const LOCKED_TABLE: &str = "00000000-0000-4000-8000-000000000008";
const CELL: &str = "00000000-0000-4000-8000-000000000009";
const ARROW: &str = "00000000-0000-4000-8000-00000000000a";
const FREE: &str = "00000000-0000-4000-8000-00000000000b";
const OTHER: &str = "00000000-0000-4000-8000-00000000000c";
const IN_LOCKED_FRAME: &str = "00000000-0000-4000-8000-00000000000d";

fn board() -> HashMap<String, Value> {
    let objects = [
        json!({"id": GROUP, "type": "group", "locked": true}),
        json!({"id": IN_GROUP, "type": "sticker", "parentId": GROUP}),
        json!({"id": LOCKED_FRAME, "type": "frame", "locked": true}),
        json!({"id": FRAME, "type": "frame"}),
        json!({"id": IN_FRAME, "type": "sticker", "parentId": FRAME}),
        json!({"id": INNER_GROUP, "type": "group", "locked": true, "parentId": FRAME}),
        json!({"id": LOCKED_TABLE, "type": "table", "locked": true}),
        json!({"id": CELL, "type": "table-cell", "parentId": LOCKED_TABLE}),
        json!({"id": ARROW, "type": "arrow", "parentId": GROUP,
               "start": {"objectId": FREE}}),
        json!({"id": FREE, "type": "sticker"}),
        json!({"id": OTHER, "type": "sticker"}),
        json!({"id": IN_LOCKED_FRAME, "type": "sticker", "parentId": LOCKED_FRAME}),
    ];
    objects
        .into_iter()
        .filter_map(|object| {
            let id = object.get("id")?.as_str()?.to_owned();
            Some((id, object))
        })
        .collect()
}

fn refused(change: &BoardChange) -> Option<String> {
    match check(change, &board(), RemoveRight::Anyone, None) {
        Err(Error::BoardChange(BoardChangeError::Locked { reference })) => {
            Some(reference)
        }
        Err(_) | Ok(_) => None,
    }
}

fn allowed(change: &BoardChange) -> bool {
    check(change, &board(), RemoveRight::Anyone, None).is_ok()
}

fn patch_of(field: &str) -> Result<ObjectPatch> {
    let patch = ObjectPatch::default();
    Ok(match field {
        "text" => ObjectPatch {
            text: Some("x".into()),
            ..patch
        },
        "at" => ObjectPatch {
            at: Some(Point::new(1.0, 1.0)?),
            ..patch
        },
        "size" => ObjectPatch {
            size: Some(Size::new(192.0, 192.0)?),
            ..patch
        },
        _ => ObjectPatch {
            color: Some("yellow4".parse()?),
            ..patch
        },
    })
}

#[rstest]
fn an_edit_of_locked_content_is_refused(
    #[values(IN_GROUP, GROUP, LOCKED_FRAME, CELL, LOCKED_TABLE)] object: &str,
    #[values("text", "at", "size", "color")] field: &str,
) -> Result<()> {
    let change = edit(object.into(), patch_of(field)?);
    assert_eq!(refused(&change).as_deref(), Some(object));
    Ok(())
}

#[test]
fn an_edit_of_many_refuses_locked_tags_and_layer() {
    let edit = |object: &str, tags: Option<Vec<String>>, stacking| ObjectEdit {
        object: object.into(),
        patch: ObjectPatch::default(),
        stacking,
        tags,
    };
    let tagged = BoardChange::EditAll(vec![
        edit(OTHER, Some(vec!["a".into()]), None),
        edit(IN_GROUP, Some(vec!["a".into()]), None),
    ]);
    let layered =
        BoardChange::EditAll(vec![edit(GROUP, None, Some(LayerOrder::Front))]);
    assert_eq!(refused(&tagged).as_deref(), Some(IN_GROUP));
    assert_eq!(refused(&layered).as_deref(), Some(GROUP));
}

#[rstest]
#[case::inside_a_locked_group(&[IN_GROUP], IN_GROUP)]
#[case::the_locked_group(&[GROUP], GROUP)]
#[case::a_locked_frame(&[LOCKED_FRAME], LOCKED_FRAME)]
#[case::a_locked_table(&[LOCKED_TABLE], LOCKED_TABLE)]
#[case::a_cell_of_a_locked_table(&[CELL], CELL)]
#[case::a_frame_holding_a_locked_group(&[FRAME], INNER_GROUP)]
#[case::an_arrow_that_goes_along(&[FREE], ARROW)]
#[case::among_free_ones(&[OTHER, IN_GROUP], IN_GROUP)]
fn a_removal_that_reaches_locked_content_is_refused(
    #[case] objects: &[&str],
    #[case] named: &str,
) {
    let change =
        BoardChange::RemoveAll(objects.iter().map(|&one| one.into()).collect());
    assert_eq!(refused(&change).as_deref(), Some(named));
}

#[test]
fn what_no_lock_reaches_is_still_changed() {
    let edit = |object: &str| {
        edit(
            object.into(),
            ObjectPatch {
                at: Some(Point::ORIGIN),
                ..ObjectPatch::default()
            },
        )
    };
    assert!(allowed(&edit(IN_FRAME)));
    assert!(allowed(&edit(IN_LOCKED_FRAME)));
    assert!(allowed(&remove(OTHER.into())));
    assert!(allowed(&remove(IN_LOCKED_FRAME.into())));
}
