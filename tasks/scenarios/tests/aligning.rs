//! Lining objects up against a side and spreading them evenly.
use holst_scenarios::{
    FixtureObject,
    Given,
    Holst,
    ObjectCall,
    ScenarioError,
    Surface,
    board_of,
    then,
};
use rstest::rstest;
use serde_json::json;

use crate::fixtures::plans_with;

/// Three stickers 192 wide at 100,100, 300,150 and 700,400.
fn scattered() -> Given {
    plans_with(vec![
        FixtureObject::Sticker("One".into()).at(100, 100),
        FixtureObject::Sticker("Two".into()).at(300, 150),
        FixtureObject::Sticker("Three".into()).at(700, 400),
    ])
}

fn call_of(holst: &Holst, numbers: &[u32]) -> Result<ObjectCall, ScenarioError> {
    numbers
        .iter()
        .try_fold(ObjectCall::align(), |call, number| {
            Ok(call.with("object", holst.fixture_object("Plans", *number)?))
        })
}

#[rstest]
#[tokio::test]
async fn left_lines_the_objects_up_against_the_leftmost(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = scattered().start().await?;
    let call = call_of(&holst, &[1, 2, 3])?.with("side", "left");
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_object_at("One", (100, 100));
    then(&read).lists_object_at("Two", (100, 150));
    then(&read).lists_object_at("Three", (100, 400));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn middle_centres_the_objects_between_top_and_bottom(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = scattered().start().await?;
    let call = call_of(&holst, &[1, 2, 3])?.with("side", "middle");
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    // the box covers 100..592 down, so each 192 tall sticker starts at 250
    then(&read).lists_object_at("One", (100, 250));
    then(&read).lists_object_at("Three", (700, 250));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_spread_makes_the_gaps_equal(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = scattered().start().await?;
    let call = call_of(&holst, &[1, 2, 3])?.with("distribute", "horizontal");
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    // 100..892 across, 576 of it stickers: two gaps of 108
    then(&read).lists_object_at("One", (100, 100));
    then(&read).lists_object_at("Two", (400, 150));
    then(&read).lists_object_at("Three", (700, 400));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_locked_object_refuses_the_call_and_nothing_is_written(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![
        FixtureObject::Group.set("locked", json!(true)),
        FixtureObject::Sticker("One".into()).inside(1),
        FixtureObject::Sticker("Two".into()).at(400, 150),
    ])
    .start()
    .await?;
    let call = call_of(&holst, &[2, 3])?.with("side", "left");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("is locked on the board")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_align_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = scattered().start().await?;
    let call = call_of(&holst, &[1, 2, 3])?.with("side", "left");
    holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    board_of(&holst).got_no_update();
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_object_at("Two", (300, 150));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_side_or_a_spread_is_needed_but_not_both(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = scattered().start().await?;
    let neither = call_of(&holst, &[1, 2])?;
    let unasked = holst.on(surface).object("Plans", neither).await?;
    then(&unasked)
        .failed_saying("unless distribute is given")
        .signalled_as(surface.usage());
    let both = call_of(&holst, &[1, 2, 3])?
        .with("side", "left")
        .with("distribute", "vertical");
    let observation = holst.on(surface).object("Plans", both).await?;
    then(&observation)
        .failed_saying("left out with side")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}
