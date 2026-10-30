#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::{Doc, Map as _, MapPrelim, Transact as _};

use super::{add, add_in, built_with_ids, edit, remove};
use crate::{
    domain::{
        board::{ObjectKind, any, shape_type::ShapeType},
        change::{
            tests::fields_of,
            value::{ArrowLook, BoardChange, ChangeUpdate, NewObject, ObjectPatch},
            write::object_map,
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        lookup,
        lookup::LookupError,
    },
};

pub(super) fn nested_frames() -> Result<(Doc, SequentialIds)> {
    let document = Doc::new();
    let ids = SequentialIds::new("r");
    let frame = |x, y, width, height| -> Result<BoardChange> {
        Ok(add(NewObject::Frame {
            title: "F".into(),
            at: Point::new(x, y)?,
            size: Some(Size::new(width, height)?),
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        }))
    };
    built_with_ids(&document, &ids, &frame(0.0, 0.0, 800.0, 600.0)?)?;
    built_with_ids(&document, &ids, &frame(100.0, 100.0, 400.0, 300.0)?)?;
    Ok((document, ids))
}

pub(super) fn move_to(
    document: &Doc,
    ids: &SequentialIds,
    object: &str,
    at: Point,
) -> Result<ChangeUpdate> {
    let patch = ObjectPatch {
        at: Some(at),
        ..ObjectPatch::default()
    };
    let change = edit(object.into(), patch);
    built_with_ids(document, ids, &change)
}

#[test]
fn a_move_reparents_like_the_web_client() -> Result<()> {
    let (document, ids) = nested_frames()?;
    assert_eq!(
        fields_of(&document, "r-2").get("parentId"),
        Some(&json!("r-1"))
    );
    let new_sticker = NewObject::Sticker {
        text: "s".into(),
        at: Point::new(900.0, 0.0)?,
        size: None,
        color: None,
    };
    built_with_ids(&document, &ids, &add(new_sticker))?;
    let sticker = "r-3";
    assert_eq!(
        fields_of(&document, sticker).get("parentId"),
        Some(&Value::Null)
    );
    let into = move_to(&document, &ids, sticker, Point::new(150.0, 150.0)?)?;
    assert_eq!(
        fields_of(&document, sticker).get("parentId"),
        Some(&json!("r-2"))
    );
    assert_eq!(
        into.objects.first().map(|t| t.fields.clone()),
        Some(vec!["position", "parent"])
    );
    move_to(&document, &ids, sticker, Point::new(10.0, 10.0)?)?;
    assert_eq!(
        fields_of(&document, sticker).get("parentId"),
        Some(&json!("r-1"))
    );
    move_to(&document, &ids, sticker, Point::new(2000.0, 0.0)?)?;
    assert_eq!(
        fields_of(&document, sticker).get("parentId"),
        Some(&Value::Null)
    );
    let removed = built_with_ids(&document, &ids, &remove("r-1".into()))?;
    assert_eq!(removed.objects.len(), 2, "the moved-out sticker stays");
    assert_eq!(crate::domain::board::objects(&document).len(), 1);
    Ok(())
}

#[test]
fn a_carried_arrow_moves_its_ends() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let sticker = |x| -> Result<BoardChange> {
        Ok(add(NewObject::Sticker {
            text: "s".into(),
            at: Point::new(x, 150.0)?,
            size: None,
            color: None,
        }))
    };
    built_with_ids(&document, &ids, &sticker(110.0)?)?;
    built_with_ids(&document, &ids, &sticker(300.0)?)?;
    let new_arrow = NewObject::Arrow {
        from: "r-3".into(),
        to: "r-7".into(),
        color: None,
        look: ArrowLook::default(),
    };
    built_with_ids(&document, &ids, &add(new_arrow))?;
    let arrow = "r-11";
    assert_eq!(
        fields_of(&document, arrow).get("parentId"),
        Some(&json!("r-2"))
    );
    {
        let objects = document.get_or_insert_map("objects");
        // imperative: yrs writes take `&mut TransactionMut`
        let mut transaction = document.transact_mut();
        let map = object_map(&transaction, &objects, arrow, &ObjectKind::Arrow)?;
        let free = json!({ "point": { "x": 120, "y": 130 } });
        map.insert(&mut transaction, "start", any(&free)?);
    }
    let built = move_to(&document, &ids, "r-1", Point::new(50.0, 20.0)?)?;
    let moved = fields_of(&document, arrow);
    assert_eq!(moved.get("position"), Some(&json!({ "x": 0, "y": 0 })));
    assert_eq!(
        moved.pointer("/start/point"),
        Some(&json!({ "x": 170, "y": 150 }))
    );
    assert_eq!(
        moved.pointer("/end/point"),
        Some(&json!({ "x": 446, "y": 266 }))
    );
    assert_eq!(moved.pointer("/end/objectId"), Some(&json!("r-7")));
    let ends = built
        .objects
        .iter()
        .find(|t| t.id == arrow)
        .map(|t| t.fields.clone());
    assert_eq!(ends, Some(vec!["ends"]));
    assert_eq!(
        fields_of(&document, "r-2").get("position"),
        Some(&json!({ "x": 150, "y": 120 }))
    );
    Ok(())
}

