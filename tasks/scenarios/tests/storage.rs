//! What holst keeps on the machine, and cleaning it. Only the CLI has it.

use holst_core::fake::FixtureObject;
use holst_scenarios::{Cleaning, Given, ScenarioError, Surface, given, kept_in, then};

const BROWSER_PROFILE: &str = "home/.config/holst/browser/Default/cookies";
const PARTIAL_LOGIN: &str = "holst/.session.1-1.tmp";
const SAVED_LOGIN: &str = "holst/session";

/// A saved login, a profile of the removed browser login of 10 bytes and a
/// temporary file of an interrupted login write of 4.
fn with_leftovers() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plan",
            "Team",
            vec![FixtureObject::Sticker("Buy milk".into())],
        )
        .kept_file(BROWSER_PROFILE, b"0123456789")
        .kept_file(PARTIAL_LOGIN, b"half")
}

#[tokio::test]
async fn storage_lists_what_holst_keeps_with_sizes() -> Result<(), ScenarioError> {
    let holst = with_leftovers().start().await?;
    then(&holst.on(Surface::Cli).objects("Plan").await?)
        .lists_objects(&[("sticker", "Buy milk")]);
    let observation = holst.on(Surface::Cli).list_storage().await?;
    then(&observation).lists_storage("browser-profile", 10);
    then(&observation).lists_storage("partial", 4);
    then(&observation).lists_a_place_of("saved-login");
    then(&observation).lists_a_place_of("cache");
    Ok(())
}

#[tokio::test]
async fn an_empty_home_lists_nothing() -> Result<(), ScenarioError> {
    let holst = given().start().await?;
    let observation = holst.on(Surface::Cli).list_storage().await?;
    then(&observation).lists_no_storage();
    Ok(())
}

#[tokio::test]
async fn a_clean_removes_old_logins_partial_files_and_stale_copies()
-> Result<(), ScenarioError> {
    let holst = with_leftovers().start().await?;
    let observation = holst
        .on(Surface::Cli)
        .clean_storage(Cleaning::Leftovers)
        .await?;
    then(&observation).shows("removed");
    kept_in(&holst).lacks(BROWSER_PROFILE);
    kept_in(&holst).lacks(PARTIAL_LOGIN);
    kept_in(&holst).has(SAVED_LOGIN);
    Ok(())
}

#[tokio::test]
async fn the_cache_is_kept_unless_asked() -> Result<(), ScenarioError> {
    let holst = with_leftovers().start().await?;
    let on = holst.on(Surface::Cli);
    then(&on.objects("Plan").await?).lists_objects(&[("sticker", "Buy milk")]);
    kept_in(&holst).has_files_in("cache");
    then(&on.clean_storage(Cleaning::Leftovers).await?).shows("removed");
    kept_in(&holst).has_files_in("cache");
    then(&on.clean_storage(Cleaning::Cache).await?).shows("removed");
    kept_in(&holst).has_no_files_in("cache");
    kept_in(&holst).has(SAVED_LOGIN);
    Ok(())
}

#[tokio::test]
async fn clean_all_removes_everything_holst_keeps() -> Result<(), ScenarioError> {
    let holst = with_leftovers().start().await?;
    let on = holst.on(Surface::Cli).confirming();
    then(&on.objects("Plan").await?).lists_objects(&[("sticker", "Buy milk")]);
    then(&on.clean_storage(Cleaning::All).await?).shows("removed");
    kept_in(&holst).has_no_files_in("cache");
    kept_in(&holst).lacks(SAVED_LOGIN);
    kept_in(&holst).lacks(BROWSER_PROFILE);
    then(&on.status().await?).shows_no_login();
    Ok(())
}

#[tokio::test]
async fn a_clean_dry_run_removes_nothing() -> Result<(), ScenarioError> {
    let holst = with_leftovers().start().await?;
    let observation = holst
        .on(Surface::Cli)
        .dry_running()
        .clean_storage(Cleaning::All)
        .await?;
    then(&observation).shows("dry_run");
    then(&observation).shows("browser");
    kept_in(&holst).has(BROWSER_PROFILE);
    kept_in(&holst).has(PARTIAL_LOGIN);
    kept_in(&holst).has(SAVED_LOGIN);
    Ok(())
}
