//! Options only some kinds take, read back from the board.

use holst_core::fake::WorkspaceRole;
use holst_scenarios::{FixtureObject, Holst, ObjectCall, ScenarioError, Surface, then};
use rstest::rstest;
use serde_json::{Value, json};

use crate::fixtures::plans_with;

// A sticker's text size and author flag are stored in its fields.
#[rstest]
#[tokio::test]
async fn a_sticker_option_is_stored_on_the_sticker(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    let call = ObjectCall::edit()
        .with("object", sticker)
        .with("text_size", "24")
        .switched("show_author");
    holst.on(surface).object("Plans", call).await?;
    let stored = holst.first_of_kind("Plans", "sticker")?;
    assert_eq!(
        (stored.get("fontSize"), stored.get("showAuthor")),
        (Some(&json!(24)), Some(&json!(true)))
    );
    Ok(())
}

// The fake reads no store: `kind_edit/reaction.rs` reads the entries.
#[rstest]
#[tokio::test]
async fn the_reactions_of_a_sticker_are_set_and_taken_away(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    let set = ObjectCall::edit()
        .with("object", sticker.clone())
        .with("reaction", "like")
        .with("reaction", "fire");
    let set = holst.on(surface).object("Plans", set).await?;
    let cleared = ObjectCall::edit()
        .with("object", sticker)
        .with("reaction", "none");
    let cleared = holst.on(surface).object("Plans", cleared).await?;
    then(&set).succeeded();
    then(&cleared).succeeded();
    Ok(())
}

