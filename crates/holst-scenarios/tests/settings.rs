//! A board's cover, the same on both surfaces.

use holst_fake::{FixtureObject, Right};
use holst_scenarios::{
    CoverEdit,
    Given,
    ScenarioError,
    Surface,
    board_of,
    cover_of,
    given,
    note_of,
    then,
    uploads_of,
};
use rstest::rstest;

/// The signature of a PNG, which is all a cover is checked by.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pixels-";

fn with_a_plan() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plan",
        "Team",
        vec![FixtureObject::Sticker("Buy milk".into())],
    )
}

#[rstest]
#[tokio::test]
async fn a_board_cover_is_set_from_a_file(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_plan().local_file("cover.png", PNG).start().await?;
    let observation = holst
        .on(surface)
        .edit_board_settings("Plan", Some(CoverEdit::File("cover.png")))
        .await?;
    then(&observation).succeeded();
    uploads_of(&holst).holds_one_named("cover.png");
    cover_of(&holst, "Plan").is_the_upload_of("cover.png");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_cover_reset_uploads_a_fresh_preview(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_plan()
        .cover("Plan", "stored-0-old.png")
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .edit_board_settings("Plan", Some(CoverEdit::Reset))
        .await?;
    then(&observation).succeeded();
    uploads_of(&holst).holds_one_named("preview.png");
    cover_of(&holst, "Plan").is_the_automatic_preview();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_cover_that_is_not_an_image_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_plan()
        .local_file("notes.png", b"plain words")
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .edit_board_settings("Plan", Some(CoverEdit::File("notes.png")))
        .await?;
    then(&observation).signalled_as(surface.usage());
    uploads_of(&holst).is_empty();
    cover_of(&holst, "Plan").is_unset();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_edit_dry_run_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_plan()
        .note("Plan", "Old")
        .local_file("cover.png", PNG)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .edit_board_settings("Plan", Some(CoverEdit::File("cover.png")))
        .await?;
    then(&observation).succeeded();
    uploads_of(&holst).is_empty();
    cover_of(&holst, "Plan").is_unset();
    assert_eq!(note_of(&holst, "Plan").as_deref(), Some("Old"));
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_edit_without_flags_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_plan().start().await?;
    let observation = holst.on(surface).edit_board_settings("Plan", None).await?;
    then(&observation).signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_you_cannot_edit_keeps_its_cover(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_as("Plan", "Team", Right::Viewer)
        .note("Plan", "Old")
        .local_file("cover.png", PNG)
        .start()
        .await?;
    let on = holst.on(surface);
    let by_cover = on
        .edit_board_settings("Plan", Some(CoverEdit::File("cover.png")))
        .await?;
    then(&by_cover).signalled_as(surface.permission_denied());
    cover_of(&holst, "Plan").is_unset();
    assert_eq!(note_of(&holst, "Plan").as_deref(), Some("Old"));
    Ok(())
}
