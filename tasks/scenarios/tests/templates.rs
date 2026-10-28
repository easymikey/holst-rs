//! Listing and deleting board templates, the same on both surfaces.

use holst_scenarios::{
    FixtureObject,
    Given,
    ScenarioError,
    Surface,
    TemplateAudience,
    TemplateCommand,
    TemplatePart,
    given,
    template_of,
    templates_of,
    then,
    thumbnails_of,
    workspace_templates_of,
};
use rstest::rstest;
use serde_json::json;

fn with_templates() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .personal_template("Retro")
        .template_category("Strategic planning", "strategic-planning", &["Roadmap"])
}

#[rstest]
#[tokio::test]
async fn a_category_is_listed_with_its_slug(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst.on(surface).list_template_categories().await?;
    then(&observation).shows("strategic-planning");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_category_is_found_by_the_id_its_listing_showed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst
        .on(surface)
        .list_standard_templates_of_listed_category()
        .await?;
    then(&observation).shows("Roadmap");
    Ok(())
}

#[rstest]
#[case::name("Strategic planning")]
#[tokio::test]
async fn a_category_is_found_by_slug_or_name(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] category: &str,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst.on(surface).list_standard_templates(category).await?;
    then(&observation).shows("Roadmap");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_category_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst.on(surface).list_standard_templates("retro").await?;
    then(&observation)
        .failed_saying("Available: strategic-planning")
        .signalled_as(surface.not_found());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_delete_removes_the_template(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .delete_template("Retro")
        .await?;
    then(&observation).succeeded();
    templates_of(&holst).lacks("Retro");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_delete_without_confirmation_removes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst.on(surface).delete_template("Retro").await?;
    then(&observation)
        .failed_saying("cannot be undone")
        .signalled_as(surface.permission_denied());
    templates_of(&holst).has("Retro");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_delete_removes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .delete_template("Retro")
        .await?;
    then(&observation).shows("board_template/delete");
    templates_of(&holst).has("Retro");
    Ok(())
}

fn with_a_board() -> Given {
    with_templates().board_with_objects(
        "Plan",
        "Team",
        vec![
            FixtureObject::Sticker("Buy milk".into()),
            FixtureObject::Shape("Goal".into()),
            FixtureObject::Frame("Q4".into()),
        ],
    )
}

fn with_a_framed_board() -> Given {
    with_templates().board_with_objects(
        "Plan",
        "Team",
        vec![
            FixtureObject::Frame("Q4".into()),
            FixtureObject::Sticker("In frame".into())
                .at(150, 200)
                .inside(1),
            FixtureObject::Sticker("Also in frame".into())
                .at(400, 200)
                .inside(1),
            FixtureObject::Sticker("Outside".into()).at(3000, 3000),
            FixtureObject::Sticker("Far away".into()).at(5000, 5000),
        ],
    )
}

