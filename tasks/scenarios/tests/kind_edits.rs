//! Options only some kinds take, read back from the board.

use holst_scenarios::{FixtureObject, ObjectCall, ScenarioError, Surface, then};
use rstest::rstest;
use serde_json::json;

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
