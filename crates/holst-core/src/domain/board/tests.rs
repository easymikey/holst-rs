#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::collections::HashMap;

use serde_json::json;
use yrs::{
    Any,
    Doc,
    Map as _,
    ReadTxn as _,
    Text as _,
    Transact as _,
    Xml as _,
    XmlTextPrelim,
};

use crate::{
    domain::{
        area::scene::Scene,
        board::{
            ArrowEnds,
            BoardContent,
            DrawnArrowEnds,
            EditRight,
            Object,
            ObjectKind,
            insert_map,
            object,
            objects,
            text_of,
            with_drawn_ends,
        },
        geometry::Point,
        lookup::rules::find_id,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        lookup,
        lookup::LookupError,
    },
};

/// An object with only the fields the listing and the text index look at.
pub(crate) fn plain_object(id: &str, kind: ObjectKind, text: &str) -> Object {
    Object {
        id: id.to_owned(),
        kind,
        parent: None,
        x: 0.0,
        y: 0.0,
        width: None,
        height: None,
        z_index: 0.0,
        text: text.to_owned(),
        color: None,
        look: None,
        media: None,
        ends: None,
        author: None,
        locked: false,
        rotation: 0.0,
        link_to: None,
        file: None,
        fields: serde_json::Value::Null,
    }
}

#[test]
fn a_loaded_arrow_keeps_its_drawn_ends() {
    let read = |fields| object(fields, |_| String::new(), &HashMap::new());
    let objects = [
        json!({"id": "s1", "type": "sticker", "position": {"x": 0, "y": 0},
                   "width": 192, "height": 192}),
        json!({"id": "a1", "type": "arrow", "position": {"x": 0, "y": 0},
                   "arrowType": "straight",
                   "start": {"objectId": "s1", "point": {"x": 0, "y": 0}},
                   "end": {"point": {"x": 500, "y": 96}}}),
    ]
    .into_iter()
    .filter_map(read)
    .collect();
    let points = with_drawn_ends(objects)
        .into_iter()
        .map(|object| object.ends.and_then(|ends| ends.drawn))
        .collect::<Vec<_>>();
    let drawn = DrawnArrowEnds {
        start: Point::whole(192, 96),
        end: Point::whole(500, 96),
    };
    assert_eq!(points, [None, Some(drawn)]);
}

#[test]
fn sticker_fields_are_read() {
    let authors = HashMap::from([(7, "u1".to_owned())]);
    let fields = json!({
        "id": "s1", "type": "sticker", "documentId": "d1", "parentId": "f1",
        "position": {"x": 100, "y": 50}, "width": 192, "height": 192, "zIndex": 3.5,
        "fillColor": {"color": "yellow4", "opacity": 1}, "created": {"a": 7, "t": 1}
    });
    let read = object(
        fields.clone(),
        |document| format!("text of {document}"),
        &authors,
    );
    assert_eq!(
        read,
        Some(Object {
            id: "s1".into(),
            kind: ObjectKind::Sticker,
            parent: Some("f1".into()),
            x: 100.0,
            y: 50.0,
            width: Some(192.0),
            height: Some(192.0),
            z_index: 3.5,
            text: "text of d1".into(),
            color: Some("yellow4".into()),
            look: None,
            media: None,
            ends: None,
            author: Some("u1".into()),
            locked: false,
            rotation: 0.0,
            link_to: None,
            file: None,
            fields,
        })
    );
}

#[test]
fn arrow_labels_come_from_slate_or_plain_text() {
    let arrow = object(
        json!({"id": "a", "type": "arrow", "texts": [
            {"id": "t1", "editor": [{"type": "paragraph", "children": [{"text": "yes"}, {"text": "!"}]}]},
            {"id": "t2", "text": "no"},
            {"id": "t3"}
        ]}),
        |_| String::new(),
        &HashMap::new(),
    );
    assert_eq!(arrow.map(|a| a.text), Some("yes!\nno".into()));
}

