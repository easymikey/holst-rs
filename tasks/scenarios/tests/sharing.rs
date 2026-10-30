//! Who can see and edit a board, on both surfaces.

use holst_core::fake::{Right, WorkspaceRole};
use holst_scenarios::{
    Given,
    Look,
    RightsChange,
    ScenarioError,
    Surface,
    given,
    id_of,
    not_owned_by,
    owned_by,
    role_on,
    sharing_of,
    then,
};
use rstest::rstest;
use serde_json::json;

const ANN: &str = "00000000-0000-4000-9000-000000000001";

fn with_a_board() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .board("Roadmap", "Team", None)
}

const GRANT_EDITOR: RightsChange<'static> = RightsChange::Grant {
    user: "Ann",
    right: "editor",
};
const GRANT_VIEWER: RightsChange<'static> = RightsChange::Grant {
    user: "Ann",
    right: "viewer",
};
const TRANSFER: RightsChange<'static> = RightsChange::Transfer {
    user: "Ann",
    with_boards: false,
};
const TRANSFER_WITH_BOARDS: RightsChange<'static> = RightsChange::Transfer {
    user: "Ann",
    with_boards: true,
};
const REVOKE: RightsChange<'static> = RightsChange::Revoke { user: "Ann" };
const SHARE_BY_LINK: RightsChange<'static> = RightsChange::Share {
    settings: &[("link", "viewer")],
};

