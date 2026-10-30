//! A drawing takes a width and a colour.
#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::Doc;

use super::{built, edit, fields_of, frame::seed};
use crate::{
    domain::{
        change::value::{ObjectPatch, ShapeEdit},
        palette::Color,
    },
    error::Result,
};

fn drawing() -> Value {
    json!({
        "id": "o", "type": "drawing", "path": "0,0,100,50",
        "strokeWidth": 3, "color": 1_973_790, "alpha": 0.5,
    })
}

fn applied(patch: ObjectPatch) -> Result<Doc> {
    let document = Doc::new();
    seed(&document, &drawing())?;
    built(&document, &edit("o".into(), patch))?;
    Ok(document)
}

#[test]
fn a_drawing_takes_a_width_and_a_colour_and_keeps_its_alpha() -> Result<()> {
    let document = applied(ObjectPatch {
        color: Some("red9".parse::<Color>()?),
        shape: ShapeEdit {
            stroke_width: Some(12.0),
            ..ShapeEdit::default()
        },
        ..ObjectPatch::default()
    })?;
    let fields = fields_of(&document, "o");
    assert_eq!(fields.get("strokeWidth"), Some(&json!(12)));
    assert_eq!(fields.get("color"), Some(&json!(0x00e5_484d)));
    assert_eq!(fields.get("alpha"), Some(&json!(0.5)));
    assert_eq!(fields.get("fillColor"), None);
    Ok(())
}

#[rstest]
#[case::too_thick(
    31.0,
    "\"31\" is not a valid drawing stroke width: expected a whole number 1 to 30"
)]
#[case::fractional(
    2.5,
    "\"2.5\" is not a valid drawing stroke width: expected a whole number 1 to 30"
)]
#[case::none(
    0.0,
    "\"0\" is not a valid drawing stroke width: expected a whole number 1 to 30"
)]
fn a_drawing_width_outside_a_whole_1_to_30_is_refused(
    #[case] width: f64,
    #[case] said: &str,
) {
    let refused = applied(ObjectPatch {
        shape: ShapeEdit {
            stroke_width: Some(width),
            ..ShapeEdit::default()
        },
        ..ObjectPatch::default()
    })
    .err()
    .map(|error| error.to_string());
    assert_eq!(refused.as_deref(), Some(said));
}

#[test]
fn a_drawing_has_no_opacity_to_change() {
    let refused = applied(ObjectPatch {
        shape: ShapeEdit {
            opacity: Some(0.5),
            ..ShapeEdit::default()
        },
        ..ObjectPatch::default()
    })
    .err()
    .map(|error| error.to_string());
    assert_eq!(
        refused.as_deref(),
        Some("drawing objects have no opacity to change")
    );
}