#[test]
fn a_moved_frame_takes_what_it_covers() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("a");
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::ORIGIN,
        size: Some(Size::new(400.0, 400.0)?),
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    built_with_ids(&document, &ids, &add(frame))?;
    let note = NewObject::Sticker {
        text: "s".into(),
        at: Point::new(1000.0, 100.0)?,
        size: None,
        color: None,
    };
    built_with_ids(&document, &ids, &add(note))?;
    let built = move_to(&document, &ids, "a-1", Point::new(900.0, 0.0)?)?;
    assert_eq!(
        fields_of(&document, "a-2").get("parentId"),
        Some(&json!("a-1"))
    );
    assert_eq!(
        fields_of(&document, "a-2").get("position"),
        Some(&json!({ "x": 1000, "y": 100 })),
        "taken, not moved"
    );
    assert_eq!(built.objects.len(), 2);
    Ok(())
}

#[test]
fn an_object_in_a_frame_must_fit_it() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let sticker = |x, y| -> Result<BoardChange> {
        Ok(add_in(
            NewObject::Sticker {
                text: "s".into(),
                at: Point::new(x, y)?,
                size: None,
                color: None,
            },
            "r-1".into(),
        ))
    };
    assert!(matches!(
        built_with_ids(&document, &ids, &sticker(700.0, 10.0)?),
        Err(Error::BoardChange(BoardChangeError::OutsideFrame { .. }))
    ));
    assert!(matches!(
        built_with_ids(&document, &ids, &sticker(-5.0, 10.0)?),
        Err(Error::BoardChange(BoardChangeError::OutsideFrame { .. }))
    ));
    let deep = built_with_ids(&document, &ids, &sticker(150.0, 150.0)?)?;
    let id = deep
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    assert_eq!(
        fields_of(&document, &id).get("parentId"),
        Some(&json!("r-2"))
    );
    let shallow = built_with_ids(&document, &ids, &sticker(10.0, 10.0)?)?;
    let shallow_id = shallow
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    assert_eq!(
        fields_of(&document, &shallow_id).get("parentId"),
        Some(&json!("r-1"))
    );
    Ok(())
}

