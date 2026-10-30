//! Exporting a board as a picture, and saving it as a folder.

use holst_scenarios::{
    Export,
    FixtureObject,
    Given,
    Import,
    Look,
    PictureFormat,
    ScenarioError,
    Surface,
    board_of,
    downloads_of,
    exported_file,
    given,
    picture,
    saved_in,
    then,
    tree_of,
};
use rstest::rstest;

use crate::fixtures::plan_with_a_frame;

fn plans_with_a_sticker() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![FixtureObject::Sticker("Buy milk".into())],
    )
}

#[rstest]
#[case::svg(PictureFormat::Svg)]
#[case::png(PictureFormat::Png)]
#[tokio::test]
async fn a_board_picture_comes_in_the_asked_format(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] picture: PictureFormat,
) -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    let observation = holst.on(surface).export_picture("Plans", picture).await?;
    then(&observation).is_a_picture(picture);
    Ok(())
}

// By board id: `@N` does not outlive one MCP server process.
#[rstest]
#[tokio::test]
async fn a_board_picture_is_asked_for_by_its_id(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    let id = holst.board_id("Plans")?;
    let observation = holst
        .on(surface)
        .export_picture(&id, PictureFormat::Svg)
        .await?;
    then(&observation).is_a_picture(PictureFormat::Svg);
    Ok(())
}

#[tokio::test]
async fn a_file_that_is_neither_svg_nor_png_is_refused() -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    let observation = holst
        .snap_to_file_with("Plans", "plans.jpg", &Look::new())
        .await?;
    then(&observation)
        .failed_saying("must be a name that ends in .svg or .png")
        .signalled_as(Surface::Cli.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_saved_board_is_one_folder_with_its_pictures_and_objects(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    holst.on(surface).save_board("Plans", &Look::new()).await?;
    saved_in(&holst).is_one_folder_of(
        "Plans",
        &["board.png", "board.svg", "images/", "objects.json"],
    );
    Ok(())
}

// A refused original is listed, not fatal: public storage needs no session.
#[rstest]
#[tokio::test]
async fn a_refused_image_is_listed_as_not_saved(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Image("cat.png".into()),
                FixtureObject::RefusedImage("dog.png".into()),
            ],
        )
        .start()
        .await?;
    let observation = holst.on(surface).save_board("Plans", &Look::new()).await?;
    then(&observation).lists_not_saved(1);
    saved_in(&holst).is_one_folder_of(
        "Plans",
        &[
            "board.png",
            "board.svg",
            "images/",
            "images/cat.png",
            "objects.json",
        ],
    );
    Ok(())
}

#[tokio::test]
async fn a_picture_goes_to_the_default_file_name() -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    holst.snap_to_default("Plans").await?;
    downloads_of(&holst).holds_a_png_of("Plans");
    Ok(())
}

#[tokio::test]
async fn a_picture_to_stdout_is_a_picture() -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    let observation = holst.snap_to_stdout("Plans").await?;
    then(&observation).is_a_picture(PictureFormat::Png);
    Ok(())
}

