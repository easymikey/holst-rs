//! Searching boards by name and the text on them.

use holst_scenarios::{FixtureObject, ScenarioError, Search, Surface, given, then};
use rstest::rstest;

use crate::fixtures::two_boards_with_milk;

fn roadmaps() -> holst_scenarios::Given {
    given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .board("Road trip", "Team", None)
        .board("Retro", "Team", None)
}

#[rstest]
#[tokio::test]
async fn boards_are_found_by_part_of_their_name(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = roadmaps().start().await?;
    let observation = holst
        .on(surface)
        .search(Search::new("ROAD").mode("names"))
        .await?;
    then(&observation).finds_names(&[("board", "Roadmap"), ("board", "Road trip")]);
    Ok(())
}

#[rstest]
#[case("folder", "Road works")]
#[case("board", "Roadmap")]
#[tokio::test]
async fn names_search_finds_folders_and_boards(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] kind: &str,
    #[case] expected: &str,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .folder("Road works", "Team")
        .board("Roadmap", "Team", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .search(Search::new("road").mode("names").kind(kind))
        .await?;
    then(&observation).finds_names(&[(kind, expected)]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn names_search_is_limited_to_a_workspace(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .workspace("Private")
        .board("Roadmap", "Team", None)
        .board("Road trip", "Private", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .search(Search::new("road").mode("names").workspace("Team"))
        .await?;
    then(&observation).finds_names(&[("board", "Roadmap")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn the_trash_is_searched_only_when_asked(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Old plan", "Team", None)
        .start()
        .await?;
    holst.on(surface).delete_node("Old plan", "Team").await?;
    let without = holst
        .on(surface)
        .search(Search::new("old plan").mode("names"))
        .await?;
    then(&without).finds_names(&[]);
    let with = holst
        .on(surface)
        .search(Search::new("old plan").mode("names").in_trash())
        .await?;
    then(&with).finds_a_trashed("Old plan");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn board_text_search_is_limited_to_one_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .search(Search::new("milk").board("Groceries"))
        .await?;
    then(&observation).finds_texts(&[
        ("Groceries", "sticker", "Milk"),
        ("Groceries", "shape", "Milk run"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn board_text_search_filters_by_object_kind(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::FreeText("Buy milk".into()),
                FixtureObject::Sticker("Buy milk".into()),
            ],
        )
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .search(Search::new("milk").kind("sticker"))
        .await?;
    then(&observation).finds_texts(&[("Plans", "sticker", "Buy milk")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn both_modes_are_the_default(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk()
        .board("Milk run", "Team", None)
        .start()
        .await?;
    let observation = holst.on(surface).search(Search::new("milk")).await?;
    let then = then(&observation);
    then.finds_names(&[("board", "Milk run")]);
    then.finds_texts(&[
        ("Plans", "sticker", "Buy milk"),
        ("Groceries", "sticker", "Milk"),
        ("Groceries", "shape", "Milk run"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_search_with_no_hits_says_so(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst.on(surface).search(Search::new("zebra")).await?;
    then(&observation).finds_matches(0);
    Ok(())
}

#[rstest]
#[case::default(None, 20)]
#[case::all(Some(0), 25)]
#[tokio::test]
async fn the_default_limit_is_twenty(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] limit: Option<u32>,
    #[case] shown: usize,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .boards(25, "Team")
        .start()
        .await?;
    let search = Search::new("board").mode("names");
    let observation = holst
        .on(surface)
        .search(limit.map_or(search, |limit| search.limit(limit)))
        .await?;
    then(&observation).finds_matches(shown);
    Ok(())
}

// The CLI names the value and lists the modes on the next line.
#[rstest]
#[case(Surface::Cli, "invalid value 'nmaes' for '--mode <mode>'")]
#[case(Surface::Mcp, "must be one of names, board_text, both")]
#[tokio::test]
async fn a_misspelt_mode_is_refused_with_the_modes_listed(
    #[case] surface: Surface,
    #[case] refusal: &str,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .search(Search::new("milk").mode("nmaes"))
        .await?;
    then(&observation)
        .failed_saying(refusal)
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_board_filter_is_not_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = two_boards_with_milk().start().await?;
    let observation = holst
        .on(surface)
        .search(Search::new("milk").board("Nowhere"))
        .await?;
    then(&observation)
        .failed_saying("Nowhere")
        .signalled_as(surface.not_found());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn search_needs_a_login(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().workspace("Team").start().await?;
    let observation = holst.on(surface).search(Search::new("milk")).await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}