#[test]
fn moving_a_frame_carries_what_it_holds() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("m");
    built_with_ids(
        &document,
        &ids,
        &add(NewObject::Frame {
            title: "F".into(),
            at: Point::ORIGIN,
            size: None,
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        }),
    )?;
    built_with_ids(
        &document,
        &ids,
        &add(NewObject::Text {
            text: "t".into(),
            at: Point::new(10.0, 20.0)?,
        }),
    )?;
    let child = document.get_or_insert_map("objects");
    {
        // imperative: yrs writes take `&mut TransactionMut`
        let mut transaction = document.transact_mut();
        let text = object_map(&transaction, &child, "m-2", &ObjectKind::Text)?;
        text.insert(&mut transaction, "parentId", "m-1");
    }
    let built = built_with_ids(
        &document,
        &ids,
        &edit(
            "m-1".into(),
            ObjectPatch {
                at: Some(Point::new(100.0, 0.0)?),
                ..ObjectPatch::default()
            },
        ),
    )?;
    let text = crate::domain::board::objects(&document)
        .into_iter()
        .find(|o| o.id == "m-2");
    assert_eq!(text.map(|t| (t.x, t.y)), Some((110.0, 20.0)));
    assert_eq!(
        built
            .objects
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["m-1", "m-2"]
    );
    Ok(())
}

#[test]
fn an_object_added_in_a_frame_counts_from_its_corner() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("p");
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::new(100.0, 50.0)?,
        size: None,
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    built_with_ids(&document, &ids, &add(frame))?;
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(10.0, 20.0)?,
        size: None,
        color: None,
    };
    let inside = add_in(sticker.clone(), "p-1".into());
    let built = built_with_ids(&document, &ids, &inside)?;
    let id = built
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    let made = crate::domain::board::objects(&document)
        .into_iter()
        .find(|o| o.id == id);
    assert_eq!(
        made.map(|o| (o.x, o.y, o.parent)),
        Some((110.0, 70.0, Some("p-1".into())))
    );
    let into_sticker = add_in(sticker, id);
    assert!(matches!(
        built_with_ids(&document, &ids, &into_sticker),
        Err(Error::BoardChange(BoardChangeError::NotFrame { .. }))
    ));
    Ok(())
}

#[test]
fn a_frame_is_named_by_title_as_a_snapshot_names_it() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("t");
    let frame = |label: &str, x: f64| -> Result<NewObject> {
        Ok(NewObject::Frame {
            title: label.into(),
            at: Point::new(x, 0.0)?,
            size: None,
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        })
    };
    built_with_ids(&document, &ids, &add(frame("Retro plan", 0.0)?))?;
    built_with_ids(&document, &ids, &add(frame("Retro", 2000.0)?))?;
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(10.0, 20.0)?,
        size: None,
        color: None,
    };
    let parent_of = |spec: &str| -> Result<Option<(f64, Option<String>)>> {
        let built =
            built_with_ids(&document, &ids, &add_in(sticker.clone(), spec.into()))?;
        let id = built.objects.first().map(|t| t.id.clone());
        Ok(crate::domain::board::objects(&document)
            .into_iter()
            .find(|o| Some(&o.id) == id.as_ref())
            .map(|o| (o.x, o.parent)))
    };
    assert_eq!(parent_of(" retro ")?, Some((2010.0, Some("t-2".into()))));
    assert_eq!(parent_of("plan")?, Some((10.0, Some("t-1".into()))));
    assert_eq!(parent_of("T-1")?, Some((10.0, Some("t-1".into()))));
    assert!(matches!(
        parent_of("budget"),
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Frame,
            miss: lookup::LookupMiss::NotFound { .. },
            ..
        }))
    ));
    Ok(())
}

#[test]
fn a_short_id_start_names_no_frame_to_add_into() -> Result<()> {
    let document = Doc::new();
    let backlog = SequentialIds::new("1f00aa");
    let week = SequentialIds::new("c200bb");
    let frame = |label: &str| {
        add(NewObject::Frame {
            title: label.into(),
            at: Point::ORIGIN,
            size: None,
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        })
    };
    built_with_ids(&document, &backlog, &frame("Backlog"))?;
    built_with_ids(&document, &week, &frame("Week 2"))?;
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(10.0, 20.0)?,
        size: None,
        color: None,
    };
    let added = built_with_ids(&document, &week, &add_in(sticker, "1".into()));
    assert!(
        matches!(
            added,
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Frame,
                miss: lookup::LookupMiss::NotFound { .. },
                ..
            }))
        ),
        "{added:?}"
    );
    Ok(())
}

