//! Resizing kinds the way the web client does: by `scale`, by `textScale`.
#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, edit, fields_of, frame::seed};
use crate::{
    domain::{
        change::{
            build::build,
            value::{ChangeSources, ObjectPatch},
        },
        geometry::Size,
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::Result,
};

fn apply(document: &Doc, patch: ObjectPatch) -> Result<()> {
    let ids = SequentialIds::new("e");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(2),
        ids: &ids,
    };
    build(document, &edit("o".into(), patch), ME, &context).map(drop)
}

fn resized(object: &Value, size: (f64, f64)) -> Result<Doc> {
    let document = Doc::new();
    seed(&document, object)?;
    apply(
        &document,
        ObjectPatch {
            size: Some(Size::new(size.0, size.1)?),
            ..ObjectPatch::default()
        },
    )?;
    Ok(document)
}

fn number(document: &Doc, name: &str) -> f64 {
    fields_of(document, "o")
        .get(name)
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

#[rstest]
#[case::dice_by_height(json!({"id":"o","type":"dice","scale":1}), (480.0, 480.0), 2.0)]
#[case::dice_drawn_at_half_unset(json!({"id":"o","type":"dice"}), (240.0, 240.0), 1.0)]
#[case::wheel_drawn_at_one_and_a_half_unset(json!({"id":"o","type":"spinner-wheel"}), (720.0, 720.0), 2.0)]
#[case::link_by_width(json!({"id":"o","type":"link","scale":1}), (600.0, 440.0), 2.0)]
#[case::task_by_width(json!({"id":"o","type":"task-card","scale":2}), (150.0, 60.0), 0.5)]
#[case::file_by_height(json!({"id":"o","type":"file"}), (240.0, 128.0), 2.0)]
#[case::flip_card_by_height(json!({"id":"o","type":"flip-card","scale":1}), (110.0, 160.0), 0.5)]
#[case::code_by_height(
    json!({"id":"o","type":"code","width":500,"height":200,"scale":1}),
    (250.0, 100.0),
    0.5
)]
#[case::drawing_by_width_to_3_places(
    json!({"id":"o","type":"drawing","path":"0,0,100,50","scale":1}),
    (33.3333, 16.0),
    0.333
)]
fn a_scaled_kind_takes_the_scale_that_draws_it_at_the_size(
    #[case] object: Value,
    #[case] size: (f64, f64),
    #[case] scale: f64,
) -> Result<()> {
    let document = resized(&object, size)?;
    assert!((number(&document, "scale") - scale).abs() < 1e-9);
    Ok(())
}

#[test]
fn a_scaled_kind_keeps_its_corner_and_stored_size() -> Result<()> {
    let object = json!({
        "id": "o", "type": "dice", "scale": 1, "position": { "x": 5, "y": 7 },
    });
    let document = resized(&object, (480.0, 480.0))?;
    let fields = fields_of(&document, "o");
    assert_eq!(fields.get("position"), Some(&json!({ "x": 5, "y": 7 })));
    assert_eq!(fields.get("width"), None);
    Ok(())
}

#[rstest]
#[case::kanban(json!({"id":"o","type":"kanban"}), "scale")]
#[case::link(json!({"id":"o","type":"link"}), "scale")]
#[case::sticker(json!({"id":"o","type":"sticker"}), "textScale")]
#[case::text(json!({"id":"o","type":"simple-text"}), "textScale")]
fn a_scale_is_written_as_given(
    #[case] object: Value,
    #[case] name: &str,
) -> Result<()> {
    let document = Doc::new();
    seed(&document, &object)?;
    apply(
        &document,
        ObjectPatch {
            scale: Some(2.5),
            ..ObjectPatch::default()
        },
    )?;
    assert!((number(&document, name) - 2.5).abs() < 1e-9);
    Ok(())
}

