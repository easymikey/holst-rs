#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, fields_of, frame::seed};
use crate::{
    domain::{
        board::ObjectKind,
        change::{
            build::build,
            value::{BoardChange, ChangeSources, ChangeUpdate, ObjectChangeKind},
        },
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::Result,
};

fn sticker(id: &str, parent: Option<&str>, x: u32, y: u32) -> Value {
    json!({
        "id": id, "type": "sticker", "parentId": parent,
        "position": { "x": x, "y": y }, "width": 100, "height": 100, "zIndex": 1000,
    })
}

/// A frame holding the sticker `a`, free stickers `b` and `c`, a table cell.
fn board() -> Result<Doc> {
    let document = Doc::new();
    seed(
        &document,
        &json!({
            "id": "F", "type": "frame", "position": { "x": 0, "y": 0 },
            "width": 800, "height": 600, "zIndex": 1,
        }),
    )?;
    seed(&document, &sticker("a", Some("F"), 10, 20))?;
    seed(&document, &sticker("b", None, 1000, 0))?;
    seed(&document, &sticker("c", None, 1200, 0))?;
    seed(&document, &json!({ "id": "tbl", "type": "table" }))?;
    seed(
        &document,
        &json!({ "id": "cell", "type": "table-cell", "parentId": "tbl" }),
    )?;
    Ok(document)
}

/// A locked group `G` of the stickers `m` and `n`, a locked frame `L`.
fn locked_board() -> Result<Doc> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "G", "type": "group", "locked": true }),
    )?;
    seed(&document, &sticker("m", Some("G"), 0, 0))?;
    seed(&document, &sticker("n", Some("G"), 100, 0))?;
    seed(
        &document,
        &json!({ "id": "L", "type": "frame", "locked": true, "width": 10, "height": 10 }),
    )?;
    Ok(document)
}

fn built(document: &Doc, change: &BoardChange) -> Result<ChangeUpdate> {
    build(
        document,
        change,
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(2),
            ids: &SequentialIds::new("g"),
        },
    )
}

fn named(objects: &[&str]) -> Vec<String> {
    objects.iter().map(|&one| one.to_owned()).collect()
}

fn refusal(document: &Doc, change: &BoardChange) -> Option<String> {
    built(document, change).err().map(|error| error.to_string())
}

fn field(document: &Doc, id: &str, name: &str) -> Option<Value> {
    fields_of(document, id).get(name).cloned()
}

#[test]
fn one_object_is_locked_in_a_group_of_its_own() -> Result<()> {
    let document = board()?;
    let update = built(&document, &BoardChange::Lock(named(&["b"])))?;
    assert_eq!(field(&document, "g-1", "locked"), Some(json!(true)));
    assert_eq!(field(&document, "g-1", "type"), Some(json!("group")));
    assert_eq!(field(&document, "b", "parentId"), Some(json!("g-1")));
    assert_eq!(
        update
            .objects
            .iter()
            .map(|touched| (touched.id.as_str(), touched.change))
            .collect::<Vec<_>>(),
        vec![
            ("g-1", ObjectChangeKind::Created),
            ("b", ObjectChangeKind::Changed),
        ]
    );
    Ok(())
}

#[test]
fn a_frame_is_locked_with_what_it_holds() -> Result<()> {
    let document = board()?;
    built(&document, &BoardChange::Lock(named(&["F", "a", "b"])))?;
    assert_eq!(field(&document, "F", "parentId"), Some(json!("g-1")));
    assert_eq!(field(&document, "b", "parentId"), Some(json!("g-1")));
    assert_eq!(field(&document, "a", "parentId"), Some(json!("F")));
    Ok(())
}

#[test]
fn a_locked_object_is_kept_from_a_removal() -> Result<()> {
    let document = board()?;
    built(&document, &BoardChange::Lock(named(&["b", "c"])))?;
    let removal = BoardChange::RemoveAll(named(&["b"]));
    assert_eq!(
        refusal(&document, &removal),
        Some(
            "\"b\" is locked on the board; unlock it in the web client first"
                .to_owned()
        )
    );
    Ok(())
}

