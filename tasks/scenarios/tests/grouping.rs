//! Putting objects into a group and dissolving it.

use holst_scenarios::{
    FixtureObject,
    Given,
    ObjectCall,
    ScenarioError,
    Surface,
    board_of,
    then,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::fixtures::plans_with;

fn three_notes() -> Given {
    plans_with(vec![
        FixtureObject::Sticker("One".into()),
        FixtureObject::Sticker("Two".into()),
        FixtureObject::Sticker("Three".into()),
    ])
}

/// A group of the stickers `One` and `Two`, and a free `Three`.
fn grouped_notes() -> Given {
    plans_with(vec![
        FixtureObject::Group,
        FixtureObject::Sticker("One".into()).inside(1),
        FixtureObject::Sticker("Two".into()).inside(1),
        FixtureObject::Sticker("Three".into()),
    ])
}

#[rstest]
#[tokio::test]
async fn grouped_objects_name_a_new_group_as_their_parent(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = three_notes().start().await?;
    let call = ObjectCall::group()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("object", holst.fixture_object("Plans", 2)?);
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects_in_any_order(&[
        ("sticker", "One"),
        ("sticker", "Two"),
        ("sticker", "Three"),
        ("group", ""),
    ]);
    then(&read).lists_inside_a_parent(&["One", "Two"]);
    let group = holst.first_of_kind("Plans", "group")?;
    let member = holst.first_of_kind("Plans", "sticker")?;
    assert_eq!(member.get("parentId"), group.get("id"));
    assert_eq!(group.get("ignoreZIndex"), Some(&json!(true)));
    let corner = |axis: &str| {
        group
            .get("position")
            .and_then(|position| position.get(axis))
            .and_then(Value::as_f64)
    };
    // the corner of the first sticker, which the fake puts at 100,100
    assert_eq!((corner("x"), corner("y")), (Some(100.0), Some(100.0)));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_group_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = three_notes().start().await?;
    let call = ObjectCall::group()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("object", holst.fixture_object("Plans", 2)?);
    holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    board_of(&holst).got_no_update();
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects(&[
        ("sticker", "One"),
        ("sticker", "Two"),
        ("sticker", "Three"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_frame_is_refused_and_nothing_is_written(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![
        FixtureObject::Frame("Q4".into()),
        FixtureObject::Sticker("One".into()),
    ])
    .start()
    .await?;
    let call = ObjectCall::group()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("object", holst.fixture_object("Plans", 2)?);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("cannot be grouped: a frame holds its content itself")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn one_object_is_not_a_group(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = three_notes().start().await?;
    let call = ObjectCall::group().with("object", holst.fixture_object("Plans", 1)?);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("a group needs at least 2 objects, not 1")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_ungrouped_group_frees_its_members_and_goes(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = grouped_notes().start().await?;
    let call = ObjectCall::ungroup().with("object", holst.fixture_object("Plans", 1)?);
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects_in_any_order(&[
        ("sticker", "One"),
        ("sticker", "Two"),
        ("sticker", "Three"),
    ]);
    then(&read).lists_inside_a_parent(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn only_a_group_can_be_ungrouped(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = grouped_notes().start().await?;
    let call = ObjectCall::ungroup().with("object", holst.fixture_object("Plans", 4)?);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("is not a group")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_group_left_with_one_member_dissolves(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = grouped_notes().start().await?;
    let one = holst.fixture_object("Plans", 2)?;
    holst.on(surface).remove_object("Plans", &one).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects_in_any_order(&[("sticker", "Two"), ("sticker", "Three")]);
    then(&read).lists_inside_a_parent(&[]);
    Ok(())
}