#[tokio::test]
async fn an_existing_picture_file_is_not_overwritten() -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    holst
        .snap_to_file_with("Plans", "plans.png", &Look::new())
        .await?;
    let before = picture(&holst, "plans.png");
    let observation = holst
        .snap_to_file_with("Plans", "plans.png", &Look::new())
        .await?;
    then(&observation).signalled_as(Surface::Cli.already_there());
    picture(&holst, "plans.png").is_unchanged_since(&before);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_part_of_a_board_is_drawn_alone(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    let on = holst.on(surface);
    let whole = on
        .drawn_size("Plan", &Look::new(), PictureFormat::Svg)
        .await?;
    let part = on
        .drawn_size("Plan", &Look::new().frame("Q4"), PictureFormat::Svg)
        .await?;
    then(&part).is_smaller_than(&whole);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_picture_is_scaled_and_limited_in_size(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    let on = holst.on(surface);
    let frame = Look::new().frame("Q4");
    let once = on.drawn_size("Plan", &frame, PictureFormat::Png).await?;
    let twice = on
        .drawn_size("Plan", &frame.clone().scale(2.0), PictureFormat::Png)
        .await?;
    then(&twice).is_as_wide_as(&once, 2);
    let limited = on
        .drawn_size("Plan", &frame.max_side(100), PictureFormat::Png)
        .await?;
    then(&limited).fits_in(100);
    Ok(())
}

// Embedding is a flag of the command line; a tool reply carries the SVG as is.
#[tokio::test]
async fn an_embedded_picture_carries_its_images() -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![FixtureObject::Image("cat.svg".into())],
        )
        .start()
        .await?;
    holst
        .snap_to_file_with("Plans", "linked.svg", &Look::new())
        .await?;
    holst
        .snap_to_file_with("Plans", "embedded.svg", &Look::new().embed())
        .await?;
    picture(&holst, "linked.svg").links_its_images();
    picture(&holst, "embedded.svg").embeds_its_images();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_saved_part_of_a_board_holds_only_that_part(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plan_with_a_frame().start().await?;
    holst
        .on(surface)
        .save_board("Plan", &Look::new().frame("Q4"))
        .await?;
    saved_in(&holst).holds_objects_of("Q4", &["Q4", "In one", "In two"]);
    Ok(())
}

// A folder of that name is never replaced: the next one takes the next number.
#[rstest]
#[tokio::test]
async fn a_save_beside_an_existing_folder_takes_the_next_name(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_a_sticker().start().await?;
    let on = holst.on(surface);
    on.save_board("Plans", &Look::new()).await?;
    on.save_board("Plans", &Look::new()).await?;
    saved_in(&holst).is_folders_of("Plans", 2);
    Ok(())
}

fn plans_with_objects() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .folder("Work", "Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Sticker("Buy milk".into()),
                FixtureObject::FreeText("Roadmap".into()),
            ],
        )
        .board("Empty", "Team", None)
        .board_with_objects(
            "Notes",
            "Team",
            vec![FixtureObject::Sticker("Old note".into())],
        )
}

const PLANS_OBJECTS: [(&str, &str); 2] = [("sticker", "Buy milk"), ("text", "Roadmap")];

