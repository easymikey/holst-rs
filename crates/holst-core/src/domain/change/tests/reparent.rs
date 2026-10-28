#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, add, edit, fields_of, frame::nested_frames};
use crate::{
    domain::{
        change::{
            build::build,
            value::{
                BoardChange,
                ChangeSources,
                ChangeUpdate,
                FrameEdit,
                NewObject,
                ObjectPatch,
                Reparent,
            },
        },
        geometry::Point,
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::Result,
};

const STICKER: &str = "r-3";
const OUTER: &str = "r-1";
const INNER: &str = "r-2";

fn apply(
    document: &Doc,
    ids: &SequentialIds,
    change: &BoardChange,
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

/// An outer frame at 0,0, an inner one at 100,100 inside it, a sticker at 50,20
/// in the outer one.
fn board_with_sticker() -> Result<(Doc, SequentialIds)> {
    let (document, ids) = nested_frames()?;
    apply(
        &document,
        &ids,
        &add(NewObject::Sticker {
            text: "s".into(),
            at: Point::new(50.0, 20.0)?,
            size: None,
            color: None,
        }),
    )?;
    Ok((document, ids))
}

fn put(object: &str, to: Reparent) -> BoardChange {
    edit(
        object.into(),
        ObjectPatch {
            reparent: Some(to),
            ..ObjectPatch::default()
        },
    )
}

fn corner(fields: &Value) -> (Option<f64>, Option<f64>) {
    let number = |axis: &str| fields.pointer(axis).and_then(Value::as_f64);
    (number("/position/x"), number("/position/y"))
}

#[test]
fn an_object_moved_into_a_frame_keeps_its_place_relative_to_it() -> Result<()> {
    let (document, ids) = board_with_sticker()?;
    assert_eq!(
        fields_of(&document, STICKER).get("parentId"),
        Some(&json!(OUTER))
    );
    let update = apply(
        &document,
        &ids,
        &put(STICKER, Reparent::Frame(INNER.into())),
    )?;
    let fields = fields_of(&document, STICKER);
    assert_eq!(
        (fields.get("parentId"), corner(&fields)),
        (Some(&json!(INNER)), (Some(150.0), Some(120.0)))
    );
    assert_eq!(
        update.objects.first().map(|touched| touched.fields.clone()),
        Some(vec!["position", "order", "parent"])
    );
    Ok(())
}

#[test]
fn an_object_that_would_lie_outside_the_frame_goes_to_a_free_spot_in_it() -> Result<()>
{
    let (document, ids) = nested_frames()?;
    apply(
        &document,
        &ids,
        &add(NewObject::Sticker {
            text: "s".into(),
            at: Point::new(600.0, 400.0)?,
            size: None,
            color: None,
        }),
    )?;
    apply(
        &document,
        &ids,
        &put(STICKER, Reparent::Frame(INNER.into())),
    )?;
    let fields = fields_of(&document, STICKER);
    assert_eq!(
        (fields.get("parentId"), corner(&fields)),
        (Some(&json!(INNER)), (Some(124.0), Some(124.0)))
    );
    Ok(())
}

#[test]
fn an_object_moved_out_of_its_frame_goes_to_the_board_in_the_same_place_relative_to_it()
-> Result<()> {
    let (document, ids) = board_with_sticker()?;
    apply(
        &document,
        &ids,
        &put(STICKER, Reparent::Frame(INNER.into())),
    )?;
    apply(&document, &ids, &put(STICKER, Reparent::Board))?;
    let fields = fields_of(&document, STICKER);
    assert_eq!(
        (fields.get("parentId"), corner(&fields)),
        (Some(&Value::Null), (Some(50.0), Some(20.0)))
    );
    Ok(())
}

fn layer(document: &Doc, id: &str) -> f64 {
    fields_of(document, id)
        .get("zIndex")
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

#[test]
fn an_object_put_into_a_frame_goes_past_the_ones_in_it() -> Result<()> {
    let (document, ids) = board_with_sticker()?;
    let made = apply(
        &document,
        &ids,
        &add(NewObject::Sticker {
            text: "t".into(),
            at: Point::new(60.0, 30.0)?,
            size: None,
            color: None,
        }),
    )?;
    let other = made
        .objects
        .first()
        .map(|touched| touched.id.clone())
        .unwrap_or_default();
    apply(
        &document,
        &ids,
        &put(STICKER, Reparent::Frame(INNER.into())),
    )?;
    apply(&document, &ids, &put(&other, Reparent::Frame(INNER.into())))?;
    let (first, second) = (layer(&document, STICKER), layer(&document, &other));
    assert_eq!(
        (
            first >= 1_099_511_627_776.0,
            second > first + 1_099_511_627_776.0
        ),
        (true, true)
    );
    Ok(())
}

#[test]
fn a_given_corner_is_the_one_kept() -> Result<()> {
    let (document, ids) = board_with_sticker()?;
    apply(
        &document,
        &ids,
        &edit(
            STICKER.into(),
            ObjectPatch {
                at: Some(Point::new(200.0, 200.0)?),
                reparent: Some(Reparent::Frame(INNER.into())),
                ..ObjectPatch::default()
            },
        ),
    )?;
    let fields = fields_of(&document, STICKER);
    assert_eq!(
        (fields.get("parentId"), corner(&fields)),
        (Some(&json!(INNER)), (Some(200.0), Some(200.0)))
    );
    Ok(())
}

fn hidden_inner() -> Result<(Doc, SequentialIds)> {
    let (document, ids) = board_with_sticker()?;
    apply(
        &document,
        &ids,
        &edit(
            INNER.into(),
            ObjectPatch {
                frame: FrameEdit {
                    hidden: Some(true),
                    ..FrameEdit::default()
                },
                ..ObjectPatch::default()
            },
        ),
    )?;
    Ok((document, ids))
}

#[rstest]
#[case::hidden(
    hidden_inner,
    STICKER,
    INNER,
    "frame \"r-2\" hides its content: objects put in it would vanish for viewers; show its content in the web client first"
)]
#[case::not_a_frame(
    board_with_sticker,
    INNER,
    STICKER,
    "\"r-3\" is not a frame: only frames hold objects"
)]
#[case::inside_itself(
    board_with_sticker,
    OUTER,
    INNER,
    "frame objects cannot move into or out of a frame: the frame lies inside it"
)]
#[case::itself(
    board_with_sticker,
    OUTER,
    OUTER,
    "frame objects cannot move into or out of a frame: the frame lies inside it"
)]
fn a_frame_that_cannot_take_the_object_is_refused(
    #[case] board: fn() -> Result<(Doc, SequentialIds)>,
    #[case] object: &str,
    #[case] frame: &str,
    #[case] said: &str,
) -> Result<()> {
    let (document, ids) = board()?;
    let refused = apply(&document, &ids, &put(object, Reparent::Frame(frame.into())));
    assert_eq!(
        refused.err().map(|error| error.to_string()),
        Some(said.to_owned())
    );
    Ok(())
}
