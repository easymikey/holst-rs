#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::json;
use yrs::Doc;

use crate::{
    domain::{
        change::{
            build::build,
            tests::{ME, fields_of},
            value::{ArrowLook, BoardChange, ChangeSources, NewEntry, NewObject},
        },
        geometry::Point,
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn entry(new: NewObject) -> NewEntry {
    NewEntry {
        new,
        frame: None,
        format: crate::domain::rich_text::TextFormat::Plain,
    }
}

fn sticker(text: &str, at: Point) -> NewEntry {
    entry(NewObject::Sticker {
        text: text.into(),
        at,
        size: None,
        color: None,
    })
}

fn arrow(from: &str, to: &str) -> NewEntry {
    entry(NewObject::Arrow {
        from: from.into(),
        to: to.into(),
        color: None,
        look: ArrowLook::default(),
    })
}

fn build_all(document: &Doc, entries: Vec<NewEntry>) -> Result<Vec<String>> {
    let ids = SequentialIds::new("n");
    let built = build(
        document,
        &BoardChange::AddAll(entries),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(5),
            ids: &ids,
        },
    )?;
    Ok(built.objects.into_iter().map(|object| object.id).collect())
}

#[test]
fn many_objects_are_one_write_in_order_each_above_the_last() -> Result<()> {
    let document = Doc::new();
    let made = build_all(
        &document,
        vec![
            sticker("a", Point::new(0.0, 0.0)?),
            entry(NewObject::Text {
                text: "b".into(),
                at: Point::new(300.0, 0.0)?,
            }),
            sticker("c", Point::new(600.0, 0.0)?),
        ],
    )?;
    let layers: Vec<f64> = made
        .iter()
        .filter_map(|id| fields_of(&document, id).get("zIndex")?.as_f64())
        .collect();
    assert_eq!(
        (made.len(), layers.len(), layers.is_sorted_by(|a, b| a < b)),
        (3, 3, true)
    );
    Ok(())
}

#[test]
fn an_arrow_between_items_is_bound_as_the_web_client_binds_one() -> Result<()> {
    let document = Doc::new();
    let made = build_all(
        &document,
        vec![
            sticker("a", Point::new(100.0, 100.0)?),
            sticker("b", Point::new(500.0, 100.0)?),
            arrow("#1", "#2"),
        ],
    )?;
    let at = |slot: usize| made.get(slot).cloned().unwrap_or_default();
    let (first, second) = (at(0), at(1));
    let fields = fields_of(&document, &at(2));
    assert_eq!(
        (
            fields.get("start"),
            fields.get("end"),
            fields.get("position"),
            fields.get("strokeColor"),
            fields.get("strokeWidth"),
            fields.get("arrowType"),
        ),
        (
            Some(&json!({"objectId": first, "control": 2,
                         "relativePoint": {"x": 1, "y": 0.5},
                         "point": {"x": 292, "y": 196}})),
            Some(&json!({"objectId": second, "control": null,
                         "point": {"x": 596, "y": 196}})),
            Some(&json!({"x": 0, "y": 0})),
            Some(&json!({"color": "gray12"})),
            None,
            Some(&json!("elbow")),
        )
    );
    Ok(())
}

#[test]
fn an_arrow_names_only_items_before_it() -> Result<()> {
    let document = Doc::new();
    let ahead = build_all(
        &document,
        vec![arrow("#2", "#2"), sticker("a", Point::new(0.0, 0.0)?)],
    );
    assert!(
        matches!(
            ahead,
            Err(Error::BoardChange(BoardChangeError::NoObject { .. }))
        ),
        "{ahead:?}"
    );
    Ok(())
}

#[test]
fn an_arrow_between_items_of_a_frame_sits_in_the_frame() -> Result<()> {
    let document = Doc::new();
    let frame = |new| NewEntry {
        new,
        frame: None,
        format: crate::domain::rich_text::TextFormat::Plain,
    };
    let made = build_all(
        &document,
        vec![frame(NewObject::Frame {
            title: "F".into(),
            at: Point::new(0.0, 0.0)?,
            size: None,
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        })],
    )?;
    let frame_id = made.first().cloned().unwrap_or_default();
    let inside = |text: &str, x: f64| -> Result<NewEntry> {
        Ok(NewEntry {
            frame: Some(frame_id.clone()),
            ..sticker(text, Point::new(x, 50.0)?)
        })
    };
    let more = build_all(
        &document,
        vec![inside("a", 50.0)?, inside("b", 400.0)?, arrow("#1", "#2")],
    )?;
    let parents: Vec<_> = more
        .iter()
        .map(|id| fields_of(&document, id).get("parentId").cloned())
        .collect();
    assert_eq!(parents, vec![Some(json!(frame_id)); 3]);
    Ok(())
}

#[test]
fn an_item_in_a_frame_of_the_same_list_gets_the_new_frame_as_parent() -> Result<()> {
    let document = Doc::new();
    let new_frame = entry(NewObject::Frame {
        title: "F".into(),
        at: Point::new(100.0, 100.0)?,
        size: None,
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    });
    let inside = |at: Point, parent: &str| NewEntry {
        frame: Some(parent.to_owned()),
        ..sticker("a", at)
    };
    let made = build_all(
        &document,
        vec![new_frame, inside(Point::new(50.0, 50.0)?, "#1")],
    )?;
    let parent = made
        .get(1)
        .and_then(|id| fields_of(&document, id).get("parentId").cloned());
    let frame_id = made.first().cloned().unwrap_or_default();
    let position = made
        .get(1)
        .and_then(|id| fields_of(&document, id).get("position").cloned());
    assert_eq!(
        (parent, position),
        (Some(json!(frame_id)), Some(json!({"x": 150, "y": 150})))
    );
    let outside = build_all(
        &Doc::new(),
        vec![
            entry(NewObject::Frame {
                title: "F".into(),
                at: Point::new(0.0, 0.0)?,
                size: None,
                color: None,
                look: crate::domain::change::value::FrameEdit::default(),
            }),
            inside(Point::new(9000.0, 0.0)?, "#1"),
        ],
    );
    assert!(
        matches!(
            outside,
            Err(Error::BoardChange(BoardChangeError::OutsideFrame { .. }))
        ),
        "{outside:?}"
    );
    Ok(())
}

#[test]
fn a_hash_frame_names_only_an_earlier_frame() {
    let sticker_first = build_all(
        &Doc::new(),
        vec![
            sticker("a", Point::whole(0, 0)),
            NewEntry {
                frame: Some("#1".to_owned()),
                ..sticker("b", Point::whole(1, 1))
            },
        ],
    );
    assert!(
        matches!(
            sticker_first,
            Err(Error::BoardChange(BoardChangeError::NotFrame { .. }))
        ),
        "{sticker_first:?}"
    );
}