#[rstest]
#[tokio::test]
async fn a_granted_role_is_held_by_the_user(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    then(&observation).succeeded();
    role_on(&holst, "Roadmap", ANN, Some("editor"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_role_is_not_granted_without_confirmation(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    then(&observation).signalled_as(surface.permission_denied());
    role_on(&holst, "Roadmap", ANN, None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn sharing_without_confirmation_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .change_rights("Roadmap", "Team", SHARE_BY_LINK)
        .await?;
    then(&observation).signalled_as(surface.permission_denied());
    sharing_of(&holst, "Roadmap").is_untouched();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_sharing_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .change_rights("Roadmap", "Team", SHARE_BY_LINK)
        .await?;
    then(&observation).shows_change("link", "none", "viewer");
    sharing_of(&holst, "Roadmap").is_untouched();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_both_kinds_of_sharing_plans_both_requests(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Share {
                settings: &[
                    ("members", "editor"),
                    ("link", "viewer"),
                    ("copy", "editor"),
                ],
            },
        )
        .await?;
    then(&observation).shows_change("members", "full", "editor");
    then(&observation).shows_change("link", "none", "viewer");
    then(&observation).shows_change("copy", "viewer", "editor");
    sharing_of(&holst, "Roadmap").is_untouched();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_a_grant_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    then(&observation).shows("channel_user");
    role_on(&holst, "Roadmap", ANN, None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_grant_to_an_unknown_user_is_refused_with_the_members_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Grant {
                user: "Nobody",
                right: "editor",
            },
        )
        .await?;
    then(&observation)
        .failed_saying("Ann")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[case("viewr")]
#[case("none")]
#[tokio::test]
async fn a_grant_with_a_misspelt_right_is_refused_with_the_rights_listed(
    #[case] right: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Grant { user: "Ann", right },
        )
        .await?;
    then(&observation)
        .failed_saying("viewer")
        .signalled_as(surface.usage());
    role_on(&holst, "Roadmap", ANN, None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_grant_changes_a_held_right(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", GRANT_VIEWER)
        .await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    then(&observation).succeeded();
    role_on(&holst, "Roadmap", ANN, Some("editor"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_revoked_right_is_gone(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", REVOKE)
        .await?;
    then(&observation).succeeded();
    role_on(&holst, "Roadmap", ANN, None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_revoke_without_confirmation_keeps_the_right(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    let observation = holst
        .on(surface)
        .change_rights("Roadmap", "Team", REVOKE)
        .await?;
    then(&observation).signalled_as(surface.permission_denied());
    role_on(&holst, "Roadmap", ANN, Some("editor"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_a_revoke_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", GRANT_EDITOR)
        .await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .change_rights("Roadmap", "Team", REVOKE)
        .await?;
    then(&observation).shows("channel_user");
    role_on(&holst, "Roadmap", ANN, Some("editor"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_revoke_of_a_right_not_held_is_told(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", REVOKE)
        .await?;
    then(&observation).told_no_right_was_held();
    role_on(&holst, "Roadmap", ANN, None);
    Ok(())
}

#[rstest]
#[case(
    "lock",
    "full",
    json!({
        "who_can_manage_lock_content": "co_owner",
        "who_can_manage_frame_hide_content": "co_owner"
    })
)]
#[tokio::test]
async fn each_typed_share_flag_writes_its_setting(
    #[case] flag: &str,
    #[case] value: &str,
    #[case] written: serde_json::Value,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Share {
                settings: &[(flag, value)],
            },
        )
        .await?;
    then(&observation).succeeded();
    sharing_of(&holst, "Roadmap").holds_only(&written);
    Ok(())
}

#[rstest]
#[case("link", "commentator", json!({ "public": true, "public_access": "commentator" }))]
#[case("link", "none", json!({ "public": false, "public_access": null }))]
#[case("members", "editor", json!({ "workspace_member_access": "editor" }))]
#[case("template", "on", json!({ "share_as_template": true }))]
#[tokio::test]
async fn shared_settings_are_read_back_in_show(
    #[case] flag: &str,
    #[case] value: &str,
    #[case] written: serde_json::Value,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let on = holst.on(surface);
    holst
        .on(surface)
        .confirming()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Share {
                settings: &[(flag, value)],
            },
        )
        .await?;
    sharing_of(&holst, "Roadmap").holds_only(&written);
    let plain = on.show_boards(&["Roadmap"], &Look::new()).await?;
    then(&plain).shows_no_part();
    let shown = on
        .show_boards(&["Roadmap"], &Look::new().with("sharing"))
        .await?;
    then(&shown).shows_sharing(&written);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn several_share_flags_are_one_change(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Share {
                settings: &[("link", "viewer"), ("copy", "editor")],
            },
        )
        .await?;
    then(&observation).succeeded();
    sharing_of(&holst, "Roadmap").holds_only(&json!({
        "public": true,
        "public_access": "viewer",
        "who_can_copy_content": "editor"
    }));
    Ok(())
}

#[rstest]
#[case("link", "full", "viewer")]
#[case("delete", "viewer", "editor")]
#[tokio::test]
async fn a_right_a_flag_does_not_take_is_refused(
    #[case] flag: &str,
    #[case] value: &str,
    #[case] listed: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(
            "Roadmap",
            "Team",
            RightsChange::Share {
                settings: &[(flag, value)],
            },
        )
        .await?;
    then(&observation)
        .failed_saying(listed)
        .signalled_as(surface.usage());
    sharing_of(&holst, "Roadmap").is_untouched();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_share_without_flags_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", RightsChange::Share { settings: &[] })
        .await?;
    then(&observation)
        .failed_saying("a setting to change")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[case(GRANT_EDITOR)]
#[case(SHARE_BY_LINK)]
#[tokio::test]
async fn a_board_you_do_not_own_cannot_be_shared_or_granted(
    #[case] change: RightsChange<'_>,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .board_as("Roadmap", "Team", Right::Editor)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", change)
        .await?;
    then(&observation).signalled_as(surface.permission_denied());
    role_on(&holst, "Roadmap", ANN, None);
    sharing_of(&holst, "Roadmap").is_untouched();
    Ok(())
}

#[rstest]
#[case(GRANT_EDITOR)]
#[case(SHARE_BY_LINK)]
#[tokio::test]
async fn an_unknown_board_is_not_found_for_a_sharing_change(
    #[case] change: RightsChange<'_>,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Nowhere", "Team", change)
        .await?;
    then(&observation).signalled_as(surface.not_found());
    Ok(())
}

#[rstest]
#[case(GRANT_EDITOR)]
#[case(REVOKE)]
#[case(SHARE_BY_LINK)]
#[tokio::test]
async fn rights_commands_need_a_login(
    #[case] change: RightsChange<'_>,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", change)
        .await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_transfer_makes_the_member_the_owner(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights("Roadmap", "Team", TRANSFER)
        .await?;
    then(&observation).succeeded();
    owned_by(&holst, "Roadmap", ANN);
    Ok(())
}

fn with_a_folder() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .member("Ann", "Team", WorkspaceRole::Member)
        .folder("Shelf", "Team")
        .board("Inside", "Team", Some("Shelf"))
}

#[rstest]
#[tokio::test]
async fn a_confirmed_transfer_with_boards_takes_the_boards_inside(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_folder().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(&id_of(&holst, "Shelf"), "Team", TRANSFER_WITH_BOARDS)
        .await?;
    then(&observation).succeeded();
    owned_by(&holst, "Shelf", ANN);
    owned_by(&holst, "Inside", ANN);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_transfer_without_boards_leaves_the_boards_inside(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_folder().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .change_rights(&id_of(&holst, "Shelf"), "Team", TRANSFER)
        .await?;
    then(&observation).succeeded();
    owned_by(&holst, "Shelf", ANN);
    not_owned_by(&holst, "Inside", ANN);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_transfer_without_confirmation_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .change_rights("Roadmap", "Team", TRANSFER)
        .await?;
    then(&observation).signalled_as(surface.permission_denied());
    not_owned_by(&holst, "Roadmap", ANN);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_a_transfer_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .change_rights("Roadmap", "Team", TRANSFER)
        .await?;
    then(&observation).shows("transfer_ownership");
    not_owned_by(&holst, "Roadmap", ANN);
    Ok(())
}