#[test]
fn a_frame_without_a_stored_size_takes_its_drawn_size() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("z");
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::ORIGIN,
        size: None,
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    built_with_ids(&document, &ids, &add(frame))?;
    {
        let objects = document.get_or_insert_map("objects");
        // imperative: yrs writes take `&mut TransactionMut`
        let mut transaction = document.transact_mut();
        let map = object_map(&transaction, &objects, "z-1", &ObjectKind::Frame)?;
        map.remove(&mut transaction, "width");
        map.remove(&mut transaction, "height");
    }
    assert_eq!(fields_of(&document, "z-1").get("width"), None);
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(300.0, 200.0)?,
        size: None,
        color: None,
    };
    let built = built_with_ids(&document, &ids, &add_in(sticker, "z-1".into()))?;
    let id = built
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    assert_eq!(
        fields_of(&document, &id).get("parentId"),
        Some(&json!("z-1"))
    );
    Ok(())
}

#[test]
fn new_objects_take_the_kind_default_size() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("d");
    let at = Point::ORIGIN;
    let news = [
        (
            ObjectKind::Sticker,
            NewObject::Sticker {
                text: String::new(),
                at,
                size: None,
                color: None,
            },
        ),
        (
            ObjectKind::Shape,
            NewObject::Shape {
                text: String::new(),
                at,
                size: None,
                color: None,
                kind: ShapeType::Square,
            },
        ),
        (
            ObjectKind::Frame,
            NewObject::Frame {
                title: String::new(),
                at,
                size: None,
                color: None,
                look: crate::domain::change::value::FrameEdit::default(),
            },
        ),
    ];
    news.into_iter().try_for_each(|(kind, new)| {
        let built = built_with_ids(&document, &ids, &add(new))?;
        let id = built
            .objects
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        let fields = fields_of(&document, &id);
        let size = kind
            .default_size()
            .map(|size| (size.width(), size.height()));
        let number = |name: &str| fields.get(name).and_then(Value::as_f64);
        let stored = number("width").zip(number("height"));
        assert_eq!(stored, size, "{kind:?}");
        Ok(())
    })
}

/// Put an object with exactly these stored fields on the board.
pub(super) fn seed(document: &Doc, fields: &Value) -> Result<()> {
    let id = fields.get("id").and_then(Value::as_str).unwrap_or_default();
    let pairs = fields
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| Ok((name.clone(), any(value)?)))
        .collect::<Result<Vec<_>>>()?;
    let objects = document.get_or_insert_map("objects");
    // imperative: yrs writes take `&mut TransactionMut`
    objects.insert(
        &mut document.transact_mut(),
        id,
        MapPrelim::from_iter(pairs),
    );
    Ok(())
}

fn member(id: &str, kind: &str, parent: &str, x: u32, y: u32) -> Value {
    json!({
        "id": id, "type": kind, "parentId": parent,
        "position": { "x": x, "y": y }, "width": 100, "height": 100,
    })
}

#[test]
fn a_move_keeps_a_group_member_in_its_group() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &json!({ "id": "g", "type": "group" }))?;
    seed(&document, &member("s", "sticker", "g", 900, 0))?;
    let built = move_to(&document, &ids, "s", Point::new(150.0, 150.0)?)?;
    let moved = fields_of(&document, "s");
    assert_eq!(moved.get("parentId"), Some(&json!("g")));
    assert_eq!(moved.get("position"), Some(&json!({ "x": 150, "y": 150 })));
    assert_eq!(
        built.objects.first().map(|t| t.fields.clone()),
        Some(vec!["position"])
    );
    Ok(())
}

