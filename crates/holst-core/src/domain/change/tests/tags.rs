#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::{ArrayPrelim, Doc, Map as _, MapPrelim, Transact as _, types::ToJson as _};

use super::built_with_ids;
use crate::{
    domain::{
        board::{self, ObjectKind},
        change::{
            tests::{board as seeded, fields_of},
            value::{
                BoardChange,
                ChangeUpdate,
                ChangedObject,
                ObjectChangeKind,
                ObjectEdit,
                ObjectPatch,
                TagChange,
            },
        },
        id_source::SequentialIds,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn apply(document: &Doc, change: &BoardChange) -> Result<ChangeUpdate> {
    built_with_ids(document, &SequentialIds::new("m"), change)
}

fn tag(reference: &str, change: TagChange) -> BoardChange {
    BoardChange::Tag {
        tag: reference.to_owned(),
        change,
    }
}

fn store_json(document: &Doc, name: &str) -> Value {
    let store = document.get_or_insert_map(name);
    let transaction = document.transact();
    board::json(&store.to_json(&transaction)).unwrap_or_default()
}

/// The board with a sticker `seed-2` carrying the tags `Urgent` and `later`,
/// and a task info holding both too.
fn tagged() -> Result<Doc> {
    let document = seeded()?;
    apply(
        &document,
        &BoardChange::EditAll(vec![ObjectEdit {
            object: "seed-2".into(),
            patch: ObjectPatch::default(),
            stacking: None,
            tags: Some(vec!["Urgent".into(), "later".into()]),
        }]),
    )?;
    let ids = tag_ids(&document);
    let info = document.get_or_insert_map("task-card-infoStore");
    info.insert(
        &mut document.transact_mut(),
        "info-1",
        MapPrelim::from_iter([
            ("tagIds", ArrayPrelim::from_iter(ids)),
            ("assigneeIds", ArrayPrelim::from_iter(["u1".to_owned()])),
        ]),
    );
    Ok(document)
}

/// The ids of the tags `Urgent` and `later`, in that order.
fn tag_ids(document: &Doc) -> Vec<String> {
    ["Urgent", "later"]
        .map(|title| id_of(document, title))
        .to_vec()
}

fn id_of(document: &Doc, title: &str) -> String {
    store_json(document, "task-card-tagsStore")
        .as_object()
        .into_iter()
        .flatten()
        .find(|(_, definition)| definition.get("title") == Some(&json!(title)))
        .map(|(id, _)| id.clone())
        .unwrap_or_default()
}

/// The value at the JSON pointer `path` in the root map `name`.
fn stored(document: &Doc, name: &str, path: &str) -> Value {
    store_json(document, name)
        .pointer(path)
        .cloned()
        .unwrap_or_default()
}

fn the_tag(
    id: &str,
    change: ObjectChangeKind,
    fields: &[&'static str],
) -> ChangedObject {
    ChangedObject {
        change,
        fields: fields.to_vec(),
        ..ChangedObject::created(id.to_owned(), ObjectKind::Other("tag".into()))
    }
}

#[rstest]
#[case::rename(
    TagChange { name: Some("Soon".into()), ..TagChange::default() },
    ("Soon", "indigo5"),
    &["name"],
)]
#[case::recolour(
    TagChange { color: "red5".parse().ok(), ..TagChange::default() },
    ("Urgent", "red5"),
    &["color"],
)]
#[case::both(
    TagChange {
        name: Some("  Soon ".into()),
        color: "red5".parse().ok(),
        ..TagChange::default()
    },
    ("Soon", "red5"),
    &["name", "color"],
)]
fn a_tag_is_renamed_or_recoloured_by_its_title(
    #[case] change: TagChange,
    #[case] (title, color): (&str, &str),
    #[case] fields: &[&'static str],
) -> Result<()> {
    let document = tagged()?;
    let id = id_of(&document, "Urgent");
    let built = apply(&document, &tag("urgent", change))?;
    let at = |field: &str| {
        stored(&document, "task-card-tagsStore", &format!("/{id}/{field}"))
    };
    assert_eq!(
        (at("title"), at("color"), built.objects),
        (
            json!(title),
            json!(color),
            vec![the_tag(&id, ObjectChangeKind::Changed, fields)]
        )
    );
    Ok(())
}

#[test]
fn a_tag_is_found_by_its_id_and_may_change_its_case() -> Result<()> {
    let document = tagged()?;
    let id = id_of(&document, "Urgent");
    apply(
        &document,
        &tag(
            &id,
            TagChange {
                name: Some("URGENT".into()),
                ..TagChange::default()
            },
        ),
    )?;
    assert_eq!(id_of(&document, "URGENT"), id);
    Ok(())
}

#[rstest]
#[case::same("later")]
#[case::other_case(" LATER ")]
fn a_rename_onto_the_title_of_another_tag_is_refused(#[case] name: &str) -> Result<()> {
    let document = tagged()?;
    let before = store_json(&document, "task-card-tagsStore");
    let refused = apply(
        &document,
        &tag(
            "Urgent",
            TagChange {
                name: Some(name.into()),
                ..TagChange::default()
            },
        ),
    )
    .err();
    assert!(
        matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::TagTaken { .. }))
        ),
        "{refused:?}"
    );
    assert_eq!(store_json(&document, "task-card-tagsStore"), before);
    Ok(())
}

#[test]
fn removing_a_tag_clears_every_sticker_and_task_reference() -> Result<()> {
    let document = tagged()?;
    let [urgent, later] = [id_of(&document, "Urgent"), id_of(&document, "later")];
    let built = apply(
        &document,
        &tag(
            "urgent",
            TagChange {
                remove: true,
                ..TagChange::default()
            },
        ),
    )?;
    let sticker = ChangedObject {
        id: "seed-2".into(),
        kind: ObjectKind::Sticker,
        change: ObjectChangeKind::Changed,
        fields: vec!["tags"],
        link: None,
    };
    assert_eq!(
        (
            store_json(&document, "task-card-tagsStore")
                .as_object()
                .map(|tags| tags.keys().cloned().collect::<Vec<_>>()),
            fields_of(&document, "seed-2")
                .get("tagIds")
                .cloned()
                .unwrap_or_default(),
            stored(&document, "task-card-infoStore", "/info-1"),
            built.objects,
        ),
        (
            Some(vec![later.clone()]),
            json!([later]),
            json!({"tagIds": [later], "assigneeIds": ["u1"]}),
            vec![the_tag(&urgent, ObjectChangeKind::Removed, &[]), sticker],
        )
    );
    Ok(())
}

#[test]
fn an_unknown_tag_is_refused() -> Result<()> {
    let document = tagged()?;
    let refused = apply(
        &document,
        &tag(
            "nobody",
            TagChange {
                remove: true,
                ..TagChange::default()
            },
        ),
    )
    .err();
    assert!(
        matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NoTag { .. }))
        ),
        "{refused:?}"
    );
    Ok(())
}

#[rstest]
#[case::nothing(TagChange::default())]
#[case::blank_name(TagChange { name: Some("  ".into()), ..TagChange::default() })]
fn a_change_that_asks_nothing_is_refused(#[case] change: TagChange) -> Result<()> {
    let document = tagged()?;
    let refused = apply(&document, &tag("urgent", change)).err();
    assert!(
        matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NothingToChange))
        ),
        "{refused:?}"
    );
    Ok(())
}
