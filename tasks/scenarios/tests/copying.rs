//! Copying objects from one board to another.

use holst_scenarios::{
    FixtureObject,
    Given,
    ObjectCall,
    ScenarioError,
    Surface,
    board_of,
    given,
    then,
    withhold_copying,
};
use rstest::rstest;

fn two_boards() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Sticker("Buy milk".into())],
        )
        .board("Notes", "Team", None)
}

#[rstest]
#[tokio::test]
async fn a_board_that_withholds_copying_is_not_copied_from(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards().start().await?;
    withhold_copying(&holst, "Plans", "editor", "viewer");
    let call = ObjectCall::copy()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("into", "Notes");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("held to editor")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    then(&holst.on(surface).objects("Notes").await?).lists_this_many_objects(0);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_copied_from_with_the_right_held_copies(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards().start().await?;
    withhold_copying(&holst, "Plans", "editor", "editor");
    let call = ObjectCall::copy()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("into", "Notes");
    holst.on(surface).object("Plans", call).await?;
    then(&holst.on(surface).objects("Notes").await?).lists_this_many_objects(1);
    Ok(())
}
