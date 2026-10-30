#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::Doc;

use super::{built, fields_of, frame::seed, remove, sticker_at};
use crate::{
    domain::{
        board::ObjectKind,
        change::value::{BoardChange, ObjectChangeKind},
    },
    error::Result,
};

/// A frame holding stickers `a` and `b`, and a free sticker `c`.
fn board() -> Result<Doc> {
    let document = Doc::new();
    seed(
        &document,
        &json!({
            "id": "F", "type": "frame", "position": { "x": 0, "y": 0 },
            "width": 800, "height": 600, "zIndex": 1}),
    )?;
    seed(&document, &sticker_at("a", Some("F"), 10, 20, 1000))?;
    seed(&document, &sticker_at("b", Some("F"), 300, 50, 1010))?;
    seed(&document, &sticker_at("c", None, 1000, 0, 1020))?;
    Ok(document)
}

fn group_of(objects: &[&str]) -> BoardChange {
    BoardChange::Group(objects.iter().map(|&one| one.to_owned()).collect())
}

fn ungroup_of(objects: &[&str]) -> BoardChange {
    BoardChange::Ungroup(objects.iter().map(|&one| one.to_owned()).collect())
}

fn refusal(document: &Doc, change: &BoardChange) -> Option<String> {
    built(document, change).err().map(|error| error.to_string())
}

fn parent(document: &Doc, id: &str) -> Option<Value> {
    fields_of(document, id).get("parentId").cloned()
}