#[test]
fn linked_json_adds_a_link_to_each_object() -> Result<()> {
    let read = |fields| object(fields, |_| String::new(), &HashMap::new());
    let sticker = read(json!({"id": "s1", "type": "sticker"}));
    let arrow =
        read(json!({"id": "a1", "type": "arrow", "position": {"x": 0, "y": 0}}));
    let image = read(
        json!({"id": "i1", "type": "image", "name": "a1.png", "originalName": "cat.png"}),
    );
    let content = BoardContent {
        edit_right: EditRight::Editable,
        objects: sticker.into_iter().chain(arrow).chain(image).collect(),
    };
    let drawn = DrawnArrowEnds {
        start: Point::whole(120, 340),
        end: Point::whole(400, 340),
    };
    let endpoints =
        |object: &Object| (object.kind == ObjectKind::Arrow).then_some(drawn);
    let listed =
        content.linked(|id| format!("https://x/?objectId={id}"), endpoints, "/s/")?;
    assert_eq!(listed.pointer("/objects/0/file"), None);
    assert_eq!(
        listed.pointer("/objects/2/file"),
        Some(&json!({"name": "a1.png", "original": "cat.png", "url": "/s/a1.png"}))
    );
    assert_eq!(listed.get("edit_right"), Some(&json!("editable")));
    assert_eq!(listed.pointer("/objects/0/kind"), Some(&json!("sticker")));
    assert_eq!(
        listed.pointer("/objects/0/link"),
        Some(&json!("https://x/?objectId=s1"))
    );
    assert_eq!(listed.pointer("/objects/0/start"), None);
    // The stored position stays as it was; the drawn ends come beside it.
    assert_eq!(
        [
            listed.pointer("/objects/1/x"),
            listed.pointer("/objects/1/start"),
            listed.pointer("/objects/1/end/x"),
        ],
        [
            Some(&json!(0.0)),
            Some(&json!({"x": 120.0, "y": 340.0})),
            Some(&json!(400.0)),
        ]
    );
    Ok(())
}

#[test]
fn frame_label_and_arrow_ends_are_read() {
    let frame = object(
        json!({"id": "f", "type": "frame", "labelText": "Plan"}),
        |_| String::new(),
        &HashMap::new(),
    );
    assert_eq!(
        frame.map(|f| (f.kind, f.text)),
        Some((ObjectKind::Frame, "Plan".into()))
    );
    let arrow = object(
        json!({"id": "a", "type": "arrow", "start": {"objectId": "s1"}, "end": {"point": {"x": 1, "y": 2}}}),
        |_| String::new(),
        &HashMap::new(),
    );
    assert_eq!(
        arrow.and_then(|a| a.ends),
        Some(ArrowEnds {
            start: Some("s1".into()),
            end: None,
            drawn: None,
        })
    );
}

#[test]
fn unknown_types_and_broken_objects_do_not_panic() {
    let other = object(
        json!({"id": "x", "type": "spinner-wheel"}),
        |_| String::new(),
        &HashMap::new(),
    );
    assert_eq!(other.map(|o| o.kind), Some(ObjectKind::Wheel));
    assert_eq!(
        object(
            json!({"type": "sticker"}),
            |_| String::new(),
            &HashMap::new()
        ),
        None
    );
    assert_eq!(
        object(json!("not an object"), |_| String::new(), &HashMap::new()),
        None
    );
}

#[test]
fn short_or_wordy_specs_name_no_object() {
    let ids = [
        "cafe1234-0000-4000-8000-000000000001",
        "bad00000-0000-4000-8000-000000000002",
        "10000000-0000-4000-8000-000000000003",
        "10000000-1111-4000-8000-000000000004",
    ];
    let named: Vec<&str> = ["Cafe", "bad", "1", "cafe123", "Planning", "cafe1234 x"]
        .into_iter()
        .filter(|spec| {
            !matches!(
                find_id(ids, spec),
                Err(Error::BoardChange(BoardChangeError::NoObject { .. }))
            )
        })
        .collect();
    assert_eq!(named, Vec::<&str>::new(), "these name no object");
    assert_eq!(
        find_id(ids, "CAFE1234").ok().as_deref(),
        ids.first().copied()
    );
    assert_eq!(
        find_id(ids, "bad00000-0000").ok().as_deref(),
        ids.get(1).copied()
    );
    assert!(matches!(
        find_id(ids, "10000000"),
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Object,
            miss: lookup::LookupMiss::Ambiguous { .. },
            ..
        }))
    ));
    assert!(matches!(
        find_id(ids, "deadbeef"),
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Object,
            miss: lookup::LookupMiss::NotFound { .. },
            ..
        }))
    ));
    assert_eq!(find_id(["s1"], "s1").ok().as_deref(), Some("s1"));
}

