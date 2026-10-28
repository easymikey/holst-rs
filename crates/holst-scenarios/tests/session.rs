//! The saved login: who is signed in, and what a user is told otherwise.

use holst_fake::LOGIN_CODE;
use holst_scenarios::{ScenarioError, Surface, given, then};
use rstest::rstest;

#[rstest]
#[tokio::test]
async fn a_logged_in_user_sees_whose_login_it_is(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Private")
        .start()
        .await?;
    let observation = holst.on(surface).status().await?;
    then(&observation).shows_login_of("Fake User", &["Team", "Private"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn without_a_login_the_status_says_so(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().workspace("Team").start().await?;
    let observation = holst.on(surface).status().await?;
    then(&observation).shows_no_login();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_rejected_login_is_told_in_words(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().rejected_login().workspace("Team").start().await?;
    let observation = holst.on(surface).status().await?;
    then(&observation).says_the_login_was_rejected();
    Ok(())
}

// Only the CLI has a logout: the MCP server never ends a login, it reads it.
#[rstest]
#[tokio::test]
async fn after_a_logout_there_is_no_login(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    holst.log_out().await?;
    let observation = holst.on(surface).status().await?;
    then(&observation).shows_no_login();
    Ok(())
}

// Only the CLI signs in: the MCP server reads what the terminal saved.
#[rstest]
#[tokio::test]
async fn a_login_made_in_a_terminal_is_seen_by_both_surfaces(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().workspace("Team").start().await?;
    holst.log_in("you@example.com", LOGIN_CODE).await?;
    let observation = holst.on(surface).status().await?;
    then(&observation).shows_login_of("Fake User", &["Team"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn status_counts_the_boards_and_folders_of_each_workspace(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Private")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .board("Retro", "Team", None)
        .board("Notes", "Private", None)
        .start()
        .await?;
    let observation = holst.on(surface).status().await?;
    then(&observation).shows_counts(&[("Team", 2, 1), ("Private", 1, 0)]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn status_with_tokens_lists_the_tokens(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .access_token("ci-runner")
        .access_token("laptop")
        .start()
        .await?;
    let observation = holst.on(surface).status_with_tokens().await?;
    then(&observation).shows("ci-runner");
    then(&observation).shows("laptop");
    Ok(())
}
