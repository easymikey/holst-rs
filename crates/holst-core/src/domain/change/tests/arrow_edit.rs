#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, add, board as seeded, edit, fields_of};
use crate::{
    domain::{
        board::shape_type::StrokeStyle,
        change::{
            build::build,
            value::{
                ArrowEdit,
                ArrowLook,
                BoardChange,
                ChangeSources,
                NewObject,
                ObjectPatch,
                ShapeEdit,
            },
        },
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

fn look(heads: &str, width: f64) -> Result<ArrowLook> {
    Ok(ArrowLook {
        arrow: ArrowEdit {
            line: Some(crate::domain::board::arrow_look::ArrowType::Curved),
            heads: heads.parse()?,
        },
        outline: ShapeEdit {
            stroke_width: Some(width),
            stroke_style: Some(StrokeStyle::Dotted),
            ..ShapeEdit::default()
        },
    })
}

/// An arrow between two seeded objects, drawn as `look`; its id.
fn arrow(document: &Doc, look: ArrowLook) -> Result<String> {
    let made = apply(
        document,
        &add(NewObject::Arrow {
            from: "seed-2".into(),
            to: "seed-6".into(),
            color: None,
            look,
        }),
    )?;
    Ok(made.into_iter().next().unwrap_or_default())
}

fn at(fields: &Value, name: &str) -> Value {
    fields.get(name).cloned().unwrap_or(Value::Null)
}

#[test]
fn a_new_arrow_holds_its_line_heads_dash_and_width() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, look("triangle,none", 4.0)?)?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        [
            at(&fields, "arrowType"),
            at(&fields, "arrowheadStart"),
            at(&fields, "arrowheadEnd"),
            at(&fields, "strokeStyle"),
            at(&fields, "strokeWidth"),
        ],
        [
            json!("curved"),
            json!("triangle"),
            json!("none"),
            json!("dotted"),
            json!(4),
        ]
    );
    Ok(())
}

#[test]
fn an_arrow_edit_changes_one_head_and_leaves_the_other() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, look("figma,stick", 2.0)?)?;
    apply(
        &document,
        &edit(
            id.clone(),
            ObjectPatch {
                arrow: ArrowEdit {
                    heads: ",slash".parse()?,
                    ..ArrowEdit::default()
                },
                ..ObjectPatch::default()
            },
        ),
    )?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        [at(&fields, "arrowheadStart"), at(&fields, "arrowheadEnd")],
        [json!("figma"), json!("slash")]
    );
    Ok(())
}

#[test]
fn an_arrow_takes_only_the_widths_the_web_offers() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, ArrowLook::default())?;
    let refused = apply(
        &document,
        &edit(
            id,
            ObjectPatch {
                shape: ShapeEdit {
                    stroke_width: Some(3.0),
                    ..ShapeEdit::default()
                },
                ..ObjectPatch::default()
            },
        ),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotArrowLook { .. }))
    ));
    Ok(())
}

#[test]
fn an_arrow_has_no_outline_corners_and_a_sticker_no_line() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, ArrowLook::default())?;
    let radius = ObjectPatch {
        shape: ShapeEdit {
            radius: Some(4),
            ..ShapeEdit::default()
        },
        ..ObjectPatch::default()
    };
    let line = ObjectPatch {
        arrow: ArrowEdit {
            line: Some(crate::domain::board::arrow_look::ArrowType::Straight),
            ..ArrowEdit::default()
        },
        ..ObjectPatch::default()
    };
    let refused = [
        apply(&document, &edit(id, radius)),
        apply(&document, &edit("seed-2".into(), line)),
    ];
    assert!(refused.iter().all(|result| matches!(
        result,
        Err(Error::BoardChange(BoardChangeError::NotChangeable { .. }))
    )));
    Ok(())
}