#[test]
fn a_move_keeps_a_mind_map_child_under_its_node() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &json!({ "id": "m", "type": "mind-map-node" }))?;
    seed(&document, &member("c", "mind-map-node", "m", 900, 0))?;
    move_to(&document, &ids, "c", Point::new(150.0, 150.0)?)?;
    assert_eq!(fields_of(&document, "c").get("parentId"), Some(&json!("m")));
    Ok(())
}

#[test]
fn a_moved_container_carries_what_it_holds() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let root = json!({
        "id": "g", "type": "group", "parentId": null,
        "position": { "x": 900, "y": 0 }, "width": 300, "height": 300,
    });
    seed(&document, &root)?;
    seed(&document, &member("s", "sticker", "g", 910, 10))?;
    let built = move_to(&document, &ids, "g", Point::new(1000.0, 50.0)?)?;
    let held = fields_of(&document, "s");
    assert_eq!(held.get("position"), Some(&json!({ "x": 1010, "y": 60 })));
    assert_eq!(held.get("parentId"), Some(&json!("g")));
    assert_eq!(built.objects.len(), 2);
    Ok(())
}

#[rstest::rstest]
#[case::kanban_card(json!({
    "id": "k", "type": "kanban" }), json!({
    "id": "x", "type": "task-card", "parentId": "k", "columnId": "c1",
    "swimlaneId": "l1", "position": { "x": 900, "y": 0 },
    "width": 100, "height": 100 }),
    "a task-card in a kanban has no position to change: the kanban places it")]
#[case::table_cell(json!({
    "id": "k", "type": "table" }), json!({
    "id": "x", "type": "table-cell", "parentId": "k",
    "position": { "x": 900, "y": 0 }, "width": 100, "height": 100 }),
    "a table-cell in a table has no position to change: the table places it")]
fn a_move_refuses_what_a_container_places(
    #[case] container: Value,
    #[case] held: Value,
    #[case] named: &str,
) -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &container)?;
    seed(&document, &held)?;
    let refused = move_to(&document, &ids, "x", Point::new(150.0, 150.0)?)
        .err()
        .map(|error| error.to_string());
    assert_eq!(refused, Some(named.to_owned()));
    assert_eq!(
        fields_of(&document, "x").get("parentId"),
        held.get("parentId")
    );
    Ok(())
}

pub(super) fn hidden_frame() -> Value {
    json!({
        "id": "h", "type": "frame", "isContentHidden": true,
        "position": { "x": 2000, "y": 0 }, "width": 400, "height": 300,
    })
}

pub(super) fn free(id: &str, x: u32, y: u32) -> Value {
    json!({
        "id": id, "type": "sticker",
        "position": { "x": x, "y": y }, "width": 100, "height": 100,
    })
}

fn parent_of(document: &Doc, id: &str) -> Option<String> {
    fields_of(document, id)
        .get("parentId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[test]
fn a_moved_object_is_not_put_in_a_hidden_frame() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &hidden_frame())?;
    seed(&document, &free("s", 900, 0))?;
    move_to(&document, &ids, "s", Point::new(2100.0, 100.0)?)?;
    assert_eq!(parent_of(&document, "s"), None);
    Ok(())
}

#[test]
fn a_new_object_is_not_put_in_a_hidden_frame() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &hidden_frame())?;
    let new = NewObject::Sticker {
        text: "n".into(),
        at: Point::new(2100.0, 100.0)?,
        size: None,
        color: None,
    };
    let built = built_with_ids(&document, &ids, &add(new))?;
    let id = built
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    assert_eq!(parent_of(&document, &id), None);
    Ok(())
}

#[test]
fn a_moved_hidden_frame_takes_in_nothing() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &hidden_frame())?;
    seed(&document, &free("u", 3050, 50))?;
    move_to(&document, &ids, "h", Point::new(3000.0, 0.0)?)?;
    assert_eq!(parent_of(&document, "u"), None);
    Ok(())
}

fn new_sticker(x: f64, y: f64) -> Result<NewObject> {
    Ok(NewObject::Sticker {
        text: "s".into(),
        at: Point::new(x, y)?,
        size: None,
        color: None,
    })
}

