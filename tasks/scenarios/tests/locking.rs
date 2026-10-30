//! Locking objects and unlocking them.

use holst_scenarios::{
    FixtureObject,
    Given,
    ObjectCall,
    ScenarioError,
    Surface,
    board_of,
    then,
    withhold_locking,
};
use rstest::rstest;
use serde_json::json;

use crate::fixtures::plans_with;

fn two_notes() -> Given {
    plans_with(vec![
        FixtureObject::Sticker("One".into()),
        FixtureObject::Sticker("Two".into()),
    ])
}

/// A locked group of `One` and `Two`, a free `Three`, an unlocked frame `Q4`.
fn locked_notes() -> Given {
    plans_with(vec![
        FixtureObject::Group.set("locked", json!(true)),
        FixtureObject::Sticker("One".into()).inside(1),
        FixtureObject::Sticker("Two".into()).inside(1),
        FixtureObject::Sticker("Three".into()),
        FixtureObject::Frame("Q4".into()),
    ])
}

#[rstest]
#[tokio::test]
async fn locked_objects_sit_in_a_locked_group_and_are_kept(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    let call = ObjectCall::lock().with("object", &sticker);
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects_in_any_order(&[
        ("sticker", "One"),
        ("sticker", "Two"),
        ("group", ""),
    ]);
    then(&read).lists_locked(&[""]);
    then(&read).lists_inside_a_parent(&["One"]);
    let edited = holst
        .on(surface)
        .change_object_text("Plans", &sticker, "Buy oat milk")
        .await?;
    then(&edited)
        .failed_saying("is locked on the board; unlock it in the web client first")
        .signalled_as(surface.permission_denied());
    let removed = holst.on(surface).remove_object("Plans", &sticker).await?;
    then(&removed)
        .failed_saying("is locked on the board; unlock it in the web client first")
        .signalled_as(surface.permission_denied());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn only_locks_a_frame_by_itself(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![
        FixtureObject::Frame("Q4".into()),
        FixtureObject::Sticker("One".into()).inside(1),
    ])
    .start()
    .await?;
    let shape = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("ratio", "1:1")
        .with("background", "fill");
    then(&holst.on(surface).object("Plans", shape).await?).succeeded();
    let call = ObjectCall::lock()
        .with("object", holst.fixture_object("Plans", 1)?)
        .switched("only");
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects(&[("frame", "Q4"), ("sticker", "One")]);
    then(&read).lists_locked(&["Q4"]);
    then(&read).lists_shape_looking("Q4", "ratio 1:1 background fill");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_lock_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let call = ObjectCall::lock().with("object", holst.fixture_object("Plans", 1)?);
    holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    board_of(&holst).got_no_update();
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects(&[("sticker", "One"), ("sticker", "Two")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_that_withholds_locking_is_not_locked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    withhold_locking(&holst, "Plans", "co_owner", "editor");
    let call = ObjectCall::lock().with("object", holst.fixture_object("Plans", 1)?);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("held to co_owner")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_that_holds_locking_to_the_role_of_the_user_is_locked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    withhold_locking(&holst, "Plans", "editor", "editor");
    let call = ObjectCall::lock().with("object", holst.fixture_object("Plans", 1)?);
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_locked(&[""]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unlocked_group_frees_its_members_and_goes(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = locked_notes().start().await?;
    let call = ObjectCall::unlock().with("object", holst.fixture_object("Plans", 1)?);
    holst.on(surface).object("Plans", call).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_objects_in_any_order(&[
        ("sticker", "One"),
        ("sticker", "Two"),
        ("sticker", "Three"),
        ("frame", "Q4"),
    ]);
    then(&read).lists_locked(&[]);
    then(&read).lists_inside_a_parent(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unlocked_frame_is_flagged_free(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![
        FixtureObject::Frame("Q4".into()).set("locked", json!(true)),
    ])
    .start()
    .await?;
    let before = holst.on(surface).objects("Plans").await?;
    then(&before).lists_locked(&["Q4"]);
    let call = ObjectCall::unlock().with("object", holst.fixture_object("Plans", 1)?);
    holst.on(surface).object("Plans", call).await?;
    let after = holst.on(surface).objects("Plans").await?;
    then(&after).lists_locked(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn unlock_all_with_nothing_locked_says_so(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_notes().start().await?;
    let observation = holst
        .on(surface)
        .object("Plans", ObjectCall::unlock().switched("all"))
        .await?;
    then(&observation)
        .failed_saying("nothing is locked on the board")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_object_that_is_not_locked_is_not_unlocked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = locked_notes().start().await?;
    let call = ObjectCall::unlock().with("object", holst.fixture_object("Plans", 4)?);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("is not locked itself")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn all_beside_an_object_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = locked_notes().start().await?;
    let call = ObjectCall::unlock()
        .with("object", holst.fixture_object("Plans", 1)?)
        .switched("all");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("left out with all")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}
