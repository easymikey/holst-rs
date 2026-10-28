#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, add, board as seeded, edit, fields_of};
use crate::{
    domain::{
        board::frame_look::{FrameBackground, FrameRatio},
        change::{
            build::build,
            value::{BoardChange, ChangeSources, FrameEdit, NewObject, ObjectPatch},
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

fn frame(document: &Doc, look: FrameEdit, size: Option<Size>) -> Result<String> {
    let made = apply(
        document,
        &add(NewObject::Frame {
            title: "Q4".into(),
            at: Point::new(0.0, 0.0)?,
            size,
            color: None,
            look,
        }),
    )?;
    Ok(made.into_iter().next().unwrap_or_default())
}

fn frame_edit(frame: FrameEdit) -> ObjectPatch {
    ObjectPatch {
        frame,
        ..ObjectPatch::default()
    }
}

fn number(fields: &Value, name: &str) -> f64 {
    fields.get(name).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

#[test]
fn a_new_frame_is_written_with_its_ratio_background_and_hidden_content() -> Result<()> {
    let document = seeded()?;
    let id = frame(
        &document,
        FrameEdit {
            ratio: Some(FrameRatio::Standard),
            background: Some(FrameBackground::Outline),
            hidden: Some(true),
        },
        Some(FrameRatio::Standard.default_size()?),
    )?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        (
            fields.get("ratio").cloned(),
            fields.get("backgroundStyle").cloned(),
            fields.get("isContentHidden").cloned(),
            (number(&fields, "width"), number(&fields, "height")),
        ),
        (
            Some(json!(2)),
            Some(json!(3)),
            Some(json!(true)),
            (1024.0, 768.0)
        )
    );
    Ok(())
}

#[test]
fn a_ratio_edit_reshapes_the_frame_keeping_its_area() -> Result<()> {
    let document = seeded()?;
    let id = frame(
        &document,
        FrameEdit::default(),
        Some(Size::new(800.0, 600.0)?),
    )?;
    apply(
        &document,
        &edit(
            id.clone(),
            frame_edit(FrameEdit {
                ratio: Some(FrameRatio::Wide),
                ..FrameEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    let (width, height) = (number(&fields, "width"), number(&fields, "height"));
    assert_eq!(fields.get("ratio"), Some(&json!(1)));
    assert!((width * height - 480_000.0).abs() < 1e-3);
    assert!((width / height - 16.0 / 9.0).abs() < 1e-9);
    Ok(())
}

#[test]
fn a_background_and_hidden_edit_change_only_those_fields() -> Result<()> {
    let document = seeded()?;
    let id = frame(
        &document,
        FrameEdit::default(),
        Some(Size::new(800.0, 600.0)?),
    )?;
    apply(
        &document,
        &edit(
            id.clone(),
            frame_edit(FrameEdit {
                background: Some(FrameBackground::Fill),
                hidden: Some(true),
                ..FrameEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        (
            fields.get("backgroundStyle").cloned(),
            fields.get("isContentHidden").cloned(),
            fields.get("ratio").cloned(),
            number(&fields, "width"),
        ),
        (Some(json!(1)), Some(json!(true)), None, 800.0)
    );
    Ok(())
}

#[test]
fn only_a_frame_has_a_ratio() -> Result<()> {
    let document = seeded()?;
    let refused = apply(
        &document,
        &edit(
            "seed-2".into(),
            frame_edit(FrameEdit {
                ratio: Some(FrameRatio::Square),
                ..FrameEdit::default()
            }),
        ),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "ratio",
            ..
        }))
    ));
    Ok(())
}
