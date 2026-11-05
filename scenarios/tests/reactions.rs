//! The reactions of an object, listed beside it.

use holst_scenarios::{FixtureObject, ObjectCall, ScenarioError, Surface, then};
use rstest::rstest;

use crate::fixtures::plans_with;

#[rstest]
#[tokio::test]
async fn reactions_on_a_sticker_are_listed_with_their_counts(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
        .start()
        .await?;
    let sticker = holst.fixture_object("Plans", 1)?;
    let react = ObjectCall::edit()
        .with("object", sticker)
        .with("reaction", "like")
        .with("reaction", "fire");
    holst.on(surface).object("Plans", react).await?;
    let listed = holst.on(surface).objects("Plans").await?;
    then(&listed).lists_object_reactions("Buy milk", &["fire:1", "like:1"]);
    Ok(())
}