#[test]
fn note_text_joins_blocks_with_newlines() {
    let document = Doc::new();
    let documents = document.get_or_insert_map("documents");
    {
        // imperative: yrs inserts through one `&mut` transaction
        let mut txn = document.transact_mut();
        let root = documents.insert(&mut txn, "d1", XmlTextPrelim::new(""));
        let wrapper = root.insert_embed(&mut txn, 0, XmlTextPrelim::new(""));
        wrapper.insert_attribute(&mut txn, "type", "wrapper");
        ["first line", "second"]
            .iter()
            .enumerate()
            .for_each(|(at, line)| {
                let paragraph = wrapper.insert_embed(
                    &mut txn,
                    u32::try_from(at).unwrap_or(0),
                    XmlTextPrelim::new(*line),
                );
                paragraph.insert_attribute(&mut txn, "type", "paragraph");
            });
    }
    let txn = document.transact();
    assert_eq!(text_of(&txn, &documents, "d1"), "first line\nsecond");
    assert_eq!(text_of(&txn, &documents, "missing"), "");
}

mod kind_names {
    use rstest::rstest;
    use strum::IntoEnumIterator as _;

    use crate::domain::board::{KNOWN, ObjectKind};

    #[rstest]
    #[case::stored("simple-text", ObjectKind::Text)]
    #[case::short("text", ObjectKind::Text)]
    #[case::case_and_space("  Sticker ", ObjectKind::Sticker)]
    #[case::stack("sticker-stack", ObjectKind::StickerStack)]
    #[case::cell("TABLE-CELL", ObjectKind::TableCell)]
    #[case::image("image", ObjectKind::Image)]
    #[case::wheel("wheel", ObjectKind::Wheel)]
    #[case::wheel_stored("spinner-wheel", ObjectKind::Wheel)]
    #[case::document("document", ObjectKind::Document)]
    #[case::flipcard("flipcard", ObjectKind::Flipcard)]
    #[case::task("task", ObjectKind::Task)]
    #[case::unhandled(" Mind-Map-Node", ObjectKind::Other("mind-map-node".into()))]
    fn a_typed_kind_names_one_variant(#[case] typed: &str, #[case] kind: ObjectKind) {
        assert_eq!(ObjectKind::typed(typed), Some(kind));
    }

    #[rstest]
    #[case::empty("")]
    #[case::spaces(" \t ")]
    fn a_blank_kind_is_no_kind(#[case] typed: &str) {
        assert_eq!(ObjectKind::typed(typed), None);
    }

    #[test]
    fn every_variant_is_in_the_known_list() {
        let missing: Vec<ObjectKind> = ObjectKind::iter()
            .filter(|kind| !matches!(kind, ObjectKind::Other(_)))
            .filter(|kind| !KNOWN.contains(kind))
            .collect();
        assert_eq!(missing, Vec::new());
    }

    const LISTED: &str = "sticker, sticker-stack, text, shape, document, frame, arrow, \
        image, file, stamp, group, table, table-cell, code, drawing, dice, \
        wheel, flipcard, task, link";

    #[rstest]
    #[case::typo("stikcer")]
    #[case::plural("stickers")]
    fn a_kind_no_object_has_is_refused_with_the_kinds_listed(#[case] typed: &str) {
        let refused = ObjectKind::Other(typed.into())
            .check_among([ObjectKind::Sticker].into_iter())
            .err()
            .map(|err| err.to_string());
        assert_eq!(
            refused,
            Some(format!(
                "\"{typed}\" is not an object kind: expected {LISTED}"
            ))
        );
    }
}

