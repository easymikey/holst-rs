#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::collections::BTreeMap;

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{Doc, Map as _, Transact as _, types::ToJson as _};

use super::add;
use crate::{
    domain::{
        board::{self, Object, ObjectKind},
        change::{
            build::build,
            tests::{ME, board as seeded, fields_of},
            value::{
                ArrowLook,
                BoardChange,
                ChangeSources,
                ChangeUpdate,
                LayerOrder,
                NewObject,
                ObjectEdit,
                ObjectPatch,
                Pasted,
            },
        },
        geometry::Point,
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn apply(document: &Doc, change: &BoardChange) -> Result<ChangeUpdate> {
    apply_as("m", document, change)
}

fn apply_as(
    prefix: &'static str,
    document: &Doc,
    change: &BoardChange,
) -> Result<ChangeUpdate> {
    let ids = SequentialIds::new(prefix);
    build(
        document,
        change,
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(9),
            ids: &ids,
        },
    )
}

fn edit_of(object: &str) -> ObjectEdit {
    ObjectEdit {
        object: object.to_owned(),
        patch: ObjectPatch::default(),
        stacking: None,
        tags: None,
    }
}

fn layer(document: &Doc, id: &str) -> f64 {
    fields_of(document, id)
        .get("zIndex")
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

fn tag_titles(document: &Doc) -> Vec<String> {
    let store = document.get_or_insert_map("task-card-tagsStore");
    let transaction = document.transact();
    store
        .iter(&transaction)
        .filter_map(|(_, definition)| board::json(&definition.to_json(&transaction)))
        .filter_map(|definition| {
            definition
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn an_edit_of_many_changes_each_object_in_one_update() -> Result<()> {
    let document = seeded()?;
    let built = apply(
        &document,
        &BoardChange::EditAll(vec![
            ObjectEdit {
                patch: ObjectPatch {
                    text: Some("first".into()),
                    ..ObjectPatch::default()
                },
                ..edit_of("seed-2")
            },
            ObjectEdit {
                patch: ObjectPatch {
                    text: Some("second".into()),
                    ..ObjectPatch::default()
                },
                ..edit_of("seed-6")
            },
        ]),
    )?;
    let texts: Vec<String> = objects_with(&document, &["seed-2", "seed-6"])
        .into_iter()
        .map(|object| object.text)
        .collect();
    assert_eq!(
        (
            built
                .objects
                .iter()
                .map(|object| object.id.as_str())
                .collect::<Vec<_>>(),
            texts
        ),
        (
            vec!["seed-2", "seed-6"],
            vec!["first".to_owned(), "second".to_owned()]
        )
    );
    Ok(())
}

#[test]
fn an_order_puts_an_object_past_all_the_others() -> Result<()> {
    let document = seeded()?;
    apply(
        &document,
        &BoardChange::EditAll(vec![
            ObjectEdit {
                stacking: Some(LayerOrder::Front),
                ..edit_of("seed-2")
            },
            ObjectEdit {
                stacking: Some(LayerOrder::Back),
                ..edit_of("seed-6")
            },
        ]),
    )?;
    let (frame, sticker, text) = (
        layer(&document, "seed-1"),
        layer(&document, "seed-2"),
        layer(&document, "seed-6"),
    );
    assert!(text < frame && frame < sticker, "{text} {frame} {sticker}");
    Ok(())
}

#[test]
fn a_frame_and_its_content_moved_in_one_edit_stay_together() -> Result<()> {
    let document = seeded()?;
    let at = |object: &str, x: f64, y: f64| -> Result<ObjectEdit> {
        Ok(ObjectEdit {
            patch: ObjectPatch {
                at: Some(Point::new(x, y)?),
                ..ObjectPatch::default()
            },
            ..edit_of(object)
        })
    };
    apply(
        &document,
        &BoardChange::EditAll(vec![
            at("seed-1", 2000.0, 0.0)?,
            at("seed-2", 2010.0, 10.0)?,
        ]),
    )?;
    let parent = fields_of(&document, "seed-2")
        .get("parentId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    assert_eq!(parent.as_deref(), Some("seed-1"));
    Ok(())
}

#[test]
fn content_put_into_a_frame_moves_with_it_in_the_same_edit() -> Result<()> {
    let document = seeded()?;
    let at = |object: &str, x: f64, y: f64| -> Result<ObjectEdit> {
        Ok(ObjectEdit {
            patch: ObjectPatch {
                at: Some(Point::new(x, y)?),
                ..ObjectPatch::default()
            },
            ..edit_of(object)
        })
    };
    apply(
        &document,
        &BoardChange::EditAll(vec![
            at("seed-6", 20.0, 20.0)?,
            at("seed-1", 3000.0, 0.0)?,
        ]),
    )?;
    let moved = fields_of(&document, "seed-6")
        .get("position")
        .and_then(|position| position.get("x"))
        .and_then(Value::as_f64);
    assert_eq!(moved, Some(3020.0));
    Ok(())
}

#[test]
fn tags_are_found_by_title_and_made_once() -> Result<()> {
    let document = seeded()?;
    let tagged = |object: &str, titles: &[&str]| ObjectEdit {
        tags: Some(titles.iter().map(|title| (*title).to_owned()).collect()),
        ..edit_of(object)
    };
    apply(
        &document,
        &BoardChange::EditAll(vec![tagged("seed-2", &["Urgent", "later"])]),
    )?;
    apply(
        &document,
        &BoardChange::EditAll(vec![tagged("seed-2", &["urgent"])]),
    )?;
    let ids = fields_of(&document, "seed-2")
        .get("tagIds")
        .and_then(Value::as_array)
        .map(Vec::len);
    let titles: Vec<String> = tag_titles(&document).into_iter().sorted().collect();
    assert_eq!(
        (ids, titles),
        (Some(1), vec!["Urgent".to_owned(), "later".to_owned()])
    );
    Ok(())
}

#[test]
fn an_empty_list_clears_the_tags_and_only_stickers_have_them() -> Result<()> {
    let document = seeded()?;
    let tags = |object: &str, titles: Vec<String>| {
        apply(
            &document,
            &BoardChange::EditAll(vec![ObjectEdit {
                tags: Some(titles),
                ..edit_of(object)
            }]),
        )
    };
    tags("seed-2", vec!["a".into()])?;
    tags("seed-2", Vec::new())?;
    let refused = tags("seed-6", vec!["a".into()]).err();
    assert_eq!(
        (
            fields_of(&document, "seed-2").get("tagIds").cloned(),
            matches!(
                refused,
                Some(Error::BoardChange(BoardChangeError::NotChangeable {
                    field: "tags",
                    ..
                }))
            )
        ),
        (Some(json!([])), true)
    );
    Ok(())
}

#[test]
fn an_edit_that_asks_for_nothing_is_refused() -> Result<()> {
    let document = seeded()?;
    let refused =
        apply(&document, &BoardChange::EditAll(vec![edit_of("seed-2")])).err();
    assert!(
        matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NothingToChange))
        ),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_removal_of_many_takes_each_object_with_its_content_once() -> Result<()> {
    let document = seeded()?;
    let built = apply(
        &document,
        &BoardChange::RemoveAll(vec![
            "seed-6".into(),
            "seed-1".into(),
            "seed-2".into(),
        ]),
    )?;
    assert_eq!(
        (
            built
                .objects
                .iter()
                .map(|object| object.id.as_str())
                .collect::<Vec<_>>(),
            board::objects(&document).len()
        ),
        (vec!["seed-6", "seed-1", "seed-2"], 0)
    );
    Ok(())
}

fn objects_with(document: &Doc, ids: &[&str]) -> Vec<Object> {
    let all = board::objects(document);
    ids.iter()
        .filter_map(|id| all.iter().find(|object| object.id == *id).cloned())
        .collect()
}

#[test]
fn a_paste_copies_a_frame_with_its_content_moved() -> Result<()> {
    let document = seeded()?;
    apply(
        &document,
        &BoardChange::Paste(Pasted {
            objects: objects_with(&document, &["seed-1", "seed-2"]),
            tags: BTreeMap::new(),
            shift: Point::new(1000.0, 0.0)?,
        }),
    )?;
    let copies: Vec<(String, f64, f64, Option<String>, String)> =
        board::objects(&document)
            .into_iter()
            .filter(|object| object.id.starts_with("m-"))
            .map(|object| (object.id, object.x, object.y, object.parent, object.text))
            .collect();
    assert_eq!(
        copies,
        vec![
            ("m-1".to_owned(), 1000.0, 0.0, None, "F".to_owned()),
            (
                "m-2".to_owned(),
                1010.0,
                10.0,
                Some("m-1".to_owned()),
                "a".to_owned()
            ),
        ]
    );
    assert!(layer(&document, "m-1") < layer(&document, "seed-1"));
    Ok(())
}

#[test]
fn a_pasted_arrow_is_bound_to_the_copies() -> Result<()> {
    let document = seeded()?;
    apply_as(
        "a",
        &document,
        &add(NewObject::Arrow {
            from: "seed-2".into(),
            to: "seed-6".into(),
            color: None,
            look: ArrowLook::default(),
        }),
    )?;
    let arrow = board::objects(&document)
        .into_iter()
        .find(|object| object.kind == ObjectKind::Arrow)
        .map(|object| vec![object]);
    let objects = [
        objects_with(&document, &["seed-2", "seed-6"]),
        arrow.unwrap_or_default(),
    ]
    .concat();
    apply(
        &document,
        &BoardChange::Paste(Pasted {
            objects,
            tags: BTreeMap::new(),
            shift: Point::new(0.0, 500.0)?,
        }),
    )?;
    let ends = board::objects(&document)
        .into_iter()
        .filter(|object| {
            object.kind == ObjectKind::Arrow && object.id.starts_with("m-")
        })
        .filter_map(|object| object.ends)
        .map(|ends| (ends.start, ends.end))
        .collect::<Vec<_>>();
    assert_eq!(ends, vec![(Some("m-1".to_owned()), Some("m-2".to_owned()))]);
    Ok(())
}

#[test]
fn pasted_tags_are_found_on_the_new_board_by_title() -> Result<()> {
    let document = seeded()?;
    let tagged = board::object_of(json!({
        "id": "x1", "type": "sticker", "position": {"x": 0, "y": 0}, "width": 192,
        "height": 192, "zIndex": 1, "documentId": "d1", "tagIds": ["t1"]
    }))
    .into_iter()
    .collect();
    apply(
        &document,
        &BoardChange::Paste(Pasted {
            objects: tagged,
            tags: BTreeMap::from([("t1".to_owned(), "Urgent".to_owned())]),
            shift: Point::new(0.0, 2000.0)?,
        }),
    )?;
    let copy = board::objects(&document)
        .into_iter()
        .find(|object| object.id.starts_with("m-"))
        .map(|object| object.fields.get("tagIds").cloned());
    assert_eq!(
        (
            tag_titles(&document),
            copy.flatten().map(|ids| ids.to_string().len() > 4)
        ),
        (vec!["Urgent".to_owned()], Some(true))
    );
    Ok(())
}