#[rstest]
#[tokio::test]
async fn workspace_templates_are_listed_apart_from_mine(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates()
        .workspace_template("Standup", "Team")
        .start()
        .await?;
    let mine = holst.on(surface).list_templates().await?;
    let shared = holst.on(surface).list_workspace_templates("Team").await?;
    then(&mine).shows("Retro");
    then(&mine).does_not_show("Standup");
    then(&shared).shows("Standup");
    then(&shared).does_not_show("Retro");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn standard_templates_are_listed_by_category(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates()
        .template_category("Retrospectives", "retrospectives", &["Start stop continue"])
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .list_standard_templates("retrospectives")
        .await?;
    then(&observation).shows("Start stop continue");
    then(&observation).does_not_show("Roadmap");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_template_is_shown_with_its_content(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates()
        .template_with_objects(
            "Plan",
            vec![
                FixtureObject::Sticker("Buy milk".into()),
                FixtureObject::Shape("Goal".into()),
                FixtureObject::Frame("Q4".into()),
            ],
        )
        .start()
        .await?;
    let observation = holst.on(surface).show_template("Plan").await?;
    then(&observation).shows("Plan");
    then(&observation).shows("sticker");
    then(&observation).shows("shape");
    then(&observation).shows("frame");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_template_is_not_found(
    #[values(TemplateCommand::Show, TemplateCommand::Remove)] command: TemplateCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given().saved_login().workspace("Team").start().await?;
    let observation = holst.on(surface).run_template_command(command).await?;
    then(&observation)
        .failed_saying("not found")
        .signalled_as(surface.not_found());
    templates_of(&holst).are(&[]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_ambiguous_template_name_lists_the_matches(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates()
        .template_category("Retrospectives", "retrospectives", &["Retro"])
        .start()
        .await?;
    let observation = holst.on(surface).show_template("Retro").await?;
    then(&observation)
        .failed_saying("matches several templates")
        .failed_saying("retrospectives/Retro")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_template_is_made_from_a_whole_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .create_template("Plan", "Roadmap", TemplatePart::Whole, TemplateAudience::Me)
        .await?;
    then(&observation).succeeded();
    template_of(&holst, "Roadmap")
        .holds_objects_of_kinds(&["sticker", "shape", "frame"]);
    thumbnails_of(&holst).are(1);
    Ok(())
}

#[rstest]
#[case::frame(TemplatePart::Frame("Q4"), &["frame", "sticker", "sticker"])]
#[case::objects(TemplatePart::Objects(&[4, 5]), &["sticker", "sticker"])]
#[tokio::test]
async fn a_template_is_made_from_a_frame_or_objects(
    #[case] part: TemplatePart<'_>,
    #[case] kinds: &[&str],
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_framed_board().start().await?;
    let observation = holst
        .on(surface)
        .create_template("Plan", "Part", part, TemplateAudience::Me)
        .await?;
    then(&observation).succeeded();
    template_of(&holst, "Part").holds_objects_of_kinds(kinds);
    Ok(())
}

#[rstest]
#[case::for_me(TemplateAudience::Me, &["Retro", "Roadmap"], &[])]
#[case::for_workspace(TemplateAudience::Workspace, &["Retro"], &["Roadmap"])]
#[tokio::test]
async fn a_template_is_for_me_or_the_workspace(
    #[case] audience: TemplateAudience,
    #[case] mine: &[&str],
    #[case] shared: &[&str],
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .create_template("Plan", "Roadmap", TemplatePart::Whole, audience)
        .await?;
    then(&observation).succeeded();
    templates_of(&holst).are(mine);
    workspace_templates_of(&holst, "Team").are(shared);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_template_dry_run_creates_and_uploads_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_board().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .create_template("Plan", "Roadmap", TemplatePart::Whole, TemplateAudience::Me)
        .await?;
    then(&observation).shows("board_template/create");
    templates_of(&holst).lacks("Roadmap");
    thumbnails_of(&holst).are(0);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_empty_selection_makes_no_template(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates()
        .board_with_objects("Plan", "Team", vec![FixtureObject::Frame("Empty".into())])
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .create_template(
            "Plan",
            "Roadmap",
            TemplatePart::Frame("Empty"),
            TemplateAudience::Me,
        )
        .await?;
    then(&observation)
        .failed_saying("Empty")
        .signalled_as(surface.data_error());
    templates_of(&holst).lacks("Roadmap");
    thumbnails_of(&holst).are(0);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_template_body_is_replaced(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .replace_template_body("Retro", r#"{"description":"Fresh start"}"#)
        .await?;
    then(&observation).succeeded();
    template_of(&holst, "Retro").has_field("description", &json!("Fresh start"));
    let shown = holst.on(surface).show_template("Retro").await?;
    then(&shown).shows("Fresh start");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_template_body_edit_is_refused_unconfirmed_or_broken(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_templates().start().await?;
    let unconfirmed = holst
        .on(surface)
        .replace_template_body("Retro", r#"{"description":"Fresh start"}"#)
        .await?;
    let broken = holst
        .on(surface)
        .replace_template_body("Retro", "not json")
        .await?;
    then(&unconfirmed)
        .failed_saying("cannot be undone")
        .signalled_as(surface.permission_denied());
    then(&broken)
        .failed_saying("body")
        .signalled_as(surface.usage());
    template_of(&holst, "Retro").lacks_field("description");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn template_commands_need_a_login(
    #[values(
        TemplateCommand::List,
        TemplateCommand::Show,
        TemplateCommand::Create,
        TemplateCommand::Edit,
        TemplateCommand::Remove
    )]
    command: TemplateCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .personal_template("Retro")
        .board("Plan", "Team", None)
        .start()
        .await?;
    let observation = holst.on(surface).run_template_command(command).await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    templates_of(&holst).are(&["Retro"]);
    Ok(())
}
