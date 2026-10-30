//! Renaming, recolouring and removing a board's tags.

use holst_scenarios::{
    FixtureObject,
    Given,
    Holst,
    ObjectCall,
    ScenarioError,
    Surface,
    TagEdit,
    then,
};
use rstest::rstest;

use crate::fixtures::plans_with;

fn tagged_note() -> Given {
    plans_with(vec![FixtureObject::Sticker("Buy milk".into())])
}

async fn tagged_urgent(holst: &Holst, surface: Surface) -> Result<(), ScenarioError> {
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("tag", "urgent");
    holst.on(surface).object("Plans", call).await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_renamed_tag_is_found_by_its_new_title(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = tagged_note().start().await?;
    tagged_urgent(&holst, surface).await?;
    let edit = TagEdit {
        name: Some("soon"),
        ..TagEdit::default()
    };
    holst.on(surface).edit_tag("Plans", "urgent", edit).await?;
    let read = holst.on(surface).objects("Plans").await?;
    then(&read).lists_object_tagged("Buy milk", &["soon"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_removal_waits_for_the_confirmation_and_then_clears_the_sticker(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = tagged_note().start().await?;
    tagged_urgent(&holst, surface).await?;
    let remove = TagEdit {
        remove: true,
        ..TagEdit::default()
    };
    let held = holst
        .on(surface)
        .edit_tag("Plans", "urgent", remove)
        .await?;
    then(&held)
        .failed_saying("removing the tag \"urgent\" from 1 object")
        .signalled_as(surface.permission_denied());
    let kept = holst.on(surface).objects("Plans").await?;
    then(&kept).lists_object_tagged("Buy milk", &["urgent"]);
    holst
        .on(surface)
        .confirming()
        .edit_tag("Plans", "urgent", remove)
        .await?;
    let cleared = holst.on(surface).objects("Plans").await?;
    then(&cleared).lists_object_tagged("Buy milk", &[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_removal_beside_a_name_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = tagged_note().start().await?;
    tagged_urgent(&holst, surface).await?;
    let both = TagEdit {
        name: Some("soon"),
        remove: true,
        ..TagEdit::default()
    };
    let refused = holst
        .on(surface)
        .confirming()
        .edit_tag("Plans", "urgent", both)
        .await?;
    then(&refused)
        .failed_saying("remove")
        .signalled_as(surface.usage());
    Ok(())
}
