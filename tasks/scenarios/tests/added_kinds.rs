//! Kinds a listing shows more of than the fields every object has.

use holst_scenarios::{FixtureObject, ObjectCall, ScenarioError, Surface, given, then};
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
