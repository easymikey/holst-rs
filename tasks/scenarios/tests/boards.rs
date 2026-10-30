//! Reading a board, searching the text on boards and the tree of boards.

use holst_core::fake::WorkspaceRole;
use holst_scenarios::{
    BoardReference,
    Export,
    FixtureObject,
    Given,
    Look,
    Ls,
    ObjectCall,
    RightsChange,
    ScenarioError,
    Search,
    Surface,
    board_of,
    favorites_of,
    given,
    then,
    tree_of,
};
use rstest::rstest;

use crate::fixtures::{plan_with_a_frame, two_boards_with_milk};

#[rstest]
#[tokio::test]
async fn a_board_shows_its_objects(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Sticker("Buy milk".into()),
                FixtureObject::FreeText("Roadmap".into()),
                FixtureObject::Shape("Decision".into()),
                FixtureObject::Frame("Q4".into()),
            ],
        )
        .start()
        .await?;
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[
        ("sticker", "Buy milk"),
        ("text", "Roadmap"),
        ("shape", "Decision"),
        ("frame", "Q4"),
    ]);
    Ok(())
}

#[rstest]
#[case::sticker("sticker", "sticker", "Buy milk")]
#[case::text("text", "text", "Roadmap")]
#[case::shape_any_case("Shape", "shape", "Decision")]
#[case::without_variant("mind-map-node", "mind-map", "Idea")]
#[case::short_word("mind-map", "mind-map", "Idea")]
#[tokio::test]
async fn objects_are_filtered_by_kind(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] kind: &str,
    #[case] shown: &str,
    #[case] text: &str,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Sticker("Buy milk".into()),
                FixtureObject::FreeText("Roadmap".into()),
                FixtureObject::Shape("Decision".into()),
                FixtureObject::MindMapNode("Idea".into()),
            ],
        )
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .object("Plans", ObjectCall::list().with("kind", kind))
        .await?;
    then(&observation).lists_objects(&[(shown, text)]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_misspelt_kind_is_refused_with_the_kinds_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .object("Plans", ObjectCall::list().with("kind", "stikcer"))
        .await?;
    then(&observation)
        .failed_saying(
            "is not an object kind: expected sticker, sticker-stack, text, shape",
        )
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn text_is_found_on_every_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .search(Search::new("milk").mode("board_text").limit(0))
        .await?;
    then(&observation).finds_text(&[
        ("Groceries", "sticker", "Milk"),
        ("Groceries", "shape", "Milk run"),
        ("Plans", "sticker", "Buy milk"),
    ]);
    Ok(())
}

#[rstest]
#[case::two(2, 2)]
#[tokio::test]
async fn a_limit_caps_the_hits(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] limit: u32,
    #[case] shown: usize,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .search(Search::new("milk").mode("board_text").limit(limit))
        .await?;
    then(&observation).finds_matches(shown);
    Ok(())
}

/// A folder `Work` holding the board `Plan`, and the board `Notes` at the
/// root.
fn work_and_notes() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .folder("Work", "Team")
        .board("Plan", "Team", Some("Work"))
        .board("Notes", "Team", None)
}