#[rstest]
#[case::kanban_size(json!({"id":"o","type":"kanban"}), Some((100.0, 100.0)), None, "kanban objects have no size to change")]
#[case::arrow_size(json!({"id":"o","type":"arrow"}), Some((100.0, 100.0)), None, "arrow objects have no size to change")]
#[case::shape_scale(json!({"id":"o","type":"shape"}), None, Some(2.0), "shape objects have no scale to change")]
#[case::arrow_scale(json!({"id":"o","type":"arrow"}), None, Some(2.0), "arrow objects have no scale to change")]
#[case::card_in_a_kanban(
    json!({"id":"o","type":"task-card","columnId":"c","parentId":"k"}),
    Some((100.0, 100.0)),
    None,
    "task-card in a kanban objects have no size to change"
)]
fn a_kind_the_web_does_not_resize_that_way_is_refused_by_name(
    #[case] object: Value,
    #[case] size: Option<(f64, f64)>,
    #[case] scale: Option<f64>,
    #[case] said: &str,
) -> Result<()> {
    let document = Doc::new();
    seed(&document, &object)?;
    let refused = apply(
        &document,
        ObjectPatch {
            size: size.map(|(w, h)| Size::new(w, h)).transpose()?,
            scale,
            ..ObjectPatch::default()
        },
    )
    .err()
    .map(|error| error.to_string());
    assert_eq!(refused.as_deref(), Some(said));
    Ok(())
}

fn table() -> Value {
    json!({
        "id": "o", "type": "table", "position": { "x": 0, "y": 0 },
        "row-a": { "index": 0, "height": 60 }, "row-b": { "index": 1, "height": 60 },
        "column-x": { "index": 0, "width": 240 }, "column-y": { "index": 1, "width": 240 },
    })
}

#[rstest]
#[case::grown((600.0, 200.0), 300.0, 100.0)]
#[case::shrunk_to_the_least((100.0, 10.0), 50.0, 32.0)]
fn a_table_spreads_the_change_over_its_rows_and_columns(
    #[case] size: (f64, f64),
    #[case] column: f64,
    #[case] row: f64,
) -> Result<()> {
    let document = resized(&table(), size)?;
    let fields = fields_of(&document, "o");
    let side = |track: &str, name: &str| {
        fields
            .get(track)
            .and_then(|found| found.get(name))
            .and_then(Value::as_f64)
            .unwrap_or_default()
    };
    let found = [
        side("column-x", "width"),
        side("column-y", "width"),
        side("row-a", "height"),
        side("row-b", "height"),
        side("row-b", "minHeight"),
        side("row-b", "index"),
    ];
    let wanted = [column, column, row, row, row, 1.0];
    assert!(
        found
            .iter()
            .zip(wanted)
            .all(|(have, want)| (have - want).abs() < 1e-9),
        "{found:?} against {wanted:?}"
    );
    assert_eq!(fields.get("width"), None);
    Ok(())
}

fn member(id: &str, parent: Option<&str>, at: (u32, u32)) -> Value {
    json!({
        "id": id, "type": "sticker", "parentId": parent,
        "position": { "x": at.0, "y": at.1 }, "width": 20, "height": 20,
    })
}

fn parent_of(document: &Doc, id: &str) -> Option<String> {
    fields_of(document, id)
        .get("parentId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn a_frame_resized_releases_what_it_no_longer_meets_and_takes_in_what_it_holds()
-> Result<()> {
    let document = Doc::new();
    seed(
        &document,
        &json!({
            "id": "o", "type": "frame", "position": { "x": 0, "y": 0 },
            "width": 100, "height": 100,
        }),
    )?;
    seed(&document, &member("kept", Some("o"), (10, 10)))?;
    seed(&document, &member("gone", Some("o"), (90, 90)))?;
    seed(&document, &member("near", None, (200, 10)))?;
    apply(
        &document,
        ObjectPatch {
            size: Some(Size::new(50.0, 50.0)?),
            ..ObjectPatch::default()
        },
    )?;
    assert_eq!(parent_of(&document, "kept").as_deref(), Some("o"));
    assert_eq!(parent_of(&document, "gone"), None);
    apply(
        &document,
        ObjectPatch {
            size: Some(Size::new(300.0, 100.0)?),
            ..ObjectPatch::default()
        },
    )?;
    assert_eq!(parent_of(&document, "near").as_deref(), Some("o"));
    Ok(())
}