#[test]
fn a_group_sits_in_the_frame_of_its_members_at_their_corner() -> Result<()> {
    let document = board()?;
    let update = built(&document, &group_of(&["a", "b"]))?;
    let group = fields_of(&document, "g-1");
    assert_eq!(group.get("type"), Some(&json!("group")));
    assert_eq!(group.get("ignoreZIndex"), Some(&json!(true)));
    assert_eq!(group.get("parentId"), Some(&json!("F")));
    assert_eq!(group.get("position"), Some(&json!({ "x": 10, "y": 20 })));
    assert_eq!(group.get("zIndex"), Some(&json!(1010)));
    assert_eq!(parent(&document, "a"), Some(json!("g-1")));
    assert_eq!(parent(&document, "b"), Some(json!("g-1")));
    assert_eq!(
        fields_of(&document, "a").get("position"),
        Some(&json!({ "x": 10, "y": 20 }))
    );
    assert_eq!(
        update
            .objects
            .iter()
            .map(|touched| (
                touched.id.as_str(),
                touched.change,
                touched.fields.clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("g-1", ObjectChangeKind::Created, vec![]),
            ("a", ObjectChangeKind::Changed, vec!["parent"]),
            ("b", ObjectChangeKind::Changed, vec!["parent"]),
        ]
    );
    Ok(())
}

#[test]
fn members_of_different_parents_make_a_group_on_the_board() -> Result<()> {
    let document = board()?;
    built(&document, &group_of(&["a", "c"]))?;
    let group = fields_of(&document, "g-1");
    assert_eq!(group.get("parentId"), None);
    assert_eq!(group.get("position"), Some(&json!({ "x": 10, "y": 0 })));
    assert_eq!(parent(&document, "c"), Some(json!("g-1")));
    Ok(())
}

#[test]
fn a_group_around_a_group_and_a_sticker_sits_at_the_corner_of_their_box() -> Result<()>
{
    let document = board()?;
    seed(&document, &json!({ "id": "G", "type": "group" }))?;
    seed(&document, &sticker_at("s", Some("G"), 50, 60, 5))?;
    built(&document, &group_of(&["G", "c"]))?;
    let group = fields_of(&document, "g-1");
    assert_eq!(group.get("position"), Some(&json!({ "x": 50, "y": 0 })));
    Ok(())
}

#[test]
fn a_group_around_two_groups_sits_at_the_corner_of_their_box() -> Result<()> {
    let document = board()?;
    seed(&document, &json!({ "id": "G1", "type": "group" }))?;
    seed(&document, &sticker_at("s1", Some("G1"), 50, 60, 5))?;
    seed(&document, &json!({ "id": "G2", "type": "group" }))?;
    seed(&document, &sticker_at("s2", Some("G2"), 300, 10, 6))?;
    built(&document, &group_of(&["G1", "G2"]))?;
    let group = fields_of(&document, "g-1");
    assert_eq!(group.get("position"), Some(&json!({ "x": 50, "y": 10 })));
    Ok(())
}

#[test]
fn a_group_in_a_table_takes_the_cell_of_its_first_member() -> Result<()> {
    let document = board()?;
    seed(&document, &json!({ "id": "T", "type": "table" }))?;
    let cell = json!({ "rowId": "r1", "columnId": "c1" });
    seed(
        &document,
        &json!({
            "id": "x", "type": "sticker", "parentId": "T", "zIndex": 1,
            "additionalPositionInfo": cell, "position": { "x": 5, "y": 5 },
            "width": 10, "height": 10}),
    )?;
    seed(&document, &sticker_at("y", Some("T"), 50, 5, 2))?;
    built(&document, &group_of(&["x", "y"]))?;
    let group = fields_of(&document, "g-1");
    assert_eq!(group.get("parentId"), Some(&json!("T")));
    assert_eq!(group.get("additionalPositionInfo"), Some(&cell));
    Ok(())
}

#[test]
fn what_lies_in_another_named_group_member_goes_with_it() -> Result<()> {
    let document = board()?;
    seed(&document, &json!({ "id": "G", "type": "group" }))?;
    seed(&document, &sticker_at("s", Some("G"), 0, 0, 5))?;
    let update = built(&document, &group_of(&["G", "s", "c"]))?;
    assert_eq!(parent(&document, "G"), Some(json!("g-1")));
    assert_eq!(parent(&document, "c"), Some(json!("g-1")));
    assert_eq!(parent(&document, "s"), Some(json!("G")));
    assert_eq!(update.objects.len(), 3);
    Ok(())
}

#[rstest]
#[case::frame(json!({ "id": "x", "type": "frame" }), "\"x\" cannot be grouped: a frame holds its content itself")]
#[case::table_cell(json!({ "id": "x", "type": "table-cell", "parentId": "T" }), "\"x\" cannot be grouped: a table cell stays in its table")]
#[case::mind_map_node(json!({ "id": "x", "type": "mind-map-node" }), "\"x\" cannot be grouped: a mind-map node stays in its map")]
#[case::in_a_group(
    sticker_at("x", Some("G"), 0, 0, 1),
    "\"x\" cannot be grouped: it is in a group already; ungroup that one first"
)]
#[case::in_a_column(json!({ "id": "x", "type": "task-card", "parentId": "K", "columnId": "c1" }), "\"x\" cannot be grouped: a card in a kanban column stays in it")]
fn a_kind_the_web_keeps_out_of_groups_is_refused(
    #[case] member: Value,
    #[case] words: &str,
) -> Result<()> {
    let document = board()?;
    seed(&document, &json!({ "id": "T", "type": "table" }))?;
    seed(&document, &json!({ "id": "G", "type": "group" }))?;
    seed(&document, &json!({ "id": "K", "type": "kanban" }))?;
    seed(&document, &member)?;
    assert_eq!(
        refusal(&document, &group_of(&["a", "x"])).as_deref(),
        Some(words)
    );
    assert_eq!(parent(&document, "a"), Some(json!("F")));
    Ok(())
}

#[rstest]
#[case::one(&["a"], "a group needs at least 2 objects, not 1: nothing was changed")]
#[case::twice_the_same(&["a", "a"], "a group needs at least 2 objects, not 1: nothing was changed")]
fn a_group_needs_two_objects(
    #[case] objects: &[&str],
    #[case] words: &str,
) -> Result<()> {
    let document = board()?;
    assert_eq!(
        refusal(&document, &group_of(objects)).as_deref(),
        Some(words)
    );
    Ok(())
}

#[test]
fn an_ungroup_leaves_the_members_in_the_parent_of_the_group() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "G", "type": "group", "parentId": "F", "ignoreZIndex": true }),
    )?;
    seed(&document, &sticker_at("s", Some("G"), 1, 1, 5))?;
    seed(&document, &sticker_at("t", Some("G"), 2, 2, 6))?;
    let update = built(&document, &ungroup_of(&["G"]))?;
    assert_eq!(parent(&document, "s"), Some(json!("F")));
    assert_eq!(parent(&document, "t"), Some(json!("F")));
    assert_eq!(fields_of(&document, "G"), Value::Null);
    assert_eq!(
        update
            .objects
            .iter()
            .map(|touched| (touched.id.as_str(), touched.change))
            .collect::<Vec<_>>(),
        vec![
            ("s", ObjectChangeKind::Changed),
            ("t", ObjectChangeKind::Changed),
            ("G", ObjectChangeKind::Removed),
        ]
    );
    Ok(())
}