fn number(fields: &Value, pointer: &str) -> Option<f64> {
    fields.pointer(pointer).and_then(Value::as_f64)
}

#[test]
fn a_new_shape_is_not_fixed_in_size() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("r");
    let shape = NewObject::Shape {
        text: "s".into(),
        at: Point::new(0.0, 0.0)?,
        size: None,
        color: None,
        kind: ShapeType::Square,
    };
    built_with_ids(&document, &ids, &add(shape))?;
    assert_eq!(
        fields_of(&document, "r-1").get("fixedSize"),
        Some(&json!(false))
    );
    Ok(())
}

#[test]
fn a_new_object_goes_into_the_frame_that_holds_its_corner() -> Result<()> {
    let (document, ids) = nested_frames()?;
    // The corner is in r-2 (100..500 by 100..400); the box crosses its edge.
    built_with_ids(&document, &ids, &add(new_sticker(450.0, 350.0)?))?;
    assert_eq!(
        fields_of(&document, "r-3").get("parentId"),
        Some(&json!("r-2"))
    );
    Ok(())
}

#[test]
fn an_explicit_frame_still_holds_the_whole_box() {
    let result = nested_frames().and_then(|(document, ids)| {
        built_with_ids(
            &document,
            &ids,
            &add_in(new_sticker(350.0, 250.0)?, "r-2".into()),
        )
    });
    // 100 + 350 puts the box past the edge of r-2.
    assert!(matches!(
        result,
        Err(Error::BoardChange(BoardChangeError::OutsideFrame { .. }))
    ));
}

#[test]
fn a_new_object_outside_every_frame_has_a_null_parent() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("r");
    built_with_ids(&document, &ids, &add(new_sticker(5.0, 5.0)?))?;
    assert_eq!(
        fields_of(&document, "r-1").get("parentId"),
        Some(&Value::Null)
    );
    Ok(())
}

#[test]
fn a_frame_in_a_frame_takes_the_outer_layer() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let z = |id: &str| fields_of(&document, id).get("zIndex").cloned();
    assert_eq!(z("r-2"), z("r-1"));
    // A frame beyond every frame goes behind them; a move into r-2 keeps its
    // layer.
    let apart = NewObject::Frame {
        title: "G".into(),
        at: Point::new(1000.0, 1000.0)?,
        size: Some(Size::new(50.0, 50.0)?),
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    built_with_ids(&document, &ids, &add(apart))?;
    let behind = z("r-3");
    assert_ne!(behind, z("r-2"));
    move_to(&document, &ids, "r-3", Point::new(150.0, 150.0)?)?;
    assert_eq!(
        fields_of(&document, "r-3").get("parentId"),
        Some(&json!("r-2"))
    );
    assert_eq!(z("r-3"), behind);
    Ok(())
}

#[test]
fn an_edit_writes_position_and_size_as_the_web_client_rounds_them() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let shape = NewObject::Shape {
        text: "s".into(),
        at: Point::new(900.0, 0.0)?,
        size: None,
        color: None,
        kind: ShapeType::Square,
    };
    built_with_ids(&document, &ids, &add(shape))?;
    let patch = ObjectPatch {
        at: Some(Point::new(10.123, 20.456)?),
        size: Some(Size::new(100.005, 50.0)?),
        ..ObjectPatch::default()
    };
    built_with_ids(&document, &ids, &edit("r-3".into(), patch))?;
    let fields = fields_of(&document, "r-3");
    // Cut towards zero: .456 is .45 and .005 is .00.
    assert_eq!(
        [
            number(&fields, "/position/x"),
            number(&fields, "/position/y"),
            number(&fields, "/width"),
            number(&fields, "/height"),
        ],
        [Some(10.12), Some(20.45), Some(100.0), Some(50.0)]
    );
    Ok(())
}

