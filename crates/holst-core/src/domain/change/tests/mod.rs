#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

mod add_all;
mod arrange;
mod arrow_edit;
mod drawing_edit;
mod frame;
mod frame_look;
mod group;
mod journal;
mod kind_text_color;
mod link;
mod lock;
mod locked;
mod made;
mod many;
mod markdown;
mod media;
mod new_kinds;
mod note;
mod objects;
mod remove;
mod reparent;
mod resize;
mod rotation;
mod schema_v3;
mod shape_edit;
mod tags;
mod text_width;

use std::collections::HashMap;

use rstest::rstest;
use serde_json::{Value, json};
use yrs::{
    Doc,
    GetString as _,
    Map as _,
    ReadTxn,
    StateVector,
    Text as _,
    Transact as _,
    Update,
    XmlTextRef,
    types::{ToJson as _, text::YChange, xml::Xml as _},
    updates::decoder::Decode as _,
};

use crate::{
    domain::{
        board,
        board::ObjectKind,
        change::{
            build::{author_number, build, client_id},
            check::{check, on_board},
            value::{
                ArrowLook,
                BoardChange,
                ChangeSources,
                ChangeUpdate,
                ChangedObject,
                NewEntry,
                NewObject,
                ObjectChangeKind,
                ObjectEdit,
                ObjectPatch,
                RemoveRight,
            },
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn add(new: NewObject) -> BoardChange {
    BoardChange::AddAll(vec![NewEntry {
        new,
        frame: None,
        format: crate::domain::rich_text::TextFormat::Plain,
    }])
}

fn add_in(new: NewObject, frame: String) -> BoardChange {
    BoardChange::AddAll(vec![NewEntry {
        new,
        frame: Some(frame),
        format: crate::domain::rich_text::TextFormat::Plain,
    }])
}

fn edit(object: String, patch: ObjectPatch) -> BoardChange {
    BoardChange::EditAll(vec![ObjectEdit {
        object,
        patch,
        stacking: None,
        tags: None,
    }])
}

fn remove(object: String) -> BoardChange {
    BoardChange::RemoveAll(vec![object])
}

const ME: &str = "00000000-0000-4000-8000-0000000000a1";

/// A square sticker at `x`, `y` on layer `z`, free or inside `parent`.
pub(super) fn sticker_at(
    id: &str,
    parent: Option<&str>,
    x: u32,
    y: u32,
    z: u32,
) -> Value {
    json!({
        "id": id, "type": "sticker", "parentId": parent,
        "position": { "x": x, "y": y }, "width": 100, "height": 100, "zIndex": z,
    })
}

/// Builds `change` as `ME` at 2 ms with ids `g-1`, `g-2`, ...
pub(super) fn built(document: &Doc, change: &BoardChange) -> Result<ChangeUpdate> {
    built_with_ids(document, &SequentialIds::new("g"), change)
}

/// Builds `change` as `ME` at 2 ms from `ids`, for tests reading ids back.
pub(super) fn built_with_ids(
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

fn board() -> Result<Doc> {
    let document = Doc::new();
    let ids = SequentialIds::new("seed");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let put = |new| build(&document, &add(new), ME, &context);
    put(NewObject::Frame {
        title: "F".into(),
        at: Point::ORIGIN,
        size: None,
        color: None,
        look: crate::domain::change::value::FrameEdit::default(),
    })?;
    put(NewObject::Sticker {
        text: "a".into(),
        at: Point::new(10.0, 10.0)?,
        size: None,
        color: None,
    })?;
    put(NewObject::Text {
        text: "b".into(),
        at: Point::new(900.0, 0.0)?,
    })?;
    Ok(document)
}

#[test]
fn a_client_id_is_redrawn_while_taken() {
    let first = client_id("seed", |_| false);
    let second = client_id("seed", |id| id == first);
    assert_eq!(first, u64::from(author_number("seed", 3)));
    assert_eq!(second, u64::from(author_number("seed", 4)));
    assert!(second < 1 << 31);
}

#[rstest]
#[case("100,-40", Some((100.0, -40.0)))]
#[case(" 1.5 , 2 ", Some((1.5, 2.0)))]
#[case("1;2", None)]
#[case("inf,2", None)]
#[case("1,NaN", None)]
fn points_parse(#[case] raw: &str, #[case] point: Option<(f64, f64)>) {
    assert_eq!(
        raw.parse::<Point>()
            .ok()
            .map(|parsed| (parsed.x(), parsed.y())),
        point
    );
}

#[rstest]
#[case::far_corner(Point::new(1e308, 0.0).map(|at| add(NewObject::Text { text: "x".into(), at })))]
#[case::huge_size(Size::new(1e13, 10.0).map(|size| edit("s-1".into(), ObjectPatch { size: Some(size), ..ObjectPatch::default() })))]
fn objects_off_the_board_are_refused(
    #[case] change: Result<BoardChange>,
) -> Result<()> {
    let checked = check(&change?, &HashMap::new(), RemoveRight::Anyone, None).err();
    assert!(
        matches!(
            checked,
            Some(Error::BoardChange(BoardChangeError::OffBoard { .. }))
        ),
        "{checked:?}"
    );
    Ok(())
}

#[test]
fn an_off_board_corner_is_shown_short() -> Result<()> {
    let far = on_board(Some(&Point::new(1e308, -2.5)?), None).err();
    assert_eq!(
        far.map(|error| error.to_string()).as_deref(),
        Some("1e308,-2.5 is off the board: coordinates and sizes go up to 1e12")
    );
    Ok(())
}

#[rstest]
#[case("192,192", Some((192.0, 192.0)))]
#[case(" 400 , 200 ", Some((400.0, 200.0)))]
#[case("192x192", None)]
#[case("400×200", None)]
#[case("0,10", None)]
#[case("-5,10", None)]
#[case("inf,10", None)]
#[case("10", None)]
fn sizes_parse(#[case] raw: &str, #[case] size: Option<(f64, f64)>) {
    assert_eq!(
        raw.parse::<Size>()
            .ok()
            .map(|parsed| (parsed.width(), parsed.height())),
        size
    );
}

#[test]
fn a_new_sticker_has_the_web_fields_and_adds_the_author() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("n");
    let built = build(
        &document,
        &add(NewObject::Sticker {
            text: "one\ntwo".into(),
            at: Point::new(100.0, 50.0)?,
            size: None,
            color: Some("blue3".parse()?),
        }),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(42),
            ids: &ids,
        },
    )?;
    let transaction = document.transact();
    let objects = transaction.get_map("objects");
    let fields = objects
        .and_then(|o| o.get(&transaction, "n-1"))
        .and_then(|v| board::json(&v.to_json(&transaction)))
        .unwrap_or_default();
    assert_eq!(fields.get("type"), Some(&json!("sticker")));
    assert_eq!(fields.get("documentId"), Some(&json!("n-2")));
    assert_eq!(
        fields.get("fillColor"),
        Some(&json!({"color": "blue3", "opacity": 1}))
    );
    assert_eq!(
        fields.get("created"),
        Some(&json!({"a": 392_241_187, "t": 42}))
    );
    assert_eq!(fields.get("updated"), Some(&Value::Null));
    assert_eq!(fields.get("width"), Some(&json!(192)));
    let users = transaction
        .get_map("usersIds")
        .map(|u| u.to_json(&transaction));
    assert_eq!(
        users.as_ref().and_then(board::json),
        Some(json!({ME: 392_241_187}))
    );
    drop(transaction);
    let listed = board::objects(&document);
    assert_eq!(listed.first().map(|o| o.text.as_str()), Some("one\ntwo"));
    assert_eq!(
        built.objects,
        [ChangedObject {
            id: "n-1".into(),
            kind: ObjectKind::Sticker,
            change: ObjectChangeKind::Created,
            fields: Vec::new(),
            link: None,
        }]
    );
    Ok(())
}

#[test]
fn the_update_is_a_diff_that_replays_the_change() -> Result<()> {
    let document = board()?;
    let before = document
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    let ids = SequentialIds::new("e");
    let built = build(
        &document,
        &edit(
            "seed-2".into(),
            ObjectPatch {
                text: Some("new".into()),
                at: Some(Point::new(5.0, 6.0)?),
                ..ObjectPatch::default()
            },
        ),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(2),
            ids: &ids,
        },
    )?;
    let replica = Doc::new();
    replica
        .transact_mut()
        .apply_update(Update::decode_v2(&before)?)?;
    replica
        .transact_mut()
        .apply_update(Update::decode_v2(&built.update)?)?;
    let sticker = board::objects(&replica)
        .into_iter()
        .find(|o| o.id == "seed-2");
    assert_eq!(
        sticker.map(|sticker| (sticker.text, sticker.x, sticker.y)),
        Some(("new".into(), 5.0, 6.0))
    );
    assert!(
        built.update.len() < before.len(),
        "a diff, not the whole board"
    );
    Ok(())
}

fn fields_of(document: &Doc, id: &str) -> Value {
    let transaction = document.transact();
    transaction
        .get_map("objects")
        .and_then(|objects| objects.get(&transaction, id))
        .and_then(|value| board::json(&value.to_json(&transaction)))
        .unwrap_or_default()
}

fn churn_scratch(server: &Doc, deleted: u32) {
    let scratch = server.get_or_insert_map("scratch");
    (0..deleted).for_each(|number| {
        // imperative: yrs writes take `&mut TransactionMut`
        let mut transaction = server.transact_mut();
        scratch.insert(
            &mut transaction,
            format!("gone-{number}"),
            f64::from(number),
        );
        scratch.insert(
            &mut transaction,
            format!("kept-{number}"),
            f64::from(number),
        );
        scratch.remove(&mut transaction, &format!("gone-{number}"));
    });
}

fn loaded_with_history(deleted: u32) -> Result<Doc> {
    let server = Doc::with_client_id(9);
    let ids = SequentialIds::new("h");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let sticker = |x| -> Result<NewObject> {
        Ok(NewObject::Sticker {
            text: "s".into(),
            at: Point::new(x, 0.0)?,
            size: None,
            color: None,
        })
    };
    build(
        &server,
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
    build(&server, &add_in(sticker(10.0)?, "h-1".into()), ME, &context)?;
    build(
        &server,
        &add(NewObject::Text {
            text: "t".into(),
            at: Point::new(900.0, 0.0)?,
        }),
        ME,
        &context,
    )?;
    build(&server, &add(sticker(2000.0)?), ME, &context)?;
    let arrow = NewObject::Arrow {
        from: "h-2".into(),
        to: "h-6".into(),
        color: None,
        look: ArrowLook::default(),
    };
    build(&server, &add(arrow), ME, &context)?;
    churn_scratch(&server, deleted);
    let loaded = Doc::with_client_id(7);
    loaded.transact_mut().apply_update(Update::decode_v2(
        &server
            .transact()
            .encode_state_as_update_v2(&StateVector::default()),
    )?)?;
    Ok(loaded)
}

fn copy(document: &Doc) -> Result<Doc> {
    let replica = Doc::new();
    replica.transact_mut().apply_update(Update::decode_v2(
        &document
            .transact()
            .encode_state_as_update_v2(&StateVector::default()),
    )?)?;
    Ok(replica)
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Allowed,
    NoSuchField(&'static str),
    WrongStickerShape,
}

fn edit_verdict(
    kind: &str,
    size: Option<&str>,
    color: Option<&str>,
) -> Result<Verdict> {
    let id = "00000000-0000-4000-8000-000000000001";
    let patch = ObjectPatch {
        size: size.map(str::parse).transpose()?,
        color: color.map(str::parse).transpose()?,
        ..ObjectPatch::default()
    };
    let states = HashMap::from([(id.to_owned(), json!({ "type": kind }))]);
    let edit = edit(id.to_owned(), patch);
    match check(&edit, &states, RemoveRight::Anyone, None) {
        Ok(_) => Ok(Verdict::Allowed),
        Err(Error::BoardChange(BoardChangeError::NotChangeable { field, .. })) => {
            Ok(Verdict::NoSuchField(field))
        }
        Err(Error::BoardChange(BoardChangeError::StickerSize { .. })) => {
            Ok(Verdict::WrongStickerShape)
        }
        Err(other) => Err(other),
    }
}

#[rstest]
#[case::square_sticker("sticker", "192,192", Verdict::Allowed)]
#[case::wide_sticker("sticker", "384,192", Verdict::Allowed)]
#[case::three_to_one_sticker("sticker", "300,100", Verdict::WrongStickerShape)]
#[case::any_shape_size("shape", "300,100", Verdict::Allowed)]
#[case::any_frame_size("frame", "300,100", Verdict::Allowed)]
#[case::text_grows_by_scale("simple-text", "192,192", Verdict::Allowed)]
#[case::stack_grows_by_scale("sticker-stack", "192,192", Verdict::Allowed)]
#[case::arrow_has_no_size("arrow", "192,192", Verdict::NoSuchField("size"))]
fn an_edit_resizes_only_what_has_a_size(
    #[case] kind: &str,
    #[case] size: &str,
    #[case] verdict: Verdict,
) -> Result<()> {
    assert_eq!(edit_verdict(kind, Some(size), None)?, verdict);
    Ok(())
}

#[rstest]
#[case::sticker("sticker", Verdict::Allowed)]
#[case::shape("shape", Verdict::Allowed)]
#[case::frame("frame", Verdict::Allowed)]
#[case::arrow("arrow", Verdict::Allowed)]
#[case::text_has_no_colour("simple-text", Verdict::NoSuchField("color"))]
fn an_edit_recolours_only_what_has_a_colour(
    #[case] kind: &str,
    #[case] verdict: Verdict,
) -> Result<()> {
    assert_eq!(edit_verdict(kind, None, Some("yellow4"))?, verdict);
    Ok(())
}

/// Per root embed: its `type`, then per inner embed its `type` and text.
fn text_layout(document: &Doc, id: &str) -> Vec<(String, Vec<(String, String)>)> {
    let transaction = document.transact();
    let attribute = |element: &XmlTextRef, name: &str| {
        element
            .get_attribute(&transaction, name)
            .map(|value| value.to_string(&transaction))
            .unwrap_or_default()
    };
    let embeds = |element: &XmlTextRef| -> Vec<XmlTextRef> {
        element
            .diff(&transaction, YChange::identity)
            .into_iter()
            .filter_map(|chunk| chunk.insert.cast::<XmlTextRef>().ok())
            .collect()
    };
    let fields = fields_of(document, id);
    let root = fields
        .get("documentId")
        .and_then(Value::as_str)
        .and_then(|name| transaction.get_map("documents")?.get(&transaction, name))
        .and_then(|out| out.cast::<XmlTextRef>().ok());
    root.into_iter()
        .flat_map(|root| embeds(&root))
        .collect::<Vec<_>>()
        .iter()
        .map(|wrapper| {
            assert_ne!(attribute(wrapper, "key"), "");
            let blocks = embeds(wrapper)
                .iter()
                .map(|block| (attribute(block, "type"), block.get_string(&transaction)))
                .collect();
            (attribute(wrapper, "type"), blocks)
        })
        .collect()
}

#[test]
fn a_two_line_sticker_is_two_wrappers() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("w");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let sticker = |text: &str| -> Result<NewObject> {
        Ok(NewObject::Sticker {
            text: text.into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })
    };
    build(&document, &add(sticker("a\nb")?), ME, &context)?;
    build(&document, &add(sticker("solo")?), ME, &context)?;
    let line = |text: &str| {
        (
            "wrapper".to_owned(),
            vec![("paragraph".to_owned(), text.to_owned())],
        )
    };
    assert_eq!(text_layout(&document, "w-1"), [line("a"), line("b")]);
    let solo = board::objects(&document)
        .into_iter()
        .find(|o| o.text == "solo")
        .map(|o| o.id)
        .unwrap_or_default();
    assert_eq!(text_layout(&document, &solo), [line("solo")]);
    build(
        &document,
        &edit(
            "w-1".into(),
            ObjectPatch {
                text: Some("x\ny".into()),
                ..ObjectPatch::default()
            },
        ),
        ME,
        &context,
    )?;
    assert_eq!(text_layout(&document, "w-1"), [line("x"), line("y")]);
    let listed = board::objects(&document);
    assert_eq!(listed.first().map(|o| o.text.as_str()), Some("x\ny"));
    Ok(())
}
