//! A text held to a width, and let go to wrap by itself.

use serde_json::Value;
use yrs::Doc;

use super::{ME, edit, fields_of};
use crate::{
    domain::{
        change::{
            build::build,
            value::{
                BoardChange,
                ChangeSources,
                NewEntry,
                NewObject,
                ObjectPatch,
                TextWidth,
            },
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        rich_text::TextFormat,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn board_with(new: NewObject) -> Result<Doc> {
    let document = Doc::new();
    let ids = SequentialIds::new("w");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let add = BoardChange::AddAll(vec![NewEntry {
        new,
        frame: None,
        format: TextFormat::Plain,
    }]);
    build(&document, &add, ME, &context)?;
    Ok(document)
}

fn a_text() -> Result<Doc> {
    board_with(NewObject::Text {
        text: "words".into(),
        at: Point::new(1.0, 1.0)?,
    })
}

fn apply(document: &Doc, patch: ObjectPatch) -> Result<()> {
    let ids = SequentialIds::new("e");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(2),
        ids: &ids,
    };
    build(document, &edit("w-1".into(), patch), ME, &context).map(drop)
}

fn width(width: TextWidth) -> ObjectPatch {
    ObjectPatch {
        width: Some(width),
        ..ObjectPatch::default()
    }
}

fn number(document: &Doc, name: &str) -> Option<f64> {
    fields_of(document, "w-1").get(name).and_then(Value::as_f64)
}

#[test]
fn a_fixed_width_is_stored_before_the_scale_with_the_fixed_flag() -> Result<()> {
    let document = a_text()?;
    apply(
        &document,
        ObjectPatch {
            size: Some(Size::new(400.0, 200.0)?),
            ..ObjectPatch::default()
        },
    )?;
    apply(&document, width(TextWidth::Fixed(300.0)))?;
    let scale = number(&document, "textScale").unwrap_or_default();
    let stored = number(&document, "width").unwrap_or_default();
    assert!(
        scale > 0.0 && (stored * scale - 300.0).abs() < 1e-9,
        "{stored} x {scale}"
    );
    assert_eq!(
        fields_of(&document, "w-1").get("fixedWidth"),
        Some(&Value::Bool(true))
    );
    Ok(())
}

#[test]
fn auto_takes_both_fields_away() -> Result<()> {
    let document = a_text()?;
    apply(&document, width(TextWidth::Fixed(300.0)))?;
    apply(&document, width(TextWidth::Auto))?;
    let fields = fields_of(&document, "w-1");
    assert_eq!(
        (fields.get("width"), fields.get("fixedWidth")),
        (None, None)
    );
    Ok(())
}

#[test]
fn only_a_text_has_a_width() -> Result<()> {
    let document = board_with(NewObject::Sticker {
        text: "note".into(),
        at: Point::new(1.0, 1.0)?,
        size: None,
        color: None,
    })?;
    let refused = apply(&document, width(TextWidth::Fixed(300.0)));
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "width",
            ..
        }))
    ));
    Ok(())
}