#[test]
fn a_carried_child_is_written_rounded() -> Result<()> {
    let (document, ids) = nested_frames()?;
    move_to(&document, &ids, "r-1", Point::new(0.333, 0.777)?)?;
    let fields = fields_of(&document, "r-2");
    assert_eq!(
        [
            number(&fields, "/position/x"),
            number(&fields, "/position/y")
        ],
        [Some(100.33), Some(100.77)]
    );
    Ok(())
}

#[test]
fn the_web_rounding_cuts_towards_zero() {
    use crate::domain::change::write::web_round;
    assert_eq!(
        json!([
            web_round(0.005),
            web_round(-20.456),
            web_round(1.0),
            web_round(0.0)
        ]),
        json!([0.0, -20.45, 1.0, 0.0])
    );
}

#[test]
fn an_adopted_object_gets_no_updated_stamp() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("a");
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::ORIGIN,
        size: Some(Size::new(400.0, 400.0)?),
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    built_with_ids(&document, &ids, &add(frame))?;
    built_with_ids(&document, &ids, &add(new_sticker(1000.0, 100.0)?))?;
    move_to(&document, &ids, "a-1", Point::new(900.0, 0.0)?)?;
    let adopted = fields_of(&document, "a-2");
    assert_eq!(adopted.get("parentId"), Some(&json!("a-1")));
    assert_eq!(adopted.get("updated"), Some(&Value::Null));
    Ok(())
}

#[test]
fn a_carried_child_whose_position_changes_gets_the_stamp() -> Result<()> {
    let (document, ids) = nested_frames()?;
    built_with_ids(&document, &ids, &add(new_sticker(150.0, 150.0)?))?;
    move_to(&document, &ids, "r-2", Point::new(200.0, 200.0)?)?;
    let carried = fields_of(&document, "r-3");
    assert_eq!(carried.get("parentId"), Some(&json!("r-2")));
    assert!(carried.get("updated").is_some_and(|stamp| !stamp.is_null()));
    Ok(())
}

fn arrow_parent(
    document: &Doc,
    ids: &SequentialIds,
    (from, to): (&str, &str),
) -> Result<Option<String>> {
    let new_arrow = NewObject::Arrow {
        from: from.into(),
        to: to.into(),
        color: None,
        look: ArrowLook::default(),
    };
    let built = built_with_ids(document, ids, &add(new_arrow))?;
    let id = built
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    Ok(parent_of(document, &id))
}

#[test]
fn an_arrow_between_two_members_of_a_group_goes_into_the_group() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(
        &document,
        &json!({ "id": "G", "type": "group", "parentId": null }),
    )?;
    seed(&document, &member("s1", "sticker", "G", 900, 0))?;
    seed(&document, &member("s2", "sticker", "G", 1100, 0))?;
    assert_eq!(
        arrow_parent(&document, &ids, ("s1", "s2"))?,
        Some("G".to_owned())
    );
    Ok(())
}

#[test]
fn an_arrow_between_a_group_member_and_a_frame_member_goes_into_the_frame() -> Result<()>
{
    let (document, ids) = nested_frames()?;
    seed(
        &document,
        &json!({ "id": "G", "type": "group", "parentId": "r-2" }),
    )?;
    seed(&document, &member("s1", "sticker", "G", 150, 150))?;
    seed(&document, &member("s2", "sticker", "r-2", 300, 150))?;
    assert_eq!(
        arrow_parent(&document, &ids, ("s1", "s2"))?,
        Some("r-2".to_owned())
    );
    Ok(())
}

#[test]
fn an_arrow_inside_a_hidden_frame_has_no_parent() -> Result<()> {
    let (document, ids) = nested_frames()?;
    seed(&document, &hidden_frame())?;
    seed(&document, &member("s1", "sticker", "h", 2000, 0))?;
    seed(&document, &member("s2", "sticker", "h", 2200, 0))?;
    assert_eq!(arrow_parent(&document, &ids, ("s1", "s2"))?, None);
    Ok(())
}
