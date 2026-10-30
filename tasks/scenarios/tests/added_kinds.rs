//! Kinds a listing shows more of than the fields every object has.

use holst_scenarios::{
    FixtureObject,
    ObjectCall,
    ScenarioError,
    Surface,
    board_of,
    given,
    then,
};
use rstest::rstest;
use serde_json::{Value, json};

fn with(object: FixtureObject, field: &str, value: Value) -> FixtureObject {
    FixtureObject::Set(Box::new(object), field.into(), value)
}

fn one_table() -> FixtureObject {
    [
        ("type", json!("table")),
        ("row-1", json!({"index": 0})),
        ("row-2", json!({"index": 1})),
        ("column-1", json!({"index": 0})),
        ("column-2", json!({"index": 1})),
        ("column-3", json!({"index": 2})),
    ]
    .into_iter()
    .fold(
        FixtureObject::Sticker("Grid".into()),
        |object, (field, value)| with(object, field, value),
    )
}

#[rstest]
#[tokio::test]
async fn a_table_lists_its_rows_and_columns(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects("Plans", "Team", vec![one_table()])
        .start()
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_detail("table", &["rows=2", "columns=3"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_die_is_added_with_its_faces_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("dice").with("faces", "20");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_detail("dice", &["faces=20", "value=1"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_wheel_is_added_with_its_choices_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("wheel")
        .with("choice", "Pizza")
        .with("choice", "Sushi")
        .with("choice", "Soup");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_detail("wheel", &["choices=3", "selected=none"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_sticker_stack_is_added_with_its_title_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("sticker-stack").text("Ideas");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Grid"), ("sticker-stack", "Ideas")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_flip_card_is_added_with_two_sides_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("flipcard").text("Q").with("back", "A");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Grid"), ("flipcard", "Q")]);
    then(&observation).lists_object_detail("flipcard", &["side=front", "back=A"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_mind_map_root_and_its_child_are_added_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::MindMapNode("Plan".into())],
        )
        .start()
        .await?;
    let root = ObjectCall::add("mind-map").text("Roadmap");
    then(&holst.on(surface).object("Plans", root).await?).succeeded();
    let child = ObjectCall::add("mind-map")
        .text("Budget")
        .with("parent", holst.fixture_object("Plans", 1)?)
        .with("direction", "down");
    then(&holst.on(surface).object("Plans", child).await?).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_detail("mind-map", &["node=pill"]);
    then(&observation).lists_inside_a_parent(&["Budget"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_whole_mind_map_is_added_in_one_write(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = ObjectCall::add("mind-map").items(vec![
        json!({ "text": "Plan" }),
        json!({ "parent": "#1", "text": "Build" }),
        json!({ "parent": "#1", "text": "Test" }),
        json!({ "parent": "#2", "text": "Ship" }),
    ]);
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_inside_a_parent(&["Build", "Test", "Ship"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_document_is_added_with_its_color_token_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("document")
        .text("Spec")
        .with("color", "white1");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Grid"), ("document", "Spec")]);
    then(&observation).lists_object_detail("document", &["color=white1"]);
    Ok(())
}
#[rstest]
#[tokio::test]
async fn a_text_stamp_is_added_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("stamp").text("\u{1F44D}");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Grid"), ("stamp", "\u{1F44D}")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_code_block_is_added_with_its_language_and_listed_with_its_text(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("code")
        .text("fn main() {}")
        .with("language", "rust");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Grid"), ("code", "fn main() {}")]);
    then(&observation).lists_object_detail("code", &["language=rust", "theme=light"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_table_is_added_with_its_counts_and_listed_with_its_size(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("table")
        .with("rows", "3")
        .with("columns", "4");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_detail("table", &["rows=3", "columns=4"]);
    then(&observation).lists_object_sized("", "960x180");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_reaction_stamp_is_stuck_to_a_sticker_and_listed_with_its_reaction(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    let call = ObjectCall::add("reaction-stamp").with("on", sticker.clone());
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_detail("reaction-stamp", &["reaction=like"]);
    let stamp = holst.first_of_kind("Plans", "reaction-stamp")?;
    assert_eq!(
        stamp
            .get("stickyPosition")
            .and_then(|sticky| sticky.get("parentId")),
        Some(&json!(sticker))
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_phosphor_icon_is_added_and_listed_with_its_name_and_weight(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = ObjectCall::add("icon").text("rocket-launch");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[("icon", "rocket-launch")]);
    then(&observation).lists_object_detail("icon", &["weight=duotone"]);
    let icon = holst.first_of_kind("Plans", "phosphor-icon")?;
    assert_eq!(
        (icon.get("iconName"), icon.get("fillColor")),
        (
            Some(&json!("rocket-launch")),
            Some(&json!({"color": "gray12", "opacity": 1}))
        )
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_icon_the_set_lacks_is_refused_with_close_names_and_nothing_is_written(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = ObjectCall::add("icon").text("rokcet");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written)
        .failed_saying("\"rokcet\" is not an icon: expected an icon name")
        .fails_saying("(close: rocket, rocket-launch)");
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_task_is_added_with_its_status_and_due_day_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("task")
        .text("Ship")
        .with("status", "in-progress")
        .with("due", "2026-11-06");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Grid"), ("task", "Ship")]);
    then(&observation).lists_object_detail(
        "task",
        &["status=in-progress", "due=2026-11-06", "assignees=0"],
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_kanban_is_added_with_its_columns_and_a_first_card_and_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let call = ObjectCall::add("kanban")
        .with("column", "Now")
        .with("column", "Next");
    let written = holst.on(surface).object("Plans", call).await?;
    then(&written).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[
        ("sticker", "Grid"),
        ("kanban", ""),
        ("task", ""),
    ]);
    then(&observation)
        .lists_object_detail("kanban", &["columns=Now,Next", "swimlanes=Untitled"]);
    then(&observation).lists_object_in_a_frame("task", "");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_task_is_added_into_a_kanban_cell_and_listed_with_its_column(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Grid".into())],
        )
        .start()
        .await?;
    let kanban = ObjectCall::add("kanban");
    then(&holst.on(surface).object("Plans", kanban).await?).succeeded();
    let id = holst
        .first_of_kind("Plans", "kanban")?
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let card = ObjectCall::add("task")
        .text("Ship")
        .with("kanban", id)
        .with("column", "Done");
    then(&holst.on(surface).object("Plans", card).await?).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[
        ("sticker", "Grid"),
        ("kanban", ""),
        ("task", ""),
        ("task", "Ship"),
    ]);
    then(&observation)
        .lists_object_detail("task", &["column=Done", "swimlane=Untitled"]);
    then(&observation).lists_inside_a_parent(&["", "Ship"]);
    Ok(())
}

fn stroke(points: [&'static str; 2]) -> ObjectCall {
    points
        .into_iter()
        .fold(ObjectCall::add("drawing"), |call, point| {
            call.with("point", point)
        })
}

/// The number the fake stores under `name` on the first drawing of `Plans`.
fn drawn(
    holst: &holst_scenarios::Holst,
    name: &str,
) -> Result<Option<f64>, ScenarioError> {
    Ok(holst
        .first_of_kind("Plans", "drawing")?
        .get(name)
        .and_then(Value::as_f64))
}

#[rstest]
#[tokio::test]
async fn a_pen_stroke_is_drawn_and_read_back(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = stroke(["10,20", "110,70"]);
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        [
            drawn(&holst, "strokeWidth")?,
            drawn(&holst, "color")?,
            drawn(&holst, "alpha")?
        ],
        [Some(3.0), Some(1_973_790.0), Some(1.0)]
    );
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[("drawing", "")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_highlighter_stroke_is_half_transparent(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = stroke(["10,20", "110,70"]).switched("highlighter");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        [
            drawn(&holst, "strokeWidth")?,
            drawn(&holst, "color")?,
            drawn(&holst, "alpha")?
        ],
        [Some(22.0), Some(16_511_040.0), Some(0.5)]
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_stroke_takes_its_palette_color_and_width(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = stroke(["10,20", "110,70"])
        .with("color", "yellow4")
        .with("stroke_width", "9");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        [drawn(&holst, "strokeWidth")?, drawn(&holst, "color")?],
        [Some(9.0), Some(16_577_455.0)]
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_stroke_in_a_frame_counts_from_its_corner(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Frame("Q4".into())
                    .at(100, 100)
                    .sized(400, 400),
            ],
        )
        .start()
        .await?;
    let call = stroke(["10,20", "110,70"]).with("frame", "Q4");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    let drawing = holst.first_of_kind("Plans", "drawing")?;
    let corner = |axis: &str| {
        drawing
            .get("position")
            .and_then(|position| position.get(axis))
            .and_then(Value::as_f64)
    };
    assert_eq!([corner("x"), corner("y")], [Some(110.0), Some(120.0)]);
    Ok(())
}

#[rstest]
#[case::one_point(&["10,20"], "at least 2 points")]
#[case::bad_point(&["10,20", "x"], "\"x\" is not a point: expected x,y")]
#[tokio::test]
async fn a_stroke_with_one_point_or_a_bad_point_is_refused(
    #[case] points: &[&'static str],
    #[case] saying: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = points
        .iter()
        .fold(ObjectCall::add("drawing"), |call, point| {
            call.with("point", *point)
        });
    then(&holst.on(surface).object("Plans", call).await?).fails_saying(saying);
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects_in_any_order(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_stroke_sends_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .start()
        .await?;
    let call = stroke(["10,20", "110,70"]);
    holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    board_of(&holst).got_no_update();
    Ok(())
}
