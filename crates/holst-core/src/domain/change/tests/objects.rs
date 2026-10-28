#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::{Doc, Map as _, ReadTxn as _, Transact as _};

use super::{
    ME,
    add,
    add_in,
    edit,
    fields_of,
    frame::{free, hidden_frame, move_to, nested_frames, seed},
    remove,
};
use crate::{
    domain::{
        change::{
            build::build,
            value::{
                ChangeSources,
                ChangeUpdate,
                NewObject,
                ObjectChangeKind,
                ObjectPatch,
            },
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

const LONG: &str = "dbc79d73-0000-4000-8000-000000000001";

fn built(
    document: &Doc,
    ids: &SequentialIds,
    change: &crate::domain::change::value::BoardChange,
) -> Result<ChangeUpdate> {
    build(
        document,
        change,
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(2),
            ids,
        },
    )
}

fn stamp_on(object: &str, x: u32, y: u32) -> Value {
    json!({
        "id": "st", "type": "stamp", "position": { "x": x, "y": y },
        "width": 60, "height": 60, "stickyPosition": { "parentId": object },
    })
}

fn count(document: &Doc) -> u32 {
    let transaction = document.transact();
    transaction
        .get_map("objects")
        .map_or(0, |objects| objects.len(&transaction))
}

#[test]
fn an_object_is_named_by_at_and_the_start_of_its_id() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &free(LONG, 900, 0))?;
    move_to(&document, &ids, "@dbc79d73", Point::new(950.0, 10.0)?)?;
    assert_eq!(
        fields_of(&document, LONG).get("position"),
        Some(&json!({ "x": 950, "y": 10 }))
    );
    Ok(())
}

#[test]
fn a_new_object_is_refused_in_a_frame_that_hides_its_content() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &hidden_frame())?;
    let before = count(&document);
    let new = NewObject::Sticker {
        text: "n".into(),
        at: Point::new(10.0, 10.0)?,
        size: None,
        color: None,
    };
    let refused = built(&document, &ids, &add_in(new, "h".into()));
    assert!(matches!(
        &refused,
        Err(Error::BoardChange(BoardChangeError::ContentHidden { .. }))
    ));
    assert!(
        refused
            .err()
            .is_some_and(|error| error.to_string().contains("\"h\""))
    );
    assert_eq!(count(&document), before);
    Ok(())
}

#[test]
fn a_stamp_stuck_to_an_object_moves_with_it() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &free("s", 900, 0))?;
    seed(&document, &stamp_on("s", 930, 10))?;
    let update = move_to(&document, &ids, "s", Point::new(1000.0, 50.0)?)?;
    let stamp = fields_of(&document, "st");
    assert_eq!(stamp.get("position"), Some(&json!({ "x": 1030, "y": 60 })));
    assert_eq!(
        stamp.get("stickyPosition"),
        Some(&json!({ "parentId": "s" }))
    );
    assert!(update.objects.iter().any(|object| object.id == "st"));
    Ok(())
}

#[test]
fn a_stamp_stuck_to_another_object_stays() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &free("s", 900, 0))?;
    seed(&document, &free("o", 900, 300))?;
    seed(&document, &stamp_on("o", 930, 310))?;
    move_to(&document, &ids, "s", Point::new(1000.0, 50.0)?)?;
    assert_eq!(
        fields_of(&document, "st").get("position"),
        Some(&json!({ "x": 930, "y": 310 }))
    );
    Ok(())
}

#[test]
fn removing_an_object_removes_the_stamps_stuck_to_it() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &free("s", 900, 0))?;
    seed(&document, &stamp_on("s", 930, 10))?;
    let update = built(&document, &ids, &remove("s".into()))?;
    let gone: Vec<&str> = update
        .objects
        .iter()
        .filter(|object| object.change == ObjectChangeKind::Removed)
        .map(|object| object.id.as_str())
        .collect();
    assert!(gone.contains(&"s") && gone.contains(&"st"), "{gone:?}");
    Ok(())
}

fn resized(document: &Doc, ids: &SequentialIds, id: &str, size: Size) -> Result<Value> {
    let patch = ObjectPatch {
        size: Some(size),
        ..ObjectPatch::default()
    };
    built(document, ids, &edit(id.into(), patch))?;
    Ok(fields_of(document, id))
}

fn number(fields: &Value, name: &str) -> Option<f64> {
    fields.get(name).and_then(Value::as_f64)
}

#[test]
fn a_sticker_grows_by_text_scale_and_snaps_its_width() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(
        &document,
        &json!({
            "id": "k", "type": "sticker", "position": { "x": 900, "y": 0 },
            "width": 192, "height": 192, "textScale": 1,
        }),
    )?;
    let big = resized(&document, &ids, "k", Size::new(384.0, 384.0)?)?;
    assert_eq!(number(&big, "textScale"), Some(2.0));
    assert_eq!(number(&big, "width"), Some(192.0));
    assert_eq!(number(&big, "height"), Some(192.0));
    let wide = resized(&document, &ids, "k", Size::new(576.0, 288.0)?)?;
    assert_eq!(number(&wide, "textScale"), Some(1.5));
    assert_eq!(number(&wide, "width"), Some(384.0));
    Ok(())
}

#[test]
fn a_sticker_stack_grows_by_text_scale() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(
        &document,
        &json!({
            "id": "k", "type": "sticker-stack", "position": { "x": 900, "y": 0 },
            "width": 192, "height": 192,
        }),
    )?;
    let grown = resized(&document, &ids, "k", Size::new(96.0, 96.0)?)?;
    assert_eq!(number(&grown, "textScale"), Some(0.5));
    assert_eq!(number(&grown, "width"), Some(192.0));
    Ok(())
}

#[test]
fn a_text_grows_by_text_scale() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let text = NewObject::Text {
        text: "hello".into(),
        at: Point::new(900.0, 0.0)?,
    };
    let made = built(&document, &ids, &add(text))?;
    let id = made
        .objects
        .first()
        .map(|object| object.id.clone())
        .unwrap_or_default();
    let grown = resized(&document, &ids, &id, Size::new(300.0, 42.0)?)?;
    assert_eq!(number(&grown, "textScale"), Some(2.0));
    assert_eq!(number(&grown, "width"), None);
    assert_eq!(number(&grown, "height"), None);
    Ok(())
}