#[test]
fn what_cannot_be_locked_is_refused() -> Result<()> {
    let document = locked_board()?;
    let said = |names: &[&str]| refusal(&document, &BoardChange::Lock(named(names)));
    assert_eq!(
        said(&["cell"]),
        Some("\"cell\" cannot be locked: a table cell stays in its table".to_owned())
    );
    assert_eq!(
        said(&["L"]),
        Some("\"L\" cannot be locked: it is locked already".to_owned())
    );
    assert_eq!(
        said(&["G"]),
        Some("\"G\" cannot be locked: it is locked already".to_owned())
    );
    Ok(())
}

#[test]
fn only_a_frame_or_a_table_is_flagged_by_itself() -> Result<()> {
    let document = board()?;
    let update = built(&document, &BoardChange::LockOnly(named(&["F", "tbl"])))?;
    assert_eq!(field(&document, "F", "locked"), Some(json!(true)));
    assert_eq!(field(&document, "tbl", "locked"), Some(json!(true)));
    assert_eq!(field(&document, "a", "parentId"), Some(json!("F")));
    assert_eq!(
        update
            .objects
            .iter()
            .map(|touched| (
                touched.id.as_str(),
                touched.kind.clone(),
                touched.fields.clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("F", ObjectKind::Frame, vec!["locked"]),
            ("tbl", ObjectKind::Table, vec!["locked"]),
        ]
    );
    Ok(())
}

#[test]
fn only_refuses_a_sticker() -> Result<()> {
    let document = board()?;
    assert_eq!(
        refusal(&document, &BoardChange::LockOnly(named(&["F", "b"]))),
        Some(
            "\"b\" cannot be locked: --only locks frames and tables, nothing else"
                .to_owned()
        )
    );
    assert_eq!(field(&document, "F", "locked"), None);
    Ok(())
}

#[test]
fn unlocking_a_group_dissolves_it_and_a_frame_is_flagged_free() -> Result<()> {
    let document = locked_board()?;
    built(&document, &BoardChange::Unlock(named(&["G", "L"])))?;
    assert_eq!(field(&document, "m", "parentId"), Some(Value::Null));
    assert_eq!(field(&document, "n", "parentId"), Some(Value::Null));
    assert_eq!(field(&document, "G", "type"), None);
    assert_eq!(field(&document, "L", "locked"), Some(json!(false)));
    Ok(())
}

#[test]
fn an_unlocked_object_is_not_unlocked() -> Result<()> {
    let document = locked_board()?;
    let said = |names: &[&str]| refusal(&document, &BoardChange::Unlock(named(names)));
    let not_locked =
        "is not locked itself: only a locked group, frame or table can be unlocked";
    assert_eq!(said(&["b"]), Some(format!("\"b\" {not_locked}")));
    assert_eq!(said(&["m"]), Some(format!("\"m\" {not_locked}")));
    assert_eq!(said(&["F"]), Some(format!("\"F\" {not_locked}")));
    Ok(())
}

#[test]
fn unlock_all_frees_everything_locked() -> Result<()> {
    let document = locked_board()?;
    built(&document, &BoardChange::UnlockAll)?;
    assert_eq!(field(&document, "G", "type"), None);
    assert_eq!(field(&document, "L", "locked"), Some(json!(false)));
    assert_eq!(
        refusal(&document, &BoardChange::UnlockAll),
        Some("nothing is locked on the board: nothing was changed".to_owned())
    );
    Ok(())
}

#[test]
fn a_locked_group_unlocked_in_a_locked_group_leaves_its_members_in_the_outer_one()
-> Result<()> {
    let document = locked_board()?;
    seed(
        &document,
        &json!({ "id": "H", "type": "group", "locked": true }),
    )?;
    seed(
        &document,
        &json!({ "id": "G2", "type": "group", "locked": true, "parentId": "H" }),
    )?;
    seed(&document, &sticker("p", Some("G2"), 0, 0))?;
    built(&document, &BoardChange::Unlock(named(&["G2"])))?;
    assert_eq!(field(&document, "p", "parentId"), Some(json!("H")));
    assert_eq!(field(&document, "G2", "type"), None);
    Ok(())
}
