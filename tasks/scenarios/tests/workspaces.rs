//! Listing workspaces, the first thing a user does with a login.

use holst_core::fake::WorkspaceRole;
use holst_scenarios::{ScenarioError, Surface, WorkspaceCommand, given, then};
use rstest::rstest;

#[rstest]
#[tokio::test]
async fn without_a_login_the_user_is_told_to_log_in(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().workspace("Team").start().await?;
    let observation = holst.on(surface).list_workspaces().await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_rejected_login_is_reported(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().rejected_login().workspace("Team").start().await?;
    let observation = holst.on(surface).list_workspaces().await?;
    then(&observation)
        .failed_saying("the saved login was revoked")
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn workspaces_show_their_organization(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace_in("Team", "Acme")
        .workspace("Ops")
        .start()
        .await?;
    let observation = holst.on(surface).list_workspace_details().await?;
    then(&observation).shows("Acme");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn joinable_workspaces_are_listed_apart(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .joinable_workspace("Guild")
        .start()
        .await?;
    let joinable = holst.on(surface).list_joinable_workspaces().await?;
    then(&joinable).shows("Guild");
    then(&joinable).does_not_show("Team");
    let own = holst.on(surface).list_workspaces().await?;
    then(&own).lists_workspaces(&["Team"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_workspace_is_shown_with_its_basics(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .start()
        .await?;
    let observation = holst.on(surface).show_workspace(Some("Team")).await?;
    then(&observation).shows("Team");
    then(&observation).shows_members(2);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_workspace_unknown_by_name_is_not_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let observation = holst.on(surface).show_workspace(Some("Nowhere")).await?;
    then(&observation).signalled_as(surface.not_found());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_ambiguous_workspace_name_lists_the_matches(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team A")
        .workspace("Team B")
        .start()
        .await?;
    let observation = holst.on(surface).show_workspace(Some("Team")).await?;
    then(&observation)
        .failed_saying("Team A")
        .failed_saying("Team B")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn no_workspace_given_takes_the_only_one(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let observation = holst.on(surface).show_workspace(None).await?;
    then(&observation).shows("Team");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn no_workspace_given_with_several_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Ops")
        .start()
        .await?;
    let observation = holst.on(surface).show_workspace(None).await?;
    then(&observation)
        .failed_saying("Team")
        .failed_saying("Ops")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn workspace_parts_are_shown_only_when_asked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .start()
        .await?;
    let plain = holst.on(surface).show_workspace(Some("Team")).await?;
    then(&plain).does_not_show("Ann");
    let with_members = holst
        .on(surface)
        .show_workspace_parts(Some("Team"), &["members"], None)
        .await?;
    then(&with_members).shows("Ann");
    Ok(())
}

#[rstest]
#[case::members("members", "Ann")]
#[case::activity("activity", "2026-03-01")]
#[case::plan("plan", "Business")]
#[case::plans("plans", "Enterprise")]
#[case::sso("sso", "enabled")]
#[case::domains("domains", "acme.test")]
#[case::created("created", "count")]
#[tokio::test]
async fn each_workspace_part_is_shown(
    #[case] part: &str,
    #[case] listed: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .domain("Team", "acme.test")
        .sso("Team")
        .plan("Team", "Business")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .show_workspace_parts(Some("Team"), &[part], None)
        .await?;
    then(&observation).shows(listed);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_member_is_shown_alone_by_user(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .member("Bob", "Team", WorkspaceRole::Member)
        .member("Cy", "Team", WorkspaceRole::Member)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .show_workspace_parts(Some("Team"), &["members"], Some("Ann"))
        .await?;
    then(&observation).shows("Ann");
    then(&observation).does_not_show("Bob");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_user_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .show_workspace_parts(Some("Team"), &["members"], Some("Nobody"))
        .await?;
    then(&observation)
        .failed_saying("Ann")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_created_workspace_is_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let created = holst.on(surface).create_workspace("Studio").await?;
    then(&created).succeeded();
    let listed = holst.on(surface).list_workspaces().await?;
    then(&listed).lists_workspaces(&["Team", "Studio"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_creates_no_workspace(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let planned = holst
        .on(surface)
        .dry_running()
        .create_workspace("Studio")
        .await?;
    then(&planned).shows("Studio");
    let listed = holst.on(surface).list_workspaces().await?;
    then(&listed).lists_workspaces(&["Team"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_renamed_workspace_has_the_new_name(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let renamed = holst
        .on(surface)
        .confirming()
        .rename_workspace("Team", "Crew")
        .await?;
    then(&renamed).succeeded();
    let listed = holst.on(surface).list_workspaces().await?;
    then(&listed).lists_workspaces(&["Crew"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_rename_without_confirmation_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let refused = holst.on(surface).rename_workspace("Team", "Crew").await?;
    then(&refused).signalled_as(surface.permission_denied());
    let listed = holst.on(surface).list_workspaces().await?;
    then(&listed).lists_workspaces(&["Team"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_rename_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let planned = holst
        .on(surface)
        .dry_running()
        .rename_workspace("Team", "Crew")
        .await?;
    then(&planned).shows("Crew");
    let listed = holst.on(surface).list_workspaces().await?;
    then(&listed).lists_workspaces(&["Team"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_member_cannot_rename_a_workspace(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .as_workspace_member("Team")
        .start()
        .await?;
    let refused = holst.on(surface).rename_workspace("Team", "Crew").await?;
    then(&refused).signalled_as(surface.permission_denied());
    let listed = holst.on(surface).list_workspaces().await?;
    then(&listed).lists_workspaces(&["Team"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_event_log_lists_newest_first(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .event("Team", "Ann", "board.create", "first-board")
        .event("Team", "Ann", "board.rename", "second-board")
        .event("Team", "Bob", "board.delete", "third-board")
        .start()
        .await?;
    let observation = holst.on(surface).workspace_log(&[]).await?;
    then(&observation).shows_in_order(&["third-board", "second-board", "first-board"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_event_log_is_filtered_by_query(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .event("Team", "Ann", "board.create", "first-board")
        .event("Team", "Bob", "board.create", "second-board")
        .event("Team", "Ann", "board.delete", "third-board")
        .start()
        .await?;
    let by_user = holst.on(surface).workspace_log(&[("user", "Ann")]).await?;
    then(&by_user).shows("first-board");
    then(&by_user).does_not_show("second-board");
    let by_both = holst
        .on(surface)
        .workspace_log(&[("user", "Ann"), ("action", "board.delete")])
        .await?;
    then(&by_both).shows("third-board");
    then(&by_both).does_not_show("first-board");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_long_event_log_gives_a_cursor(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .event("Team", "Ann", "board.create", "first-board")
        .event("Team", "Ann", "board.create", "second-board")
        .event("Team", "Ann", "board.create", "third-board")
        .start()
        .await?;
    let page = [("pageSize", "2")];
    let first = holst.on(surface).workspace_log(&page).await?;
    then(&first).shows("second-board");
    then(&first).does_not_show("first-board");
    then(&first).shows_a_cursor();
    let rest = holst.on(surface).workspace_log_after(&first, &page).await?;
    then(&rest).shows("first-board");
    then(&rest).shows_no_cursor();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn workspace_commands_need_a_login(
    #[values(
        WorkspaceCommand::Show,
        WorkspaceCommand::Create,
        WorkspaceCommand::Rename,
        WorkspaceCommand::Log
    )]
    command: WorkspaceCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().workspace("Team").start().await?;
    let observation = holst.on(surface).run_workspace_command(command).await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}