#[rstest]
#[case::compressed(Export::new(), true)]
#[case::plain(Export::new().uncompressed(), false)]
#[tokio::test]
async fn an_export_carries_compressed_or_plain_bytes(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] how: Export,
    #[case] gzip: bool,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface);
    let exported = on.export_board("Plans", "plans.board", how).await?;
    then(&exported).succeeded();
    exported_file(&holst, "plans.board").is_gzip(gzip);
    let imported = on
        .import_board("plans.board", Import::new().name("Copy"))
        .await?;
    then(&imported).succeeded();
    let copy = on.objects("Copy").await?;
    then(&copy).lists_objects_in_any_order(&PLANS_OBJECTS);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_export_with_an_object_list_lists_the_objects(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    holst
        .on(surface)
        .export_board("Plans", "plans.board", Export::new().object_list())
        .await?;
    exported_file(&holst, "plans.board.objects.jsonl").lists_objects(&PLANS_OBJECTS);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_export_to_an_existing_file_is_not_overwritten(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface);
    on.export_board("Plans", "plans.board", Export::new())
        .await?;
    let observation = on
        .export_board("Notes", "plans.board", Export::new())
        .await?;
    then(&observation).signalled_as(surface.already_there());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_import_creates_a_new_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface);
    on.export_board("Plans", "plans.board", Export::new())
        .await?;
    let imported = on
        .import_board(
            "plans.board",
            Import::new().workspace("Team").folder("Work").name("Plan2"),
        )
        .await?;
    then(&imported).succeeded();
    tree_of(&holst, "Team").has_board("Plan2", Some("Work"));
    let objects = on.objects("Plan2").await?;
    then(&objects).lists_objects_in_any_order(&PLANS_OBJECTS);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_import_without_a_name_takes_the_file_name(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface);
    on.export_board("Plans", "Holiday.ydoc.gz", Export::new())
        .await?;
    on.import_board("Holiday.ydoc.gz", Import::new()).await?;
    tree_of(&holst, "Team").has_board("Holiday", None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_import_dry_run_creates_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    holst
        .on(surface)
        .export_board("Plans", "plans.board", Export::new())
        .await?;
    holst
        .on(surface)
        .dry_running()
        .import_board("plans.board", Import::new().name("Plan2"))
        .await?;
    tree_of(&holst, "Team").has_nothing_named("Plan2");
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_import_into_a_board_is_refused_unconfirmed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    holst
        .on(surface)
        .export_board("Plans", "plans.board", Export::new())
        .await?;
    let observation = holst
        .on(surface)
        .import_board("plans.board", Import::new().into("Empty"))
        .await?;
    then(&observation).signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_import_into_an_empty_board_fills_it(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface).confirming();
    on.export_board("Plans", "plans.board", Export::new())
        .await?;
    let imported = on
        .import_board("plans.board", Import::new().into("Empty"))
        .await?;
    then(&imported).succeeded();
    let objects = on.objects("Empty").await?;
    then(&objects).lists_objects_in_any_order(&PLANS_OBJECTS);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_that_holds_objects_is_not_overwritten(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface);
    on.export_board("Plans", "plans.board", Export::new())
        .await?;
    let observation = on
        .import_board("plans.board", Import::new().into("Notes"))
        .await?;
    then(&observation)
        .failed_saying("already has 1 objects")
        .signalled_as(surface.permission_denied());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_merged_import_keeps_what_is_there(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    let on = holst.on(surface).confirming();
    on.export_board("Plans", "plans.board", Export::new())
        .await?;
    let imported = on
        .import_board("plans.board", Import::new().into("Notes").merge())
        .await?;
    then(&imported).succeeded();
    let objects = on.objects("Notes").await?;
    then(&objects).lists_objects_in_any_order(&[
        ("sticker", "Buy milk"),
        ("text", "Roadmap"),
        ("sticker", "Old note"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_import_of_a_broken_file_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects()
        .local_file("broken.board", b"this is not a board")
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .import_board("broken.board", Import::new())
        .await?;
    then(&observation).signalled_as(surface.data_error());
    tree_of(&holst, "Team").has_nothing_named("broken");
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Whole {
    Show,
    Snap,
    Save,
    Export,
    Import,
}

async fn run_whole(
    holst: &holst_scenarios::Holst,
    surface: Surface,
    verb: Whole,
    board: &str,
) -> Result<holst_scenarios::Observation, ScenarioError> {
    let on = holst.on(surface);
    match verb {
        Whole::Show => on.show_boards(&[board], &Look::new()).await,
        Whole::Snap => on.export_picture(board, PictureFormat::Png).await,
        Whole::Save => on.save_board(board, &Look::new()).await,
        Whole::Export => on.export_board(board, "x.board", Export::new()).await,
        Whole::Import => on.import_board("x.board", Import::new().into(board)).await,
    }
}

#[rstest]
#[case::show(Whole::Show)]
#[case::snap(Whole::Snap)]
#[case::save(Whole::Save)]
#[case::export(Whole::Export)]
#[case::import(Whole::Import)]
#[tokio::test]
async fn an_unknown_board_is_not_found_by_the_whole_board_commands(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] verb: Whole,
) -> Result<(), ScenarioError> {
    let holst = plans_with_objects().start().await?;
    holst
        .on(surface)
        .export_board("Plans", "x.board", Export::new())
        .await?;
    let observation = run_whole(&holst, surface, verb, "Nowhere").await?;
    then(&observation)
        .failed_saying("Nowhere")
        .signalled_as(surface.not_found());
    Ok(())
}

#[rstest]
#[case::show(Whole::Show)]
#[case::snap(Whole::Snap)]
#[case::save(Whole::Save)]
#[case::export(Whole::Export)]
#[case::import(Whole::Import)]
#[tokio::test]
async fn whole_board_commands_need_a_login(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
    #[case] verb: Whole,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .local_file("x.board", b"a board file")
        .start()
        .await?;
    let observation = run_whole(&holst, surface, verb, "Plans").await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}