#[test]
fn an_attached_arrow_is_listed_where_it_starts_not_at_the_corner() {
    let document = Doc::new();
    let objects = document.get_or_insert_map("objects");
    {
        // imperative: yrs inserts through one `&mut` transaction
        let mut txn = document.transact_mut();
        let all = [
            json!({"id": "s1", "type": "sticker", "position": {"x": 0, "y": 500},
                   "width": 192, "height": 192}),
            json!({"id": "s2", "type": "sticker", "position": {"x": 0, "y": 900},
                   "width": 192, "height": 192}),
            json!({"id": "a1", "type": "arrow", "position": {"x": 0, "y": 0},
                   "arrowType": "straight",
                   "start": {"objectId": "s1", "point": {"x": 0, "y": 0}},
                   "end": {"objectId": "s2", "point": {"x": 0, "y": 0}}}),
        ];
        // imperative: yrs inserts through one `&mut` transaction
        for fields in all {
            let value = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
            let id = fields.get("id").and_then(|id| id.as_str()).unwrap_or("");
            objects.insert(&mut txn, id, value);
        }
    }
    let ids: Vec<String> = crate::domain::board::objects(&document)
        .into_iter()
        .map(|object| object.id)
        .collect();
    assert_eq!(ids, ["s1", "a1", "s2"]);
}

#[test]
fn stored_card_flip_card_and_task_card_read_as_the_user_kinds() {
    let read = |name| ObjectKind::of(name);
    assert_eq!(read("card"), ObjectKind::Document);
    assert_eq!(read("flip-card"), ObjectKind::Flipcard);
    assert_eq!(read("task-card"), ObjectKind::Task);
    assert_eq!(read("task"), ObjectKind::Other("task".into()));
}

#[test]
fn listings_print_the_user_words_not_the_stored_names() {
    let printed: Vec<_> = ["card", "flip-card", "spinner-wheel", "task-card", "task"]
        .into_iter()
        .map(|name| serde_json::to_value(ObjectKind::of(name)).ok())
        .collect();
    assert_eq!(
        printed,
        ["document", "flipcard", "wheel", "task", "task"].map(|word| Some(json!(word)))
    );
}

/// A document holding one sticker `id` under the objects root named `root`.
pub(crate) fn sticker_under(root: &str, id: &str) -> Result<Doc> {
    let document = Doc::new();
    let fields = json!({
        "id": id, "type": "sticker", "position": { "x": 0, "y": 0 },
        "width": 100, "height": 100,
    });
    let objects = document.get_or_insert_map(root);
    insert_map(
        &mut document.transact_mut(),
        &objects,
        id,
        fields
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.clone())),
    )?;
    Ok(document)
}

#[test]
fn a_code_block_is_drawn_as_high_as_its_lines() -> Result<()> {
    let document = Doc::new();
    let objects_map = document.get_or_insert_map("objects");
    let fields = json!({
        "id": "c", "type": "code", "documentId": "d1",
        "position": { "x": 0, "y": 0 }, "width": 500,
    });
    insert_map(
        &mut document.transact_mut(),
        &objects_map,
        "c",
        fields
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.clone())),
    )?;
    document.get_or_insert_text("code-mirror-d1").insert(
        &mut document.transact_mut(),
        0,
        "a\nb\nc",
    );
    let listed = objects(&document);
    let height = listed
        .first()
        .and_then(|code| Scene::new(&listed).rectangle_of(code))
        .map(|rectangle| rectangle.height);
    assert_eq!(height, Some(87.0));
    Ok(())
}

#[test]
fn a_v3_document_lists_the_objects_under_objects_v3() -> Result<()> {
    let listed = objects(&sticker_under("objects_v3", "s")?);
    assert_eq!(
        listed.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        ["s"]
    );
    Ok(())
}

#[test]
fn reading_a_v2_document_creates_no_objects_v3_root() -> Result<()> {
    let document = sticker_under("objects", "s")?;
    let listed = objects(&document);
    assert_eq!(listed.len(), 1);
    assert!(document.transact().get_map("objects_v3").is_none());
    Ok(())
}
