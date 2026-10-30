#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{
    ArrayRef,
    Doc,
    GetString as _,
    Map as _,
    MapPrelim,
    Text as _,
    Transact as _,
    types::{ToJson as _, text::YChange},
};

use super::{ME, add, add_in, board, built, built_with_ids, fields_of};
use crate::{
    domain::{
        board::{self as stored, ObjectKind, document_root, json as stored_json},
        change::{
            build::build,
            value::{
                BoardChange,
                ChangeSources,
                Made,
                NewEntry,
                NewObject,
                StoreEntry,
            },
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        rich_text::{Kept, TextFormat},
        time::Timestamp,
    },
    error::Result,
};

fn made(kind: ObjectKind, x: f64, y: f64) -> Result<Made> {
    Ok(Made {
        kind,
        at: Point::new(x, y)?,
        size: Some(Size::new(50.0, 50.0)?),
        parent: None,
        fields: Vec::new(),
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    })
}

/// The fields of the object without the header every new object has.
fn bare(fields: &Value) -> Value {
    let header = ["id", "zIndex", "created", "updated"];
    Value::Object(
        fields
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, _)| !header.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

fn entry(store: &'static str, id: &str, keep: bool) -> StoreEntry {
    StoreEntry {
        store,
        id: id.into(),
        fields: vec![("title", json!("new"))],
        lists: Vec::new(),
        keep,
    }
}

fn kanban() -> Result<Made> {
    let code = Made {
        code: Some("let x = 1;".into()),
        ..made(ObjectKind::Code, 5.0, 5.0)?
    };
    let sticker = Made {
        texts: vec![("documentId", "inner".into())],
        ..made(ObjectKind::Sticker, 6.0, 6.0)?
    };
    Ok(Made {
        fields: vec![("title", json!("K"))],
        texts: vec![
            ("documentId", "hello".into()),
            ("titleDocumentId", "heading".into()),
        ],
        parts: vec![code, sticker],
        entries: vec![
            entry("task-card-statusesStore", "todo", true),
            entry("task-card-statusesStore", "done", true),
            StoreEntry {
                lists: vec![("cardIds", vec![json!("x"), json!("y")])],
                ..entry("kanban-columnsStore", "c1", false)
            },
        ],
        ..made(ObjectKind::Other("kanban".into()), 10.0, 20.0)?
    })
}

fn store_json(document: &Doc, name: &str) -> Value {
    let store = document.get_or_insert_map(name);
    let transaction = document.transact();
    stored_json(&store.to_json(&transaction)).unwrap_or_default()
}

#[test]
fn a_made_object_is_written_with_texts_parts_code_and_entries() -> Result<()> {
    let document = Doc::new();
    let seeded = document.get_or_insert_map("task-card-statusesStore");
    seeded.insert(
        &mut document.transact_mut(),
        "todo",
        MapPrelim::from_iter([("title", "mine")]),
    );
    let update = built(&document, &add(NewObject::Made(kanban()?)))?;
    let id = update.objects.first().map(|object| object.id.clone());
    let id = id.unwrap_or_default();
    assert_eq!(
        update
            .objects
            .iter()
            .map(|object| (object.kind.clone(), object.fields.clone()))
            .collect::<Vec<_>>(),
        [(ObjectKind::Other("kanban".into()), Vec::new())]
    );
    let main = bare(&fields_of(&document, &id));
    let documents = document.get_or_insert_map("documents");
    let transaction = document.transact();
    let blocks = |field: &str| {
        main.get(field)
            .and_then(Value::as_str)
            .and_then(|name| document_root(&transaction, &documents, name))
            .map(|root| root.diff(&transaction, YChange::identity).len())
    };
    assert_eq!(
        (
            main.get("type").cloned(),
            main.get("position").cloned(),
            main.get("title").cloned(),
            main.get("parentId").cloned(),
        ),
        (
            Some(json!("kanban")),
            Some(json!({"x": 10, "y": 20})),
            Some(json!("K")),
            // The web client stores no `parentId` on a kanban outside a frame.
            None
        )
    );
    assert_eq!(
        (blocks("documentId"), blocks("titleDocumentId")),
        (Some(1), Some(1))
    );
    drop(transaction);
    assert_eq!(
        stored::objects(&document)
            .into_iter()
            .find(|object| object.id == id)
            .map(|object| object.text),
        Some("hello".into())
    );
    let held: Vec<_> = stored::objects(&document)
        .into_iter()
        .filter(|object| object.parent.as_deref() == Some(id.as_str()))
        .collect();
    assert_eq!(
        held.iter()
            .map(|part| (part.kind.clone(), part.text.clone()))
            .sorted_by_key(|(_, text)| text.clone())
            .collect::<Vec<_>>(),
        [
            (ObjectKind::Sticker, "inner".into()),
            (ObjectKind::Code, "let x = 1;".into())
        ]
    );
    Ok(())
}

#[test]
fn the_code_of_a_part_is_a_root_text_named_by_its_document() -> Result<()> {
    let document = Doc::new();
    built(&document, &add(NewObject::Made(kanban()?)))?;
    let code = stored::objects(&document)
        .into_iter()
        .find(|object| object.kind == ObjectKind::Code)
        .map(|object| object.id)
        .unwrap_or_default();
    let name = fields_of(&document, &code)
        .get("documentId")
        .and_then(Value::as_str)
        .map(|document| format!("code-mirror-{document}"))
        .unwrap_or_default();
    let text = document.get_or_insert_text(name.as_str());
    assert_eq!(text.get_string(&document.transact()), "let x = 1;");
    Ok(())
}

#[test]
fn entries_go_to_their_stores_and_a_kept_one_stays() -> Result<()> {
    let document = Doc::new();
    let seeded = document.get_or_insert_map("task-card-statusesStore");
    seeded.insert(
        &mut document.transact_mut(),
        "todo",
        MapPrelim::from_iter([("title", "mine")]),
    );
    built(&document, &add(NewObject::Made(kanban()?)))?;
    assert_eq!(
        store_json(&document, "task-card-statusesStore"),
        json!({"todo": {"title": "mine"}, "done": {"id": "done", "title": "new"}})
    );
    let columns = document.get_or_insert_map("kanban-columnsStore");
    let transaction = document.transact();
    let list = columns
        .get(&transaction, "c1")
        .and_then(|column| column.cast::<yrs::MapRef>().ok())
        .and_then(|column| column.get(&transaction, "cardIds"))
        .and_then(|cards| cards.cast::<ArrayRef>().ok())
        .map(|cards| stored_json(&cards.to_json(&transaction)));
    assert_eq!(list, Some(Some(json!(["x", "y"]))));
    Ok(())
}

#[test]
fn a_made_object_with_a_parent_is_held_by_it_and_not_by_a_frame() -> Result<()> {
    let document = board()?;
    let inside = Point::new(20.0, 20.0)?;
    let free = Made {
        at: inside,
        ..made(ObjectKind::Other("node".into()), 0.0, 0.0)?
    };
    let held = Made {
        parent: Some("seed-2".into()),
        ..free.clone()
    };
    let ids = SequentialIds::new("m");
    let parent_of = |id: &str| fields_of(&document, id).get("parentId").cloned();
    built_with_ids(&document, &ids, &add(NewObject::Made(free)))?;
    built_with_ids(&document, &ids, &add(NewObject::Made(held)))?;
    assert_eq!(
        (parent_of("m-1"), parent_of("m-2")),
        (Some(json!("seed-1")), Some(json!("seed-2")))
    );
    Ok(())
}

#[test]
fn a_made_object_in_a_frame_is_placed_from_its_corner() -> Result<()> {
    let document = board()?;
    let inside = Made {
        at: Point::new(30.0, 40.0)?,
        ..made(ObjectKind::Other("node".into()), 0.0, 0.0)?
    };
    let ids = SequentialIds::new("m");
    built_with_ids(
        &document,
        &ids,
        &add_in(NewObject::Made(inside), "seed-1".into()),
    )?;
    let fields = fields_of(&document, "m-1");
    assert_eq!(
        (fields.get("parentId"), fields.get("position")),
        (Some(&json!("seed-1")), Some(&json!({"x": 30, "y": 40})))
    );
    Ok(())
}

#[test]
fn the_markdown_of_a_part_reports_what_it_keeps() -> Result<()> {
    let quote = Made {
        texts: vec![("documentId", "> quoted\n".into())],
        ..made(ObjectKind::Sticker, 1.0, 1.0)?
    };
    let whole = Made {
        parts: vec![quote],
        ..made(ObjectKind::Other("holder".into()), 0.0, 0.0)?
    };
    let change = BoardChange::AddAll(vec![NewEntry {
        new: NewObject::Made(whole),
        frame: None,
        format: TextFormat::Markdown,
    }]);
    let ids = SequentialIds::new("k");
    let built = build(
        &Doc::new(),
        &change,
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(1),
            ids: &ids,
        },
    )?;
    assert_eq!(built.kept, [Kept::Quote].into());
    Ok(())
}
