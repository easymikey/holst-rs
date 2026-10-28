#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, add, board as seeded, edit, fields_of};
use crate::{
    domain::{
        board::{arrow_look::ArrowType, shape_type::StrokeStyle},
        change::{
            build::build,
            check::rectangle_of,
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
            line: Some(ArrowType::Curved),
            heads: heads.parse()?,
            ..ArrowEdit::default()
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
            line: Some(ArrowType::Straight),
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

fn arrow_patch(arrow: ArrowEdit) -> ObjectPatch {
    ObjectPatch {
        arrow,
        ..ObjectPatch::default()
    }
}

fn is_unfit(result: &Result<Vec<String>>) -> bool {
    matches!(
        result,
        Err(Error::BoardChange(BoardChangeError::NotArrowLook { .. }))
    )
}

#[test]
fn a_curved_arrow_bends_at_a_point_and_an_elbow_on_one_axis() -> Result<()> {
    let (on_curved, on_elbow) = (seeded()?, seeded()?);
    let curved = arrow(&on_curved, look("none,miro", 2.0)?)?;
    let elbow = arrow(&on_elbow, ArrowLook::default())?;
    let bend = |document: &Doc, id: &String, at: &str| {
        apply(
            document,
            &edit(
                id.clone(),
                arrow_patch(ArrowEdit {
                    bend: at.parse().ok(),
                    ..ArrowEdit::default()
                }),
            ),
        )
    };
    bend(&on_curved, &curved, "120,-80.5")?;
    bend(&on_elbow, &elbow, ",300")?;
    assert_eq!(
        at(&fields_of(&on_curved, &curved), "midwayControls"),
        json!([{ "x": 120, "y": -80.5 }])
    );
    assert_eq!(
        at(&fields_of(&on_elbow, &elbow), "elbowMidwayControls"),
        json!([{ "x": null, "y": 300 }])
    );
    Ok(())
}

#[test]
fn a_bend_changes_with_the_line_given_in_the_same_edit() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, ArrowLook::default())?;
    apply(
        &document,
        &edit(
            id.clone(),
            arrow_patch(ArrowEdit {
                line: Some(ArrowType::Straight),
                bend: "10,20".parse().ok(),
                ..ArrowEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    assert_eq!(
        [at(&fields, "arrowType"), at(&fields, "midwayControls")],
        [json!("straight"), json!([{ "x": 10, "y": 20 }])]
    );
    Ok(())
}

#[test]
fn a_bend_the_line_cannot_take_is_refused() -> Result<()> {
    let (on_curved, on_elbow) = (seeded()?, seeded()?);
    let elbow = arrow(&on_elbow, ArrowLook::default())?;
    let curved = arrow(&on_curved, look("none,miro", 2.0)?)?;
    let refuse = |document: &Doc, id: String, at: &str| {
        apply(
            document,
            &edit(
                id,
                arrow_patch(ArrowEdit {
                    bend: at.parse().ok(),
                    ..ArrowEdit::default()
                }),
            ),
        )
    };
    assert!(is_unfit(&refuse(&on_elbow, elbow, "10,20")));
    assert!(is_unfit(&refuse(&on_curved, curved, "10,")));
    Ok(())
}

#[test]
fn an_end_moves_to_the_side_of_an_object_or_to_a_free_point() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, ArrowLook::default())?;
    apply(
        &document,
        &edit(
            id.clone(),
            arrow_patch(ArrowEdit {
                from: "seed-6:left".parse().ok(),
                to: "40,50.5".parse().ok(),
                ..ArrowEdit::default()
            }),
        ),
    )?;
    let fields = fields_of(&document, &id);
    let middle = rectangle_of(&fields_of(&document, "seed-6"))
        .map(|rect| [Some(rect.x), Some(rect.y + rect.height / 2.0)]);
    let start = at(&fields, "start");
    let point = at(&start, "point");
    assert_eq!(
        [
            at(&start, "objectId"),
            at(&start, "control"),
            at(&start, "relativePoint")
        ],
        [json!("seed-6"), json!(4), json!({ "x": 0, "y": 0.5 })]
    );
    assert_eq!(
        Some([at(&point, "x").as_f64(), at(&point, "y").as_f64()]),
        middle
    );
    assert_eq!(
        at(&fields, "end"),
        json!({ "point": { "x": 40, "y": 50.5 } })
    );
    Ok(())
}

#[test]
fn an_end_cannot_stick_to_the_arrow_itself() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, ArrowLook::default())?;
    let refused = apply(
        &document,
        &edit(
            id.clone(),
            arrow_patch(ArrowEdit {
                to: format!("{id}:top").parse().ok(),
                ..ArrowEdit::default()
            }),
        ),
    );
    assert!(is_unfit(&refused));
    Ok(())
}

#[test]
fn an_arrow_edit_replaces_its_labels_with_their_text_place_and_editor() -> Result<()> {
    let document = seeded()?;
    let id = arrow(&document, ArrowLook::default())?;
    let label = |raw: &str| raw.parse().ok();
    apply(
        &document,
        &edit(
            id.clone(),
            arrow_patch(ArrowEdit {
                labels: [label("Yes@0.25"), label("No")]
                    .into_iter()
                    .flatten()
                    .collect(),
                ..ArrowEdit::default()
            }),
        ),
    )?;
    apply(
        &document,
        &edit(
            id.clone(),
            arrow_patch(ArrowEdit {
                labels: label("Maybe@1").into_iter().collect(),
                ..ArrowEdit::default()
            }),
        ),
    )?;
    assert_eq!(
        at(&fields_of(&document, &id), "texts"),
        json!([{
            "id": "e-1",
            "text": "Maybe",
            "position": 1,
            "editor": [{ "type": "paragraph", "children": [{ "text": "Maybe" }] }],
        }])
    );
    Ok(())
}