#[rstest]
#[tokio::test]
async fn boards_are_listed_as_a_tree(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = work_and_notes().start().await?;
    let observation = holst.on(surface).list_tree(Ls::new()).await?;
    then(&observation).lists_tree(&["Work/", "  Plan", "Notes"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_listing_is_limited_to_one_workspace(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Ops")
        .board("Alpha", "Team", None)
        .board("Beta", "Ops", None)
        .start()
        .await?;
    let team = holst
        .on(surface)
        .list_tree(Ls::new().workspace("Team"))
        .await?;
    then(&team).lists_among(&["Alpha"]);
    then(&team).lists_none_of(&["Beta"]);
    let all = holst.on(surface).list_tree(Ls::new()).await?;
    then(&all).lists_among(&["Alpha", "Beta"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_folder_lists_only_its_content(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = work_and_notes().start().await?;
    let observation = holst
        .on(surface)
        .list_tree(Ls::new().folder("Work"))
        .await?;
    then(&observation).lists_among(&["Plan"]);
    then(&observation).lists_none_of(&["Notes"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_trash_view_lists_trashed_boards(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = work_and_notes().start().await?;
    holst.on(surface).delete_node("Notes", "Team").await?;
    let trash = holst.on(surface).list_tree(Ls::new().view("trash")).await?;
    then(&trash).lists_tree(&["Notes [in trash]"]);
    let tree = holst.on(surface).list_tree(Ls::new()).await?;
    then(&tree).lists_none_of(&["Notes", "Notes [in trash]"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_listing_is_paged_by_limit_and_cursor(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(5, "Team")
        .start()
        .await?;
    let first = holst.on(surface).list_tree(Ls::new().limit(2)).await?;
    then(&first).lists_tree(&["Board 1", "Board 2"]);
    let cursor = then(&first).next_cursor();
    let second = holst
        .on(surface)
        .list_tree(Ls::new().limit(2).cursor(&cursor))
        .await?;
    then(&second).lists_tree(&["Board 3", "Board 4"]);
    let all = holst.on(surface).list_tree(Ls::new().limit(0)).await?;
    then(&all).lists_tree(&["Board 1", "Board 2", "Board 3", "Board 4", "Board 5"]);
    then(&all).ends_the_listing();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn favorites_are_listed_in_their_order(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(3, "Team")
        .start()
        .await?;
    holst.on(surface).favorite_board("Board 3", "Team").await?;
    holst.on(surface).favorite_board("Board 1", "Team").await?;
    let observation = holst
        .on(surface)
        .list_tree(Ls::new().view("favorites"))
        .await?;
    then(&observation).lists_tree(&["Board 3", "Board 1"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn recent_boards_are_per_account_across_workspaces(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Ops")
        .board("Alpha", "Team", None)
        .board("Beta", "Ops", None)
        .start()
        .await?;
    holst.on(surface).touch_board("Alpha", "Team").await?;
    holst.on(surface).touch_board("Beta", "Ops").await?;
    let observation = holst
        .on(surface)
        .list_tree(Ls::new().view("recent").workspace("Team"))
        .await?;
    then(&observation).lists_among(&["Alpha", "Beta"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_sidebar_view_lists_the_sidebar_order(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(3, "Team")
        .sidebar_order(&["Board 3", "Board 1"])
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .list_tree(Ls::new().view("sidebar"))
        .await?;
    then(&observation).lists_tree(&["Board 3", "Board 1"]);
    Ok(())
}

fn two_plans() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .folder("Work", "Team")
        .board("Plan", "Team", Some("Work"))
        .folder("Archive", "Team")
        .board("Plan", "Team", Some("Archive"))
}

#[rstest]
#[tokio::test]
async fn an_ambiguous_board_name_lists_the_matches(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_plans().start().await?;
    let observation = holst
        .on(surface)
        .rename_node("Plan", "Team", "Plan 2")
        .await?;
    then(&observation)
        .failed_saying("Work")
        .failed_saying("Archive")
        .signalled_as(surface.usage());
    tree_of(&holst, "Team").has_board("Plan", Some("Work"));
    tree_of(&holst, "Team").has_board("Plan", Some("Archive"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_workspace_flag_picks_between_equal_names(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Ops")
        .board("Plan", "Team", None)
        .board("Plan", "Ops", None)
        .start()
        .await?;
    holst
        .on(surface)
        .rename_node("Plan", "Ops", "Plan 2")
        .await?;
    tree_of(&holst, "Ops").has_board("Plan 2", None);
    tree_of(&holst, "Team").has_board("Plan", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_is_found_by_part_of_its_name_by_id_and_by_link(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[values(BoardReference::Part, BoardReference::Id, BoardReference::Link)]
    reference: BoardReference,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .board("Notes", "Team", None)
        .start()
        .await?;
    let reference = holst.reference_to(reference, "Roadmap")?;
    holst
        .on(surface)
        .rename_node(&reference, "Team", "Plan")
        .await?;
    tree_of(&holst, "Team").has_board("Plan", None);
    tree_of(&holst, "Team").has_nothing_named("Roadmap");
    tree_of(&holst, "Team").has_board("Notes", None);
    Ok(())
}

// The CLI forgets a listing when it ends; only MCP hands out handles.
#[tokio::test]
async fn a_board_is_found_by_its_handle_from_the_listing() -> Result<(), ScenarioError>
{
    let holst = work_and_notes().start().await?;
    holst
        .on(Surface::Mcp)
        .rename_listed_board("Notes", "Team", "Ideas")
        .await?;
    tree_of(&holst, "Team").has_board("Ideas", None);
    tree_of(&holst, "Team").has_board("Plan", Some("Work"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_favorite_is_placed_at_the_asked_index(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(3, "Team")
        .start()
        .await?;
    holst.on(surface).favorite_board("Board 1", "Team").await?;
    holst.on(surface).favorite_board("Board 2", "Team").await?;
    holst
        .on(surface)
        .favorite_board_at("Board 3", "Team", 0)
        .await?;
    favorites_of(&holst, "Team").are(&["Board 3", "Board 1", "Board 2"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unfavorited_board_leaves_the_favorites(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(2, "Team")
        .start()
        .await?;
    holst.on(surface).favorite_board("Board 1", "Team").await?;
    holst.on(surface).favorite_board("Board 2", "Team").await?;
    holst
        .on(surface)
        .unfavorite_board("Board 1", "Team")
        .await?;
    favorites_of(&holst, "Team").are(&["Board 2"]);
    tree_of(&holst, "Team").has_board("Board 1", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_touched_board_is_recent(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(2, "Team")
        .start()
        .await?;
    holst.on(surface).touch_board("Board 1", "Team").await?;
    holst.on(surface).touch_board("Board 2", "Team").await?;
    let observation = holst
        .on(surface)
        .list_tree(Ls::new().view("recent"))
        .await?;
    then(&observation).lists_tree(&["Board 2", "Board 1"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_left_board_is_gone_from_the_list(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .invited_board("Shared", "Team", "Ann")
        .board("Plan", "Team", None)
        .start()
        .await?;
    holst.on(surface).leave_board("Shared", "Team").await?;
    let observation = holst.on(surface).list_tree(Ls::new()).await?;
    then(&observation).lists_tree(&["Plan"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_own_board_cannot_be_left(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Plan", "Team", None)
        .start()
        .await?;
    let observation = holst.on(surface).leave_board("Plan", "Team").await?;
    then(&observation)
        .failed_saying("rm")
        .signalled_as(surface.usage());
    tree_of(&holst, "Team").has_board("Plan", None);
    Ok(())
}

fn plan_of_four() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plan",
        "Team",
        vec![
            FixtureObject::Sticker("Buy milk".into()),
            FixtureObject::Sticker("Buy eggs".into()),
            FixtureObject::FreeText("Roadmap".into()),
            FixtureObject::Shape("Decision".into()),
        ],
    )
}

#[rstest]
#[tokio::test]
async fn a_board_is_shown_with_its_objects_and_details(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_of_four().start().await?;
    let observation = holst
        .on(surface)
        .show_boards(&["Plan"], &Look::new())
        .await?;
    then(&observation).shows_boards(&["Plan"]);
    then(&observation).shows_it_at_the_root();
    then(&observation).shows_objects(&[("sticker", 2), ("text", 1), ("shape", 1)]);
    then(&observation).shows_no_part();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_in_a_folder_is_shown_with_its_folder(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Work", "Team")
        .board("Plan", "Team", Some("Work"))
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .show_boards(&["Plan"], &Look::new())
        .await?;
    then(&observation).shows_it_in_a_folder();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn several_boards_are_shown_at_once(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .show_boards(&["Plans", "Groceries"], &Look::new())
        .await?;
    then(&observation).shows_boards(&["Plans", "Groceries"]);
    Ok(())
}

// The CLI shows the found first and ends as not found; a tool replies whole.
#[rstest]
#[tokio::test]
async fn a_board_not_found_among_several_is_told_after_the_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .show_boards(&["Plans", "Nowhere"], &Look::new())
        .await?;
    let exit = (surface == Surface::Cli).then(|| surface.not_found());
    then(&observation).reports_missing(1, exit);
    then(&observation).shows_only(&["Plans"]);
    Ok(())
}

#[rstest]
#[case::members("members", "editor")]
#[case::rights("rights", "[]")]
#[tokio::test]
async fn board_parts_are_shown_when_asked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] part: &str,
    #[case] text: &str,
) -> Result<(), ScenarioError> {
    let holst = plan_of_four()
        .member("Ann", "Team", WorkspaceRole::Member)
        .start()
        .await?;
    holst
        .on(surface)
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
    let plain = holst
        .on(surface)
        .show_boards(&["Plan"], &Look::new().refresh())
        .await?;
    then(&plain).shows_no_part();
    let asked = holst
        .on(surface)
        .show_boards(&["Plan"], &Look::new().with(part))
        .await?;
    then(&asked).shows_part(part, text);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_part_of_a_board_is_shown_by_frame(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    let observation = holst
        .on(surface)
        .show_boards(&["Plan"], &Look::new().frame("Q4"))
        .await?;
    then(&observation).shows_objects(&[("sticker", 2)]);
    Ok(())
}

#[rstest]
#[case::rectangle(Look::new().rectangle("0,0,400,300"))]
#[case::objects(Look::new().object("20000000-0000-4000-8000-000040960002").object("20000000-0000-4000-8000-000040960003").padding(0.0))]
#[tokio::test]
async fn a_part_of_a_board_is_shown_by_rectangle_and_object(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] look: Look<'_>,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    let observation = holst.on(surface).show_boards(&["Plan"], &look).await?;
    then(&observation).shows_objects(&[("frame", 1), ("sticker", 2)]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_frame_link_selects_that_part(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    let link = holst.object_link("Plan", 1)?;
    let observation = holst
        .on(surface)
        .show_boards(&[&link], &Look::new())
        .await?;
    then(&observation).shows_objects(&[("sticker", 2)]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_frame_is_not_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    let observation = holst
        .on(surface)
        .show_boards(&["Plan"], &Look::new().frame("Nowhere"))
        .await?;
    then(&observation)
        .failed_saying("Nowhere")
        .signalled_as(surface.not_found());
    Ok(())
}

// A copy is checked against the server first, so an edit behind its back shows.
#[rstest]
#[tokio::test]
async fn a_cached_board_is_checked_and_read_again_when_it_changed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_of_four().start().await?;
    let on = holst.on(surface);
    let first = on.show_boards(&["Plan"], &Look::new()).await?;
    then(&first).shows_objects(&[("sticker", 2), ("text", 1), ("shape", 1)]);
    holst.edit_board_behind_its_back(
        "Plan",
        vec![FixtureObject::Sticker("New".into())],
    )?;
    let changed = on.show_boards(&["Plan"], &Look::new()).await?;
    then(&changed).shows_objects(&[("sticker", 3), ("text", 1), ("shape", 1)]);
    let refreshed = on.show_boards(&["Plan"], &Look::new().refresh()).await?;
    then(&refreshed).shows_objects(&[("sticker", 3), ("text", 1), ("shape", 1)]);
    Ok(())
}

// Offline is a flag of the command line, by id.
#[tokio::test]
async fn an_offline_read_uses_the_copy() -> Result<(), ScenarioError> {
    let holst = plan_of_four().start().await?;
    let on = holst.on(Surface::Cli);
    let id = holst.board_id("Plan")?;
    on.show_boards(&["Plan"], &Look::new()).await?;
    holst.edit_board_behind_its_back(
        "Plan",
        vec![FixtureObject::Sticker("New".into())],
    )?;
    let offline = on.show_boards(&[&id], &Look::new().offline()).await?;
    then(&offline).shows_objects(&[("sticker", 2), ("text", 1), ("shape", 1)]);
    Ok(())
}

#[tokio::test]
async fn an_offline_read_without_a_copy_is_not_found() -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let on = holst.on(Surface::Cli);
    // A read online first: the login learns whose copies there are.
    on.show_boards(&["Groceries"], &Look::new()).await?;
    let id = holst.board_id("Plans")?;
    let observation = on.show_boards(&[&id], &Look::new().offline()).await?;
    then(&observation)
        .failed_saying("is not in the cache")
        .signalled_as(Surface::Cli.not_found());
    Ok(())
}

fn stored_as(schema_version: u32, provider: &'static str) -> Given {
    given().saved_login().workspace("Team").board_stored_as(
        "Old",
        "Team",
        schema_version,
        provider,
    )
}

const FORMAT_REFUSAL: &str = "cannot read or write";

#[rstest]
#[tokio::test]
async fn a_board_in_another_format_is_not_listed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[values((1, "hud"), (2, "hocuspocus"))] storage: (u32, &'static str),
) -> Result<(), ScenarioError> {
    let holst = stored_as(storage.0, storage.1).start().await?;
    let observation = holst.on(surface).objects("Old").await?;
    then(&observation)
        .failed_saying(FORMAT_REFUSAL)
        .signalled_as(surface.unavailable());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_in_another_format_takes_no_object(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[values((1, "hud"), (2, "hocuspocus"))] storage: (u32, &'static str),
) -> Result<(), ScenarioError> {
    let holst = stored_as(storage.0, storage.1).start().await?;
    let observation = holst
        .on(surface)
        .add_sticker("Old", "Call Ann", None)
        .await?;
    then(&observation)
        .failed_saying(FORMAT_REFUSAL)
        .signalled_as(surface.unavailable());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_v3_board_is_read_and_written(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = stored_as(3, "hud").start().await?;
    holst.edit_board_behind_its_back(
        "Old",
        vec![FixtureObject::Sticker("Buy milk".to_owned())],
    )?;
    let before = holst.on(surface).objects("Old").await?;
    then(&before).lists_objects(&[("sticker", "Buy milk")]);
    holst
        .on(surface)
        .add_sticker("Old", "Call Ann", None)
        .await?;
    board_of(&holst).got_one_update();
    let after = holst.on(surface).objects("Old").await?;
    then(&after).lists_objects_in_any_order(&[
        ("sticker", "Buy milk"),
        ("sticker", "Call Ann"),
    ]);
    let exported = holst
        .on(surface)
        .export_board("Old", "old.board", Export::new())
        .await?;
    then(&exported)
        .failed_saying(FORMAT_REFUSAL)
        .signalled_as(surface.unavailable());
    Ok(())
}
