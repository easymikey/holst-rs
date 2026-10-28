#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, add, board as seeded, edit, fields_of};
use crate::{
    domain::{
        board::shape_type::{ShapeType, StrokeStyle},
        change::{
            build::build,
            value::{BoardChange, ChangeSources, NewObject, ObjectPatch, ShapeEdit},
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn apply(document: &Doc, change: &BoardChange) -> Result<Vec<String>> {
    let ids = SequentialIds::new("e");
    let built = build(
        document,
        change,
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(9),
            ids: &ids,
        },
    )?;
    Ok(built.objects.into_iter().map(|object| object.id).collect())
}

/// A `200x120` shape at (100, 100) on the seeded board; its id.
fn shape(document: &Doc) -> Result<String> {
    let made = apply(
        document,
        &add(NewObject::Shape {
            text: "s".into(),
            at: Point::new(100.0, 100.0)?,
            size: Some(Size::new(200.0, 120.0)?),
            color: None,
            kind: ShapeType::Square,
        }),
    )?;
    Ok(made.into_iter().next().unwrap_or_default())
}

fn shape_edit(shape: ShapeEdit) -> ObjectPatch {
    ObjectPatch {
        shape,
        ..ObjectPatch::default()
    }
}

fn at(fields: &Value, pointer: &str) -> Value {
    fields.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn number(fields: &Value, name: &str) -> f64 {
    fields.get(name).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

#[test]
fn a_shape_changes_its_kind_and_stays_fixed_in_size() -> Result<()> {
    let document = seeded()?;
    let id = shape(&document)?;
    apply(
        &document,
        &edit(
            id.clone(),
            shape_edit(ShapeEdit {
                kind: Some(ShapeType::FlowchartDecision),
                ..ShapeEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        (at(&fields, "/shapeType"), at(&fields, "/fixedSize")),
        (json!("flowchart-decision"), json!(true))
    );
    Ok(())
}

#[test]
fn a_bpmn_kind_clears_the_markers_and_keeps_the_centre() -> Result<()> {
    let document = seeded()?;
    let id = shape(&document)?;
    apply(
        &document,
        &edit(
            id.clone(),
            shape_edit(ShapeEdit {
                kind: Some(ShapeType::BpmnGateway),
                ..ShapeEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    // 144x144 to 48x48 is a third, and the shape is 120 high: 120 / 144 * 48.
    let side = 120.0 / 144.0 * 48.0;
    let position = at(&fields, "/position");
    let centre = (
        number(&position, "x") + number(&fields, "width") / 2.0,
        number(&position, "y") + number(&fields, "height") / 2.0,
    );
    assert_eq!(
        (
            at(&fields, "/shapeType"),
            at(&fields, "/bpmnResources"),
            number(&fields, "width"),
            number(&fields, "height"),
            centre,
        ),
        (json!("bpmn-gateway"), json!({}), side, side, (200.0, 160.0))
    );
    Ok(())
}

#[test]
fn the_outline_corners_and_fill_opacity_are_written_and_the_radius_clamped()
-> Result<()> {
    let document = seeded()?;
    let id = shape(&document)?;
    apply(
        &document,
        &edit(
            id.clone(),
            shape_edit(ShapeEdit {
                stroke: Some("blue4".parse()?),
                stroke_width: Some(4.0),
                stroke_style: Some(StrokeStyle::Dashed),
                radius: Some(500),
                opacity: Some(0.5),
                ..ShapeEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        (
            at(&fields, "/strokeColor"),
            at(&fields, "/strokeWidth"),
            at(&fields, "/strokeStyle"),
            at(&fields, "/borderRadius"),
            at(&fields, "/fillColor/opacity"),
            at(&fields, "/fillColor/color"),
        ),
        (
            json!({"color": "blue4", "opacity": 1}),
            json!(4),
            json!("dashed"),
            // A square rounds up to half of its shorter side.
            json!(60),
            json!(0.5),
            json!("white6"),
        )
    );
    Ok(())
}

#[test]
fn only_a_shape_has_an_outline() -> Result<()> {
    let document = seeded()?;
    let refused = apply(
        &document,
        &edit(
            "seed-2".into(),
            shape_edit(ShapeEdit {
                stroke_width: Some(3.0),
                ..ShapeEdit::default()
            }),
        ),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "stroke-width",
            ..
        }))
    ));
    Ok(())
}
