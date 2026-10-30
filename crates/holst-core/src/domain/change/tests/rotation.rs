use std::collections::HashMap;

use rstest::rstest;
use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, board as seeded, edit, fields_of};
use crate::{
    domain::{
        change::{
            build::build,
            check::check,
            value::{ChangeSources, ObjectPatch, RemoveRight},
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn turn(document: &Doc, degrees: f64) -> Result<()> {
    turn_with(document, degrees, ObjectPatch::default())
}

fn turn_with(document: &Doc, degrees: f64, patch: ObjectPatch) -> Result<()> {
    let ids = SequentialIds::new("r");
    build(
        document,
        &edit(
            "seed-2".into(),
            ObjectPatch {
                rotation: Some(degrees),
                ..patch
            },
        ),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(9),
            ids: &ids,
        },
    )
    .map(drop)
}

fn turned(fields: &Value) -> Option<f64> {
    fields.get("rotation").and_then(Value::as_f64)
}

/// The rotation and the corner of the seeded sticker, rounded to hundredths.
fn pose(document: &Doc) -> Value {
    let fields = fields_of(document, "seed-2");
    let round = |name: &str| {
        let number = fields
            .get("position")
            .and_then(|position| position.get(name))
            .and_then(Value::as_f64)
            .unwrap_or_default();
        (number * 100.0).round() / 100.0
    };
    json!([turned(&fields), round("x"), round("y")])
}

fn refusal(kind: &str) -> Option<&'static str> {
    let id = "00000000-0000-4000-8000-000000000001";
    let patch = ObjectPatch {
        rotation: Some(30.0),
        ..ObjectPatch::default()
    };
    let states = HashMap::from([(id.to_owned(), json!({ "type": kind }))]);
    match check(
        &edit(id.to_owned(), patch),
        &states,
        RemoveRight::Anyone,
        None,
    ) {
        Err(Error::BoardChange(BoardChangeError::NotChangeable { field, .. })) => {
            Some(field)
        }
        Ok(_) | Err(_) => None,
    }
}

#[test]
fn a_turn_keeps_the_centre_of_the_object_where_it_was() -> Result<()> {
    let document = seeded()?;
    turn(&document, 90.0)?;
    assert_eq!(pose(&document), json!([90.0, 202.0, 10.0]));
    Ok(())
}

#[test]
fn turning_back_to_zero_brings_the_corner_home() -> Result<()> {
    let document = seeded()?;
    turn(&document, 90.0)?;
    turn(&document, 0.0)?;
    assert_eq!(pose(&document), json!([0.0, 10.0, 10.0]));
    Ok(())
}

#[test]
fn a_turn_from_an_angle_goes_by_the_difference() -> Result<()> {
    let document = seeded()?;
    turn(&document, 30.0)?;
    turn(&document, 120.0)?;
    let again = seeded()?;
    turn(&again, 120.0)?;
    // Each turn cuts the corner to hundredths, so two turns may differ by one.
    let (two, one) = (pose(&document), pose(&again));
    let gap = |axis: usize| {
        two.get(axis)
            .and_then(Value::as_f64)
            .zip(one.get(axis).and_then(Value::as_f64))
            .map(|(left, right)| (left - right).abs())
    };
    assert_eq!(two.get(0), one.get(0));
    assert!(
        [1, 2]
            .into_iter()
            .all(|axis| gap(axis).is_some_and(|gap| gap < 0.0101)),
        "{two} against {one}"
    );
    Ok(())
}

#[rstest]
#[case::past_a_round(450.0, 90.0)]
#[case::negative(-90.0, 270.0)]
#[case::full_turn(360.0, 0.0)]
fn the_angle_is_kept_below_a_full_turn(
    #[case] asked: f64,
    #[case] stored: f64,
) -> Result<()> {
    let document = seeded()?;
    turn(&document, asked)?;
    assert_eq!(turned(&fields_of(&document, "seed-2")), Some(stored));
    Ok(())
}

#[test]
fn a_turn_with_a_move_turns_about_the_centre_of_the_new_place() -> Result<()> {
    let document = seeded()?;
    turn_with(
        &document,
        90.0,
        ObjectPatch {
            at: Some(Point::new(100.0, 100.0)?),
            ..ObjectPatch::default()
        },
    )?;
    assert_eq!(pose(&document), json!([90.0, 292.0, 100.0]));
    Ok(())
}

#[test]
fn a_turn_with_a_size_turns_about_the_centre_of_the_new_size() -> Result<()> {
    let document = seeded()?;
    turn_with(
        &document,
        90.0,
        ObjectPatch {
            size: Some(Size::new(384.0, 192.0)?),
            ..ObjectPatch::default()
        },
    )?;
    assert_eq!(pose(&document), json!([90.0, 298.0, -86.0]));
    Ok(())
}

#[rstest]
#[case::table("table", Some("rotation"))]
#[case::table_cell("table-cell", Some("rotation"))]
#[case::frame("frame", Some("rotation"))]
#[case::group("group", Some("rotation"))]
#[case::arrow("arrow", Some("rotation"))]
#[case::sticker("sticker", None)]
#[case::shape("shape", None)]
#[case::text("simple-text", None)]
#[case::image("image", None)]
fn only_what_can_turn_takes_a_rotation(
    #[case] kind: &str,
    #[case] refused: Option<&'static str>,
) {
    assert_eq!(refusal(kind), refused);
}
