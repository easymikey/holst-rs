//! Changing folders and boards, the same on both surfaces.

use holst_fake::{Right, WorkspaceRole};
use holst_scenarios::{
    CopyAs,
    Given,
    NewParent,
    RightsChange,
    ScenarioError,
    Surface,
    TreeCommand,
    favorites_of,
    given,
    id_of,
    role_on,
    then,
    trash_of,
    tree_of,
};
use rstest::rstest;
use serde_json::{Value, json};

const UNKNOWN_FOLDER_ID: &str = "00000000-0000-4000-8000-0000000000ff";
const ANN: &str = "00000000-0000-4000-9000-000000000001";

/// A folder `Work` holding the board `Plan`.
fn work_with_plan() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .folder("Work", "Team")
        .board("Plan", "Team", Some("Work"))
}

#[rstest]
#[tokio::test]
async fn a_created_folder_appears_in_the_tree(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    holst.on(surface).create_folder("Plans", "Team").await?;
    tree_of(&holst, "Team").has_folder("Plans", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_is_created_in_a_folder(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .start()
        .await?;
    holst
        .on(surface)
        .create_board("Roadmap", "Team", Some("Plans"))
        .await?;
    tree_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_moves_to_the_root(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    holst
        .on(surface)
        .move_node("Roadmap", "Team", NewParent::Root)
        .await?;
    tree_of(&holst, "Team").has_board("Roadmap", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_moves_into_a_folder(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    holst
        .on(surface)
        .move_node("Roadmap", "Team", NewParent::Folder("Plans"))
        .await?;
    tree_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    Ok(())
}

// The MCP server shows an id as an alias; which one is not tested.
#[rstest]
#[case::cli_name(Surface::Cli, "Nowhere", "Nowhere")]
#[case::mcp_name(Surface::Mcp, "Nowhere", "Nowhere")]
#[case::cli_id(Surface::Cli, UNKNOWN_FOLDER_ID, UNKNOWN_FOLDER_ID)]
#[case::mcp_id(Surface::Mcp, UNKNOWN_FOLDER_ID, "@")]
#[tokio::test]
async fn a_move_to_a_folder_that_does_not_exist_is_refused(
    #[case] surface: Surface,
    #[case] folder: &str,
    #[case] shown: &str,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .move_node("Roadmap", "Team", NewParent::Folder(folder))
        .await?;
    then(&observation)
        .failed_saying(&format!("folder \"{shown}"))
        .failed_saying("not found")
        .signalled_as(surface.not_found());
    tree_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_folder_is_not_moved_into_itself(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .move_node("Plans", "Team", NewParent::Folder("Plans"))
        .await?;
    then(&observation)
        .failed_with_message(
            "cannot move \"Plans\" into itself or into a folder inside it",
        )
        .signalled_as(surface.usage());
    tree_of(&holst, "Team").has_folder("Plans", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_creates_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .create_folder("Plans", "Team")
        .await?;
    then(&observation).showed_dry_run("/gapi/channel/create", "name", &json!("Plans"));
    tree_of(&holst, "Team").has_nothing_named("Plans");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_moves_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .move_node("Roadmap", "Team", NewParent::Root)
        .await?;
    then(&observation).showed_dry_run("/move", "newParentId", &Value::Null);
    tree_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_created_in_a_folder_is_open_to_the_folders_members(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .folder("Plans", "Team")
        .start()
        .await?;
    holst
        .on(Surface::Mcp)
        .confirming()
        .change_rights(
            &id_of(&holst, "Plans"),
            "Team",
            RightsChange::Grant {
                user: "Ann",
                right: "editor",
            },
        )
        .await?;
    holst
        .on(surface)
        .create_board("Roadmap", "Team", Some("Plans"))
        .await?;
    role_on(&holst, "Roadmap", ANN, Some("editor"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_duplicated_board_appears_next_to_the_original(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    holst.on(surface).duplicate_board("Roadmap", "Team").await?;
    tree_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    tree_of(&holst, "Team").has_board("Roadmap-copy", Some("Plans"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_deleted_board_goes_to_the_trash(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    holst.on(surface).delete_node("Roadmap", "Team").await?;
    tree_of(&holst, "Team").has_nothing_named("Roadmap");
    trash_of(&holst, "Team").has_board("Roadmap", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_deletes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .delete_node("Roadmap", "Team")
        .await?;
    then(&observation).showed_dry_run("/delete", "permanently", &json!(false));
    tree_of(&holst, "Team").has_board("Roadmap", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_folder_with_boards_is_not_deleted_unconfirmed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    let observation = holst.on(surface).delete_node("Plans", "Team").await?;
    then(&observation)
        .failed_saying(
            "moving a folder with 1 board to the trash takes everything inside along",
        )
        .signalled_as(surface.permission_denied());
    tree_of(&holst, "Team").has_folder("Plans", None);
    tree_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_delete_takes_a_folder_and_its_boards_to_the_trash(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    holst
        .on(surface)
        .confirming()
        .delete_node("Plans", "Team")
        .await?;
    tree_of(&holst, "Team").has_nothing_named("Plans");
    trash_of(&holst, "Team").has_folder("Plans", None);
    trash_of(&holst, "Team").has_board("Roadmap", Some("Plans"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_folder_with_boards_is_not_moved_unconfirmed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .folder("Archive", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .move_node("Plans", "Team", NewParent::Folder("Archive"))
        .await?;
    then(&observation)
        .failed_saying("moving a folder with 1 board takes everything inside along")
        .signalled_as(surface.permission_denied());
    tree_of(&holst, "Team").has_folder("Plans", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_trashed_board_is_restored(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    holst.on(surface).delete_node("Roadmap", "Team").await?;
    trash_of(&holst, "Team").has_board("Roadmap", None);
    holst.on(surface).restore_node("Roadmap", "Team").await?;
    tree_of(&holst, "Team").has_board("Roadmap", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_duplicates_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .duplicate_board("Roadmap", "Team")
        .await?;
    then(&observation).showed_dry_run(
        "/gapi/channel/duplicate",
        "name",
        &json!("Roadmap-copy"),
    );
    tree_of(&holst, "Team").has_nothing_named("Roadmap-copy");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn what_is_not_in_the_trash_is_not_restored(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    let observation = holst.on(surface).restore_node("Budget", "Team").await?;
    then(&observation)
        .failed_with_message("board or folder \"Budget\" not found in the trash")
        .signalled_as(surface.not_found());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_is_created_in_the_named_workspace(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Ops")
        .start()
        .await?;
    holst.on(surface).create_board("Plan", "Ops", None).await?;
    tree_of(&holst, "Ops").has_board("Plan", None);
    tree_of(&holst, "Team").has_nothing_named("Plan");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_create_without_a_workspace_with_several_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Ops")
        .start()
        .await?;
    let observation = holst.on(surface).create_board_anywhere("Plan").await?;
    then(&observation)
        .failed_saying("Team")
        .failed_saying("Ops")
        .signalled_as(surface.usage());
    tree_of(&holst, "Team").has_nothing_named("Plan");
    tree_of(&holst, "Ops").has_nothing_named("Plan");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_is_not_created_in_a_missing_folder(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let observation = holst
        .on(surface)
        .create_board("Plan", "Team", Some("Nowhere"))
        .await?;
    then(&observation)
        .failed_saying("folder \"Nowhere\"")
        .failed_saying("not found")
        .signalled_as(surface.not_found());
    tree_of(&holst, "Team").has_nothing_named("Plan");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_move_takes_a_folder_with_its_boards(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = work_with_plan().folder("Archive", "Team").start().await?;
    holst
        .on(surface)
        .confirming()
        .move_node("Work", "Team", NewParent::Folder("Archive"))
        .await?;
    tree_of(&holst, "Team").has_folder("Work", Some("Archive"));
    tree_of(&holst, "Team").has_board("Plan", Some("Work"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_copy_takes_a_name_and_a_folder(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = work_with_plan()
        .board("Notes", "Team", None)
        .start()
        .await?;
    holst
        .on(surface)
        .copy_board(
            "Notes",
            "Team",
            CopyAs {
                name: Some("Notes 2"),
                folder: Some("Work"),
                rights: false,
            },
        )
        .await?;
    tree_of(&holst, "Team").has_board("Notes 2", Some("Work"));
    tree_of(&holst, "Team").has_board("Notes", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_copy_keeps_rights_only_when_asked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .board("Plan", "Team", None)
        .start()
        .await?;
    holst
        .on(Surface::Mcp)
        .confirming()
        .change_rights(
            "Plan",
            "Team",
            RightsChange::Grant {
                user: "Ann",
                right: "editor",
            },
        )
        .await?;
    let copy_as = |name| CopyAs {
        name: Some(name),
        folder: None,
        rights: name == "Shared",
    };
    holst
        .on(surface)
        .copy_board("Plan", "Team", copy_as("Plain"))
        .await?;
    holst
        .on(surface)
        .copy_board("Plan", "Team", copy_as("Shared"))
        .await?;
    role_on(&holst, "Plain", ANN, None);
    role_on(&holst, "Shared", ANN, Some("editor"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_permanent_delete_is_refused_unconfirmed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plan", "Team", None)
        .start()
        .await?;
    let observation = holst.on(surface).delete_permanently("Plan", "Team").await?;
    then(&observation)
        .failed_saying("for good")
        .signalled_as(surface.permission_denied());
    tree_of(&holst, "Team").has_board("Plan", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_permanent_delete_removes_the_board_for_good(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plan", "Team", None)
        .start()
        .await?;
    holst
        .on(surface)
        .confirming()
        .delete_permanently("Plan", "Team")
        .await?;
    tree_of(&holst, "Team").has_nothing_named("Plan");
    trash_of(&holst, "Team").has_nothing_named("Plan");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_folder_deleted_for_good_leaves_its_board_at_the_root(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Plans", "Team")
        .board("Roadmap", "Team", Some("Plans"))
        .start()
        .await?;
    holst
        .on(surface)
        .confirming()
        .delete_permanently(&id_of(&holst, "Plans"), "Team")
        .await?;
    tree_of(&holst, "Team").has_nothing_named("Plans");
    tree_of(&holst, "Team").has_board("Roadmap", None);
    trash_of(&holst, "Team").has_nothing_named("Plans");
    trash_of(&holst, "Team").has_nothing_named("Roadmap");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_restored_folder_brings_its_boards_back(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = work_with_plan().start().await?;
    holst
        .on(surface)
        .confirming()
        .delete_node("Work", "Team")
        .await?;
    trash_of(&holst, "Team").has_folder("Work", None);
    holst.on(surface).restore_node("Work", "Team").await?;
    tree_of(&holst, "Team").has_folder("Work", None);
    tree_of(&holst, "Team").has_board("Plan", Some("Work"));
    Ok(())
}

/// A small write is one that needs no confirmation and is easy to undo; a dry
/// run of it changes nothing.
fn small_writes() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board("Plan", "Team", None)
        .invited_board("Shared", "Team", "Ann")
}

#[rstest]
#[tokio::test]
async fn a_dry_run_changes_nothing_for_a_restore(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = small_writes().start().await?;
    holst.on(surface).delete_node("Plan", "Team").await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .restore_node("Plan", "Team")
        .await?;
    then(&observation).showed_a_preview("/restore/");
    trash_of(&holst, "Team").has_board("Plan", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_changes_nothing_for_a_rename(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = small_writes().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .rename_node("Plan", "Team", "Plan 2")
        .await?;
    then(&observation).showed_a_preview("/channel/");
    tree_of(&holst, "Team").has_board("Plan", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_changes_nothing_for_a_favorite(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = small_writes().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .favorite_board("Plan", "Team")
        .await?;
    then(&observation).showed_a_preview("favorite");
    favorites_of(&holst, "Team").are(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_changes_nothing_for_an_unfavorite(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = small_writes().start().await?;
    holst.on(surface).favorite_board("Plan", "Team").await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .unfavorite_board("Plan", "Team")
        .await?;
    then(&observation).showed_a_preview("favorite");
    favorites_of(&holst, "Team").are(&["Plan"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_changes_nothing_for_a_touch(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = small_writes().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .touch_board("Plan", "Team")
        .await?;
    then(&observation).showed_a_preview("recent");
    assert_eq!(holst.recent_boards(), Vec::<String>::new());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_changes_nothing_for_leaving(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = small_writes().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .leave_board("Shared", "Team")
        .await?;
    then(&observation).showed_a_preview("/leave");
    tree_of(&holst, "Team").has_board("Shared", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_you_may_not_change_is_refused(
    #[values(TreeCommand::Rename, TreeCommand::Remove, TreeCommand::Move)]
    command: TreeCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Work", "Team")
        .board_as("Plan", "Team", Right::Viewer)
        .start()
        .await?;
    let on = holst.on(surface);
    let observation = on.run_tree_command(command, "Plan").await?;
    then(&observation).signalled_as(surface.permission_denied());
    tree_of(&holst, "Team").has_board("Plan", None);
    trash_of(&holst, "Team").has_nothing_named("Plan");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_board_is_not_found(
    #[values(
        TreeCommand::Rename,
        TreeCommand::Remove,
        TreeCommand::Copy,
        TreeCommand::Favorite
    )]
    command: TreeCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plan", "Team", None)
        .start()
        .await?;
    let on = holst.on(surface);
    let observation = on.run_tree_command(command, "Budget").await?;
    then(&observation)
        .failed_saying("\"Budget\"")
        .failed_saying("not found")
        .signalled_as(surface.not_found());
    tree_of(&holst, "Team").has_board("Plan", None);
    favorites_of(&holst, "Team").are(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn board_tree_commands_need_a_login(
    #[values(
        TreeCommand::List,
        TreeCommand::Create,
        TreeCommand::Remove,
        TreeCommand::Favorite
    )]
    command: TreeCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .board("Plan", "Team", None)
        .start()
        .await?;
    let on = holst.on(surface);
    let observation = on.run_tree_command(command, "Plan").await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    tree_of(&holst, "Team").has_board("Plan", None);
    Ok(())
}
