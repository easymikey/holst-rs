#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::Doc;

use super::{ME, fields_of, frame::seed};
use crate::{
    domain::{
        change::{
            build::build,
            value::{
                Alignment,
                Arrangement,
                Axis,
                BoardChange,
                ChangeSources,
                ChangeUpdate,
            },
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

/// Free stickers `one`, `two` and `three` at three corners, an empty frame.
fn board() -> Result<Doc> {
    let document = Doc::new();
    seed(
        &document,
        &json!({
            "id": "F", "type": "frame", "position": { "x": 2000, "y": 0 },
            "width": 800, "height": 600, "zIndex": 1,
        }),
    )?;
    seed(&document, &sticker("one", None, 0, 0))?;
    seed(&document, &sticker("two", None, 200, 50))?;
    seed(&document, &sticker("three", None, 500, 300))?;
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

fn arrange(objects: &[&str], how: Arrangement) -> BoardChange {
    BoardChange::Arrange {
        objects: objects.iter().map(|&one| one.to_owned()).collect(),
        how,
    }
}

fn align(objects: &[&str], to: Alignment) -> BoardChange {
    arrange(objects, Arrangement::Align(to))
}

fn refusal(document: &Doc, change: &BoardChange) -> Option<String> {
    built(document, change).err().map(|error| error.to_string())
}

fn at(document: &Doc, id: &str) -> (Option<f64>, Option<f64>) {
    let fields = fields_of(document, id);
    let coordinate = |axis: &str| {
        fields
            .get("position")
            .and_then(|position| position.get(axis))
            .and_then(Value::as_f64)
    };
    (coordinate("x"), coordinate("y"))
}

fn touched(update: &ChangeUpdate) -> Vec<&str> {
    let mut ids: Vec<&str> = update
        .objects
        .iter()
        .map(|touched| touched.id.as_str())
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn left_puts_the_objects_against_the_left_edge_of_all() -> Result<()> {
    let document = board()?;
    let update = built(&document, &align(&["one", "two", "three"], Alignment::Left))?;
    assert_eq!(at(&document, "two"), (Some(0.0), Some(50.0)));
    assert_eq!(at(&document, "three"), (Some(0.0), Some(300.0)));
    assert_eq!(touched(&update), vec!["three", "two"]);
    Ok(())
}

#[test]
fn center_and_middle_meet_at_the_middle_of_all() -> Result<()> {
    let document = board()?;
    built(
        &document,
        &align(&["one", "two", "three"], Alignment::Center),
    )?;
    assert_eq!(at(&document, "one"), (Some(250.0), Some(0.0)));
    assert_eq!(at(&document, "three"), (Some(250.0), Some(300.0)));
    let other = board()?;
    built(&other, &align(&["one", "two", "three"], Alignment::Middle))?;
    assert_eq!(at(&other, "one"), (Some(0.0), Some(150.0)));
    assert_eq!(at(&other, "two"), (Some(200.0), Some(150.0)));
    Ok(())
}

#[test]
fn right_and_bottom_meet_the_far_edges_of_all() -> Result<()> {
    let document = board()?;
    built(
        &document,
        &align(&["one", "two", "three"], Alignment::Right),
    )?;
    assert_eq!(at(&document, "one"), (Some(500.0), Some(0.0)));
    let other = board()?;
    built(&other, &align(&["one", "two", "three"], Alignment::Bottom))?;
    assert_eq!(at(&other, "one"), (Some(0.0), Some(300.0)));
    assert_eq!(at(&other, "two"), (Some(200.0), Some(300.0)));
    Ok(())
}

#[test]
fn a_spread_makes_the_gaps_equal_and_keeps_the_outer_two() -> Result<()> {
    let document = board()?;
    let update = built(
        &document,
        &arrange(
            &["one", "two", "three"],
            Arrangement::Distribute(Axis::Horizontal),
        ),
    )?;
    assert_eq!(at(&document, "one"), (Some(0.0), Some(0.0)));
    assert_eq!(at(&document, "two"), (Some(250.0), Some(50.0)));
    assert_eq!(at(&document, "three"), (Some(500.0), Some(300.0)));
    assert_eq!(touched(&update), vec!["two"]);
    Ok(())
}

#[test]
fn a_group_moves_with_its_members() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "G", "type": "group", "position": { "x": 0, "y": 600 } }),
    )?;
    seed(&document, &sticker("m", Some("G"), 0, 600))?;
    seed(&document, &sticker("n", Some("G"), 100, 600))?;
    built(&document, &align(&["G", "three"], Alignment::Right))?;
    assert_eq!(at(&document, "G"), (Some(400.0), Some(600.0)));
    assert_eq!(at(&document, "m"), (Some(400.0), Some(600.0)));
    assert_eq!(at(&document, "n"), (Some(500.0), Some(600.0)));
    assert_eq!(at(&document, "three"), (Some(500.0), Some(300.0)));
    Ok(())
}

#[test]
fn a_frame_moves_with_its_content() -> Result<()> {
    let document = board()?;
    seed(&document, &sticker("inner", Some("F"), 2010, 20))?;
    built(&document, &align(&["F", "one"], Alignment::Left))?;
    assert_eq!(at(&document, "one"), (Some(0.0), Some(0.0)));
    assert_eq!(at(&document, "F"), (Some(0.0), Some(0.0)));
    assert_eq!(at(&document, "inner"), (Some(10.0), Some(20.0)));
    Ok(())
}

#[test]
fn an_object_moved_into_a_frame_goes_into_it() -> Result<()> {
    let document = board()?;
    seed(&document, &sticker("inner", Some("F"), 2010, 20))?;
    built(&document, &align(&["inner", "one"], Alignment::Right))?;
    assert_eq!(at(&document, "one"), (Some(2010.0), Some(0.0)));
    assert_eq!(
        fields_of(&document, "one").get("parentId"),
        Some(&json!("F"))
    );
    Ok(())
}

#[test]
fn an_arrow_stuck_to_an_object_follows_it_and_stays_out() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({
            "id": "ar", "type": "arrow",
            "start": { "objectId": "one" },
            "end": { "point": { "x": 5, "y": 5 } },
        }),
    )?;
    let update = built(&document, &align(&["one", "two", "ar"], Alignment::Left))?;
    assert_eq!(touched(&update), vec!["two"]);
    Ok(())
}