#[test]
fn an_ungroup_on_the_board_leaves_members_without_a_parent() -> Result<()> {
    let document = board()?;
    seed(&document, &json!({ "id": "G", "type": "group" }))?;
    seed(&document, &sticker_at("s", Some("G"), 1, 1, 5))?;
    built(&document, &ungroup_of(&["G"]))?;
    assert_eq!(parent(&document, "s"), Some(Value::Null));
    Ok(())
}

#[test]
fn nested_groups_ungrouped_together_land_in_the_outer_parent() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "G1", "type": "group", "parentId": "F" }),
    )?;
    seed(
        &document,
        &json!({ "id": "G2", "type": "group", "parentId": "G1" }),
    )?;
    seed(&document, &sticker_at("s", Some("G2"), 1, 1, 5))?;
    built(&document, &ungroup_of(&["G2", "G1"]))?;
    assert_eq!(parent(&document, "s"), Some(json!("F")));
    assert_eq!(fields_of(&document, "G1"), Value::Null);
    assert_eq!(fields_of(&document, "G2"), Value::Null);
    Ok(())
}

#[test]
fn only_an_unlocked_group_can_be_ungrouped() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "G", "type": "group", "locked": true }),
    )?;
    assert_eq!(
        refusal(&document, &ungroup_of(&["G"])).as_deref(),
        Some("\"G\" is locked on the board; unlock it in the web client first")
    );
    assert_eq!(
        refusal(&document, &ungroup_of(&["a"])).as_deref(),
        Some("\"a\" is not a group: only groups can be ungrouped")
    );
    Ok(())
}

fn group_with(document: &Doc, members: &[&str]) -> Result<()> {
    seed(
        document,
        &json!({ "id": "G", "type": "group", "parentId": "F" }),
    )?;
    members
        .iter()
        .zip(5..)
        .try_for_each(|(&id, z)| seed(document, &sticker_at(id, Some("G"), 1, 1, z)))
}

#[rstest]
#[case::one_left(&["s", "t"], &["s"], Some("F"), true)]
#[case::all_gone(&["s", "t"], &["s", "t"], None, true)]
#[case::two_left(&["s", "t", "u"], &["s"], Some("G"), false)]
fn a_group_short_of_members_dissolves_as_in_the_web(
    #[case] members: &[&str],
    #[case] removed: &[&str],
    #[case] survivor_parent: Option<&str>,
    #[case] dissolved: bool,
) -> Result<()> {
    let document = board()?;
    group_with(&document, members)?;
    let update = built(
        &document,
        &BoardChange::RemoveAll(removed.iter().map(|&one| one.to_owned()).collect()),
    )?;
    let survivor = members.iter().find(|member| !removed.contains(member));
    assert_eq!(
        survivor.map(|id| parent(&document, id)),
        survivor_parent.map(|parent| Some(json!(parent)))
    );
    assert_eq!(fields_of(&document, "G") == Value::Null, dissolved);
    assert_eq!(
        update
            .objects
            .iter()
            .any(|touched| touched.id == "G" && touched.kind == ObjectKind::Group),
        dissolved
    );
    Ok(())
}

#[test]
fn a_group_dissolved_by_a_removal_may_leave_its_own_group_short() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "G1", "type": "group", "parentId": "F" }),
    )?;
    seed(
        &document,
        &json!({ "id": "G2", "type": "group", "parentId": "G1" }),
    )?;
    seed(&document, &sticker_at("s", Some("G2"), 1, 1, 5))?;
    seed(&document, &sticker_at("t", Some("G2"), 2, 2, 6))?;
    built(&document, &remove("s".into()))?;
    assert_eq!(parent(&document, "t"), Some(json!("F")));
    assert_eq!(fields_of(&document, "G1"), Value::Null);
    assert_eq!(fields_of(&document, "G2"), Value::Null);
    Ok(())
}

#[test]
fn removing_a_whole_group_removes_it_once() -> Result<()> {
    let document = board()?;
    group_with(&document, &["s", "t"])?;
    let update = built(&document, &remove("G".into()))?;
    let removed: Vec<_> = update
        .objects
        .iter()
        .filter(|touched| touched.id == "G")
        .collect();
    assert_eq!(removed.len(), 1);
    Ok(())
}