/// A board with a die of 20 faces and a wheel of three choices, and their ids.
async fn dice_and_wheel(
    surface: Surface,
) -> Result<(Holst, Vec<String>), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let die = ObjectCall::add("dice").with("faces", "20");
    let wheel = ObjectCall::add("wheel")
        .with("choice", "Pizza")
        .with("choice", "Sushi")
        .with("choice", "Soup");
    holst.on(surface).object("Plans", die).await?;
    holst.on(surface).object("Plans", wheel).await?;
    let ids = ["dice", "spinner-wheel"]
        .into_iter()
        .map(|kind| {
            holst.first_of_kind("Plans", kind).map(|object| {
                object
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_default()
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((holst, ids))
}

// The draw is a fresh id: read the stored fields, not a number.
#[rstest]
#[tokio::test]
async fn a_die_and_a_wheel_are_rolled_and_stamped_with_the_time(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let (holst, ids) = dice_and_wheel(surface).await?;
    let rolled = ids
        .into_iter()
        .fold(ObjectCall::edit().switched("roll"), |call, id| {
            call.with("object", id)
        });
    let rolled = holst.on(surface).object("Plans", rolled).await?;
    let die = holst.first_of_kind("Plans", "dice")?;
    let wheel = holst.first_of_kind("Plans", "spinner-wheel")?;
    then(&rolled).succeeded();
    assert_eq!(
        (
            die.get("value")
                .and_then(Value::as_i64)
                .is_some_and(|value| (1..=20).contains(&value)),
            die.get("spinTime") != Some(&json!(0)),
            wheel.get("selectedItemId").is_some_and(Value::is_string),
            wheel.get("spinTime") != Some(&json!(0)),
        ),
        (true, true, true, true)
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_faces_of_a_die_change_and_are_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let (holst, ids) = dice_and_wheel(surface).await?;
    let call = ObjectCall::edit()
        .with("object", ids.first().cloned().unwrap_or_default())
        .with("faces", "4");
    let changed = holst.on(surface).object("Plans", call).await?;
    let shown = holst.on(surface).objects("Plans").await?;
    then(&changed).succeeded();
    then(&shown).lists_object_detail("dice", &["faces=4"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_children_of_a_mind_map_node_are_hidden_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::MindMapNode("Plan".into())])
        .start()
        .await?;
    let root = holst.fixture_object("Plans", 1)?;
    let child = ObjectCall::add("mind-map")
        .text("Budget")
        .with("parent", root.clone());
    holst.on(surface).object("Plans", child).await?;
    let call = ObjectCall::edit()
        .with("object", root)
        .switched("hide_children");
    let changed = holst.on(surface).object("Plans", call).await?;
    let shown = holst.on(surface).objects("Plans").await?;
    then(&changed).succeeded();
    then(&shown).lists_object_detail("mind-map", &["children=hidden"]);
    Ok(())
}
#[rstest]
#[tokio::test]
async fn a_flip_card_is_turned_over_and_its_back_rewritten(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let added = ObjectCall::add("flipcard").text("Q").with("back", "A");
    holst.on(surface).object("Plans", added).await?;
    let card = holst
        .first_of_kind("Plans", "flip-card")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let turned = ObjectCall::edit()
        .with("object", card.clone())
        .switched("flip");
    let turned = holst.on(surface).object("Plans", turned).await?;
    let after_turn = holst.on(surface).objects("Plans").await?;
    let rewritten = ObjectCall::edit().with("object", card).with("back", "B");
    let rewritten = holst.on(surface).object("Plans", rewritten).await?;
    let after_back = holst.on(surface).objects("Plans").await?;
    then(&turned).succeeded();
    then(&rewritten).succeeded();
    then(&after_turn).lists_object_detail("flipcard", &["side=back", "back=A"]);
    then(&after_back).lists_object_detail("flipcard", &["side=back", "back=B"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_language_and_the_theme_of_a_code_block_change_and_are_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let added = ObjectCall::add("code").text("fn main() {}");
    holst.on(surface).object("Plans", added).await?;
    let block = holst
        .first_of_kind("Plans", "code")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let call = ObjectCall::edit()
        .with("object", block)
        .with("language", "python")
        .with("theme", "dark");
    let changed = holst.on(surface).object("Plans", call).await?;
    let after = holst.on(surface).objects("Plans").await?;
    then(&changed).succeeded();
    then(&after).lists_object_detail("code", &["language=python", "theme=dark"]);
    Ok(())
}

/// A board with a table of two rows and two columns, and its id.
async fn a_table(surface: Surface) -> Result<(Holst, String), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let added = ObjectCall::add("table")
        .with("rows", "2")
        .with("columns", "2");
    holst.on(surface).object("Plans", added).await?;
    let table = holst
        .first_of_kind("Plans", "table")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    Ok((holst, table))
}

#[rstest]
#[tokio::test]
async fn a_cell_gets_text_and_a_row_is_inserted_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let (holst, table) = a_table(surface).await?;
    let written = ObjectCall::edit()
        .with("object", table.clone())
        .with("cell", "1,2")
        .with("text", "Hi");
    let written = holst.on(surface).object("Plans", written).await?;
    let inserted = ObjectCall::edit()
        .with("object", table.clone())
        .with("insert_row", "1");
    let inserted = holst.on(surface).object("Plans", inserted).await?;
    let shown = holst.on(surface).objects("Plans").await?;
    let cell = holst.first_of_kind("Plans", "table-cell")?;
    then(&written).succeeded();
    then(&inserted).succeeded();
    then(&shown).lists_object_detail("table", &["rows=3", "columns=2"]);
    then(&shown).lists_text("Hi", true);
    assert_eq!(
        (
            cell.get("parentId"),
            cell.get("rowId"),
            cell.get("columnId")
        ),
        (Some(&json!(table)), Some(&json!("1")), Some(&json!("2")))
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn deleting_a_row_with_text_waits_for_the_confirmation(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let (holst, table) = a_table(surface).await?;
    let written = ObjectCall::edit()
        .with("object", table.clone())
        .with("cell", "1,1")
        .with("text", "Hi");
    holst.on(surface).object("Plans", written).await?;
    let delete = || {
        ObjectCall::edit()
            .with("object", table.clone())
            .with("delete_row", "1")
    };
    let held = holst.on(surface).object("Plans", delete()).await?;
    let kept = holst.on(surface).objects("Plans").await?;
    then(&held)
        .failed_saying("removing 1 table cell with text")
        .signalled_as(surface.permission_denied());
    then(&kept).lists_text("Hi", true);
    holst
        .on(surface)
        .confirming()
        .object("Plans", delete())
        .await?;
    let gone = holst.on(surface).objects("Plans").await?;
    then(&gone).lists_object_detail("table", &["rows=1", "columns=2"]);
    then(&gone).lists_text("Hi", false);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn merging_two_cells_writes_the_block_and_a_colour_writes_its_index(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let (holst, table) = a_table(surface).await?;
    let merge = ObjectCall::edit()
        .with("object", table.clone())
        .with("merge", "1,1,1,2");
    let merged = holst.on(surface).object("Plans", merge).await?;
    let paint = ObjectCall::edit()
        .with("object", table)
        .with("color", "blue3");
    let painted = holst.on(surface).object("Plans", paint).await?;
    let fields = holst.first_of_kind("Plans", "table")?;
    then(&merged).succeeded();
    then(&painted).succeeded();
    assert_eq!(
        (
            fields.get("merge-1-1"),
            fields.pointer("/fillColor/color"),
            fields.get("fillColorIndex")
        ),
        (
            Some(&json!({"countRow": 1, "countColumn": 2, "row": "1", "column": "1"})),
            Some(&json!("blue3")),
            Some(&json!(2))
        )
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn clearing_cells_with_text_waits_for_the_confirmation(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let (holst, table) = a_table(surface).await?;
    let written = ObjectCall::edit()
        .with("object", table.clone())
        .with("cell", "1,1")
        .with("text", "Hi");
    holst.on(surface).object("Plans", written).await?;
    let clear = || {
        ObjectCall::edit()
            .with("object", table.clone())
            .with("clear", "1,1,2,2")
    };
    let held = holst.on(surface).object("Plans", clear()).await?;
    let kept = holst.on(surface).objects("Plans").await?;
    then(&held)
        .failed_saying("removing 1 table cell with text")
        .signalled_as(surface.permission_denied());
    then(&kept).lists_text("Hi", true);
    holst
        .on(surface)
        .confirming()
        .object("Plans", clear())
        .await?;
    let gone = holst.on(surface).objects("Plans").await?;
    then(&gone).lists_text("Hi", false);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_task_takes_a_status_and_a_due_day_and_lists_them(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Grid".into())])
        .start()
        .await?;
    let add = ObjectCall::add("task").text("Ship");
    holst.on(surface).object("Plans", add).await?;
    let card = holst.first_of_kind("Plans", "task-card")?;
    let id = card
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let call = ObjectCall::edit()
        .with("object", id)
        .with("status", "done")
        .with("due", "2026-12-01");
    let written = holst.on(surface).object("Plans", call).await?;
    let listed = holst.on(surface).objects("Plans").await?;
    then(&written).succeeded();
    then(&listed).lists_object_detail("task", &["status=done", "due=2026-12-01"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_task_card_takes_tags_lists_them_and_is_found_by_them(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Grid".into())])
        .start()
        .await?;
    holst
        .on(surface)
        .object("Plans", ObjectCall::add("task").text("Ship"))
        .await?;
    let card = holst.first_of_kind("Plans", "task-card")?;
    let id = card
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let tag = |tag: &'static str| {
        ObjectCall::edit()
            .with("object", id.clone())
            .with("tag", tag)
    };
    let written = holst.on(surface).object("Plans", tag("urgent")).await?;
    let tagged = holst.on(surface).objects("Plans").await?;
    let found = holst
        .on(surface)
        .object("Plans", ObjectCall::list().with("tag", "urgent"))
        .await?;
    then(&written).succeeded();
    then(&tagged).lists_object_tagged("Ship", &["urgent"]);
    then(&found).lists_objects(&[("task", "Ship")]);
    holst.on(surface).object("Plans", tag("")).await?;
    let cleared = holst.on(surface).objects("Plans").await?;
    then(&cleared).lists_object_tagged("Ship", &[]);
    Ok(())
}

// The assignees are members of the board's workspace, found by name.
#[rstest]
#[tokio::test]
async fn a_task_is_assigned_to_a_workspace_member_and_cleared(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Grid".into())])
        .member("Ann", "Team", WorkspaceRole::Member)
        .start()
        .await?;
    holst
        .on(surface)
        .object("Plans", ObjectCall::add("task").text("Ship"))
        .await?;
    let card = holst.first_of_kind("Plans", "task-card")?;
    let id = card
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let assign = |who: &'static str| {
        ObjectCall::edit()
            .with("object", id.clone())
            .with("assignee", who)
    };
    let unknown = holst.on(surface).object("Plans", assign("Zed")).await?;
    let dry = holst
        .on(surface)
        .dry_running()
        .object("Plans", assign("ann"))
        .await?;
    let untouched = holst.on(surface).objects("Plans").await?;
    let written = holst.on(surface).object("Plans", assign("ann")).await?;
    let assigned = holst.on(surface).objects("Plans").await?;
    holst.on(surface).object("Plans", assign("none")).await?;
    let cleared = holst.on(surface).objects("Plans").await?;
    then(&unknown)
        .failed_saying("a member of the workspace")
        .signalled_as(surface.usage());
    then(&dry).succeeded();
    then(&untouched).lists_object_detail("task", &["assignees=0"]);
    then(&written).succeeded();
    then(&assigned).lists_object_detail("task", &["assignees=1"]);
    then(&cleared).lists_object_detail("task", &["assignees=0"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_task_card_moves_into_a_kanban_cell_and_out_again(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Grid".into())])
        .start()
        .await?;
    let id_of = |kind: &str| {
        holst.first_of_kind("Plans", kind).map(|object| {
            object
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default()
        })
    };
    holst
        .on(surface)
        .object("Plans", ObjectCall::add("task").text("Ship"))
        .await?;
    let card = id_of("task-card")?;
    holst
        .on(surface)
        .object("Plans", ObjectCall::add("kanban"))
        .await?;
    let kanban = id_of("kanban")?;
    let edit = || ObjectCall::edit().with("object", card.clone());
    let unknown = holst
        .on(surface)
        .object(
            "Plans",
            edit()
                .with("kanban", kanban.clone())
                .with("column", "Later"),
        )
        .await?;
    let into = holst
        .on(surface)
        .object(
            "Plans",
            edit().with("kanban", kanban.clone()).with("column", "Done"),
        )
        .await?;
    let inside = holst.on(surface).objects("Plans").await?;
    let out = holst
        .on(surface)
        .object("Plans", edit().with("kanban", "none"))
        .await?;
    let outside = holst.on(surface).objects("Plans").await?;
    then(&unknown)
        .failed_saying("the title of a column of the kanban")
        .signalled_as(surface.usage());
    then(&into).succeeded();
    then(&inside).lists_object_detail("task", &["column=Done", "swimlane=Untitled"]);
    then(&inside).lists_inside_a_parent(&["", "Ship"]);
    then(&out).succeeded();
    then(&outside).lists_inside_a_parent(&[""]);
    Ok(())
}

// Text on a document, a task and a code block; colour on a document and a task.
#[rstest]
#[tokio::test]
async fn the_text_and_the_colour_of_new_kinds_are_edited_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Grid".into())])
        .start()
        .await?;
    let id_of = |kind: &str| {
        holst.first_of_kind("Plans", kind).map(|object| {
            object
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default()
        })
    };
    let on = async |call: ObjectCall| holst.on(surface).object("Plans", call).await;
    on(ObjectCall::add("document").text("Draft")).await?;
    on(ObjectCall::add("task").text("Ship")).await?;
    on(ObjectCall::add("code").text("let x = 1;")).await?;
    let edit = |kind: &str| id_of(kind).map(|id| ObjectCall::edit().with("object", id));
    let document = edit("card")?.with("text", "Final").with("color", "blue3");
    let task = edit("task-card")?.with("text", "Ship it");
    let code = edit("code")?.with("text", "let y = 2;");
    let outside = edit("card")?.with("color", "blue9");
    let tinted = edit("task-card")?.with("color", "red");
    let token = edit("task-card")?.with("color", "red4");
    let edited = (
        on(document).await?,
        on(task).await?,
        on(code).await?,
        on(tinted).await?,
    );
    let refused = on(outside).await?;
    let unnamed = on(token).await?;
    let shown = holst.on(surface).objects("Plans").await?;
    then(&edited.0).succeeded();
    then(&edited.1).succeeded();
    then(&edited.2).succeeded();
    then(&edited.3).succeeded();
    then(&refused)
        .failed_saying("expected one of white1, gray3")
        .signalled_as(surface.usage());
    then(&unnamed)
        .failed_saying("expected one of gray, green")
        .signalled_as(surface.usage());
    then(&shown).lists_objects(&[
        ("sticker", "Grid"),
        ("document", "Final"),
        ("task", "Ship it"),
        ("code", "let y = 2;"),
    ]);
    then(&shown).lists_object_detail("document", &["color=blue3"]);
    Ok(())
}