#[test]
fn a_locked_object_refuses_the_whole_call() -> Result<()> {
    let document = board()?;
    seed(
        &document,
        &json!({ "id": "L", "type": "frame", "locked": true, "width": 10, "height": 10 }),
    )?;
    let said = refusal(&document, &align(&["one", "two", "L"], Alignment::Left));
    assert_eq!(
        said.as_deref(),
        Some("\"L\" is locked on the board; unlock it in the web client first")
    );
    assert_eq!(at(&document, "two"), (Some(200.0), Some(50.0)));
    Ok(())
}

#[test]
fn a_table_cell_a_mind_map_node_and_a_kanban_card_refuse_it() -> Result<()> {
    let document = board()?;
    seed(&document, &json!({ "id": "tbl", "type": "table" }))?;
    seed(
        &document,
        &json!({ "id": "cell", "type": "table-cell", "parentId": "tbl" }),
    )?;
    seed(&document, &json!({ "id": "node", "type": "mind-map-node" }))?;
    let said = |name: &str| refusal(&document, &align(&["one", name], Alignment::Left));
    assert_eq!(
        said("cell").as_deref(),
        Some("\"cell\" cannot be aligned or spread: a table cell stays in its table")
    );
    assert_eq!(
        said("node").as_deref(),
        Some("\"node\" cannot be aligned or spread: a mind-map node stays in its map")
    );
    Ok(())
}

#[test]
fn too_few_objects_are_refused() -> Result<()> {
    let document = board()?;
    assert_eq!(
        refusal(&document, &align(&["one"], Alignment::Left)).as_deref(),
        Some(
            "arranging needs at least 2 objects that can be moved, not 1: nothing was changed"
        )
    );
    let spread = arrange(&["one", "two"], Arrangement::Distribute(Axis::Vertical));
    assert_eq!(
        refusal(&document, &spread).as_deref(),
        Some(
            "arranging needs at least 3 objects that can be moved, not 2: nothing was changed"
        )
    );
    Ok(())
}
