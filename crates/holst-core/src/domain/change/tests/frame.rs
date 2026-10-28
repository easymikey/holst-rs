#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::{Doc, Map as _, MapPrelim, Transact as _};

use super::{add, add_in, edit, remove};
use crate::{
    domain::{
        board::{ObjectKind, any, shape_type::ShapeType},
        change::{
            build::build,
            tests::{ME, fields_of},
            value::{
                ArrowLook,
                BoardChange,
                ChangeSources,
                ChangeUpdate,
                NewObject,
                ObjectPatch,
            },
            write::object_map,
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        time::Timestamp,
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    build(&document, &frame(0.0, 0.0, 800.0, 600.0)?, ME, &context)?;
    build(&document, &frame(100.0, 100.0, 400.0, 300.0)?, ME, &context)?;
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
    build(
        document,
        &change,
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(2),
            ids,
        },
    )
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
    build(
        &document,
        &add(new_sticker),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(1),
            ids: &ids,
        },
    )?;
    let sticker = "r-3";
    assert_eq!(fields_of(&document, sticker).get("parentId"), None);
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
    let removed = build(
        &document,
        &remove("r-1".into()),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(3),
            ids: &ids,
        },
    )?;
    assert_eq!(removed.objects.len(), 2, "the moved-out sticker stays");
    assert_eq!(crate::domain::board::objects(&document).len(), 1);
    Ok(())
}

#[test]
fn a_carried_arrow_moves_its_ends() -> Result<()> {
    let (document, ids) = nested_frames()?;
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let sticker = |x| -> Result<BoardChange> {
        Ok(add(NewObject::Sticker {
            text: "s".into(),
            at: Point::new(x, 150.0)?,
            size: None,
            color: None,
        }))
    };
    build(&document, &sticker(110.0)?, ME, &context)?;
    build(&document, &sticker(300.0)?, ME, &context)?;
    let new_arrow = NewObject::Arrow {
        from: "r-3".into(),
        to: "r-7".into(),
        color: None,
        look: ArrowLook::default(),
    };
    build(&document, &add(new_arrow), ME, &context)?;
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::ORIGIN,
        size: Some(Size::new(400.0, 400.0)?),
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    build(&document, &add(frame), ME, &context)?;
    let note = NewObject::Sticker {
        text: "s".into(),
        at: Point::new(1000.0, 100.0)?,
        size: None,
        color: None,
    };
    build(&document, &add(note), ME, &context)?;
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
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
        build(&document, &sticker(700.0, 10.0)?, ME, &context),
        Err(Error::BoardChange(BoardChangeError::OutsideFrame { .. }))
    ));
    assert!(matches!(
        build(&document, &sticker(-5.0, 10.0)?, ME, &context),
        Err(Error::BoardChange(BoardChangeError::OutsideFrame { .. }))
    ));
    let deep = build(&document, &sticker(150.0, 150.0)?, ME, &context)?;
    let id = deep
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    assert_eq!(
        fields_of(&document, &id).get("parentId"),
        Some(&json!("r-2"))
    );
    let shallow = build(&document, &sticker(10.0, 10.0)?, ME, &context)?;
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    build(
        &document,
        &add(NewObject::Frame {
            title: "F".into(),
            at: Point::ORIGIN,
            size: None,
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        }),
        ME,
        &context,
    )?;
    build(
        &document,
        &add(NewObject::Text {
            text: "t".into(),
            at: Point::new(10.0, 20.0)?,
        }),
        ME,
        &context,
    )?;
    let child = document.get_or_insert_map("objects");
    {
        // imperative: yrs writes take `&mut TransactionMut`
        let mut transaction = document.transact_mut();
        let text = object_map(&transaction, &child, "m-2", &ObjectKind::Text)?;
        text.insert(&mut transaction, "parentId", "m-1");
    }
    let built = build(
        &document,
        &edit(
            "m-1".into(),
            ObjectPatch {
                at: Some(Point::new(100.0, 0.0)?),
                ..ObjectPatch::default()
            },
        ),
        ME,
        &context,
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::new(100.0, 50.0)?,
        size: None,
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    build(&document, &add(frame), ME, &context)?;
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(10.0, 20.0)?,
        size: None,
        color: None,
    };
    let inside = add_in(sticker.clone(), "p-1".into());
    let built = build(&document, &inside, ME, &context)?;
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
        build(&document, &into_sticker, ME, &context),
        Err(Error::BoardChange(BoardChangeError::NotFrame { .. }))
    ));
    Ok(())
}

#[test]
fn a_frame_is_named_by_title_as_a_snapshot_names_it() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("t");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let frame = |label: &str, x: f64| -> Result<NewObject> {
        Ok(NewObject::Frame {
            title: label.into(),
            at: Point::new(x, 0.0)?,
            size: None,
            color: None,
            look: crate::domain::change::value::FrameEdit::default(),
        })
    };
    build(&document, &add(frame("Retro plan", 0.0)?), ME, &context)?;
    build(&document, &add(frame("Retro", 2000.0)?), ME, &context)?;
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(10.0, 20.0)?,
        size: None,
        color: None,
    };
    let parent_of = |spec: &str| -> Result<Option<(f64, Option<String>)>> {
        let built = build(
            &document,
            &add_in(sticker.clone(), spec.into()),
            ME,
            &context,
        )?;
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
    build(
        &document,
        &frame("Backlog"),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(1),
            ids: &backlog,
        },
    )?;
    build(
        &document,
        &frame("Week 2"),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(1),
            ids: &week,
        },
    )?;
    let sticker = NewObject::Sticker {
        text: "in".into(),
        at: Point::new(10.0, 20.0)?,
        size: None,
        color: None,
    };
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &week,
    };
    let added = build(&document, &add_in(sticker, "1".into()), ME, &context);
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let frame = NewObject::Frame {
        title: "F".into(),
        at: Point::ORIGIN,
        size: None,
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    };
    build(&document, &add(frame), ME, &context)?;
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
    let built = build(&document, &add_in(sticker, "z-1".into()), ME, &context)?;
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
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
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
        let built = build(&document, &add(new), ME, &context)?;
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
    let built = build(
        &document,
        &add(new),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(2),
            ids: &ids,
        },
    )?;
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
