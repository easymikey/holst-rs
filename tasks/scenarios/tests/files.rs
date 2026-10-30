//! Backups and uploaded files.

use holst_core::fake::{FixtureObject, png};
use holst_scenarios::{
    CoverEdit,
    Given,
    Holst,
    ObjectCall,
    Observation,
    ScenarioError,
    Surface,
    board_of,
    downloaded,
    downloaded_bytes,
    given,
    objects_on,
    then,
    uploads_of,
};
use rstest::rstest;
use serde_json::{Value, json};

fn with_a_backup() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .backup("Roadmap", "snap-1", "SNAPSHOT-BYTES")
}

#[rstest]
#[tokio::test]
async fn a_backup_is_saved_to_a_new_file(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup().start().await?;
    let observation = holst
        .on(surface)
        .download_backup("snap-1", "snapshot.ydoc", None, false)
        .await?;
    then(&observation).succeeded();
    assert_eq!(
        downloaded(&holst, "snapshot.ydoc").as_deref(),
        Some("SNAPSHOT-BYTES")
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_backup_the_server_does_not_have_saves_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup().start().await?;
    let observation = holst
        .on(surface)
        .download_backup("snap-2", "snapshot.ydoc", None, false)
        .await?;
    then(&observation)
        .failed_saying("backup \"snap-2\" not found")
        .signalled_as(surface.not_found());
    assert_eq!(downloaded(&holst, "snapshot.ydoc"), None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn backups_are_listed_newest_first_with_provider_and_date(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None)
        .backup_made("Roadmap", "snap-old", "A", ("2026-08-01T00:00:00Z", "hud"))
        .backup_made("Roadmap", "snap-new", "B", ("2026-10-05T00:00:00Z", "hud"))
        .backup_made(
            "Roadmap",
            "snap-mid",
            "C",
            ("2026-09-01T00:00:00Z", "drive"),
        )
        .start()
        .await?;
    let observation = holst.on(surface).list_backups("Roadmap").await?;
    then(&observation).shows_in_order(&["snap-new", "snap-mid", "snap-old"]);
    then(&observation).shows("drive");
    then(&observation).shows("2026-09-01");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_board_without_backups_lists_none(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup().board("Retro", "Team", None).start().await?;
    let observation = holst.on(surface).list_backups("Retro").await?;
    then(&observation).does_not_show("snap-1");
    Ok(())
}

// Only the CLI has a default place to save to, and standard output.
#[rstest]
#[tokio::test]
async fn a_backup_goes_to_the_default_file_name() -> Result<(), ScenarioError> {
    let holst = with_a_backup().start().await?;
    let observation = holst
        .on(Surface::Cli)
        .download_backup_to_downloads("snap-1")
        .await?;
    then(&observation).succeeded();
    assert_eq!(
        downloaded(&holst, "Downloads/backup-snap-1.ydoc").as_deref(),
        Some("SNAPSHOT-BYTES")
    );
    Ok(())
}

#[tokio::test]
async fn a_backup_to_stdout() -> Result<(), ScenarioError> {
    let holst = with_a_backup().start().await?;
    let observation = holst
        .on(Surface::Cli)
        .download_backup_to_stdout("snap-1")
        .await?;
    then(&observation).shows("SNAPSHOT-BYTES");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_existing_backup_file_is_not_overwritten(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup()
        .local_file("snapshot.ydoc", b"MINE")
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .download_backup("snap-1", "snapshot.ydoc", None, false)
        .await?;
    then(&observation).signalled_as(surface.already_there());
    assert_eq!(downloaded(&holst, "snapshot.ydoc").as_deref(), Some("MINE"));
    Ok(())
}

/// A board with one backup, which the server sends gzip-packed or plain.
fn backup_sent(gzipped: bool) -> Given {
    let made = ("2026-10-01T00:00:00Z", "hud");
    let fixture = given()
        .saved_login()
        .workspace("Team")
        .board("Roadmap", "Team", None);
    if gzipped {
        fixture.gzipped_backup("Roadmap", "snap-1", "SNAPSHOT-BYTES", made)
    } else {
        fixture.backup_made("Roadmap", "snap-1", "SNAPSHOT-BYTES", made)
    }
}

/// Whichever way the server sends it, the file is plain or gzip as asked.
#[rstest]
#[case::plain_on_the_server(backup_sent(false))]
#[case::gzipped_on_the_server(backup_sent(true))]
#[tokio::test]
async fn a_backup_comes_plain_or_compressed(
    #[case] fixture: Given,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = fixture.start().await?;
    let on = holst.on(surface);
    then(
        &on.download_backup("snap-1", "plain.ydoc", None, false)
            .await?,
    )
    .succeeded();
    then(
        &on.download_backup("snap-1", "packed.ydoc", None, true)
            .await?,
    )
    .succeeded();
    assert_eq!(
        downloaded(&holst, "plain.ydoc").as_deref(),
        Some("SNAPSHOT-BYTES")
    );
    let packed = downloaded_bytes(&holst, "packed.ydoc.gz").unwrap_or_default();
    assert_eq!(packed.get(..2), Some(&[0x1f_u8, 0x8b][..]), "{packed:?}");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_backup_is_taken_from_the_named_provider(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup()
        .backup_made(
            "Roadmap",
            "snap-2",
            "FROM-OLD-STORE",
            ("2026-10-02T00:00:00Z", "hocuspocus"),
        )
        .start()
        .await?;
    let on = holst.on(surface);
    then(
        &on.download_backup("snap-2", "from-old.ydoc", Some("hocuspocus"), false)
            .await?,
    )
    .succeeded();
    assert_eq!(
        downloaded(&holst, "from-old.ydoc").as_deref(),
        Some("FROM-OLD-STORE")
    );
    let observation = on
        .download_backup("snap-1", "wrong-store.ydoc", Some("hocuspocus"), false)
        .await?;
    then(&observation)
        .failed_saying("not found")
        .signalled_as(surface.not_found());
    assert_eq!(downloaded(&holst, "wrong-store.ydoc"), None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unknown_backup_provider_is_a_usage_error(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup().start().await?;
    let observation = holst
        .on(surface)
        .download_backup("snap-1", "elsewhere.ydoc", Some("elsewhere"), false)
        .await?;
    then(&observation)
        .failed_saying("provider of `backup get` must be hud or hocuspocus")
        .signalled_as(surface.usage());
    assert_eq!(downloaded(&holst, "elsewhere.ydoc"), None);
    Ok(())
}

fn four_objects() -> Vec<FixtureObject> {
    ["One", "Two", "Three", "Four"]
        .map(|text| FixtureObject::Sticker(text.into()))
        .to_vec()
}

/// A board of four objects, and a backup of it from when it had two.
fn with_a_backup_of_two_objects() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board_with_objects("Plan", "Team", four_objects())
        .backup_of_objects(
            "Plan",
            "snap-1",
            vec![
                FixtureObject::Sticker("One".into()),
                FixtureObject::Sticker("Two".into()),
            ],
        )
}

#[rstest]
#[tokio::test]
async fn a_restored_backup_replaces_the_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup_of_two_objects().start().await?;
    let observation = holst
        .on(surface)
        .confirming()
        .restore_backup("Plan", "snap-1")
        .await?;
    then(&observation).succeeded();
    assert_eq!(objects_on(&holst, "Plan"), 2);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_restore_without_confirmation_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup_of_two_objects().start().await?;
    let observation = holst.on(surface).restore_backup("Plan", "snap-1").await?;
    then(&observation)
        .failed_saying("cannot be undone")
        .signalled_as(surface.permission_denied());
    assert_eq!(objects_on(&holst, "Plan"), 4);
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_restore_changes_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup_of_two_objects().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .restore_backup("Plan", "snap-1")
        .await?;
    then(&observation).shows_a_restore_of(2, 4);
    then(&observation).shows("2026-10-01");
    assert_eq!(objects_on(&holst, "Plan"), 4);
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_restore_of_an_unknown_key_is_not_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_backup_of_two_objects().start().await?;
    let observation = holst.on(surface).restore_backup("Plan", "snap-9").await?;
    then(&observation)
        .failed_saying("backup \"snap-9\" not found")
        .signalled_as(surface.not_found());
    assert_eq!(objects_on(&holst, "Plan"), 4);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_read_only_board_refuses_a_restore(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .read_only_board_with_objects("Plan", "Team", four_objects())
        .backup_of_objects("Plan", "snap-1", vec![FixtureObject::Sticker("One".into())])
        .start()
        .await?;
    let observation = holst.on(surface).restore_backup("Plan", "snap-1").await?;
    then(&observation).signalled_as(surface.permission_denied());
    assert_eq!(objects_on(&holst, "Plan"), 4);
    Ok(())
}

fn asks_to_sign_in(observation: &Observation, surface: Surface) {
    then(observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
}

#[rstest]
#[tokio::test]
async fn backup_and_settings_commands_need_a_login(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .board("Plan", "Team", None)
        .backup("Plan", "snap-1", "SNAPSHOT-BYTES")
        .start()
        .await?;
    let on = holst.on(surface);
    asks_to_sign_in(&on.list_backups("Plan").await?, surface);
    asks_to_sign_in(
        &on.download_backup("snap-1", "snapshot.ydoc", None, false)
            .await?,
        surface,
    );
    asks_to_sign_in(&on.restore_backup("Plan", "snap-1").await?, surface);
    asks_to_sign_in(
        &on.edit_board_settings("Plan", Some(CoverEdit::Reset))
            .await?,
        surface,
    );
    assert_eq!(downloaded(&holst, "snapshot.ydoc"), None);
    Ok(())
}

fn with_originals() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![
            FixtureObject::Image("cat.png".into()),
            FixtureObject::Sticker("Buy milk".into()),
            FixtureObject::RefusedImage("dog.png".into()),
        ],
    )
}

#[rstest]
#[tokio::test]
async fn an_original_is_downloaded(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_originals().start().await?;
    let call = ObjectCall::get()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("dir", holst.work_place("saved"));
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).succeeded();
    assert_eq!(
        downloaded(&holst, "saved/cat.png").as_deref(),
        Some("pixels")
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_sticker_has_no_original(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_originals().start().await?;
    let call = ObjectCall::get()
        .with("object", holst.fixture_object("Plans", 2)?)
        .with("dir", holst.work_place("saved"));
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("has no file to download")
        .signalled_as(surface.usage());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_refused_original_is_told(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_originals().start().await?;
    let call = ObjectCall::get()
        .with("object", holst.fixture_object("Plans", 3)?)
        .with("dir", holst.work_place("saved"));
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).fails_saying("dog.png");
    assert_eq!(downloaded(&holst, "saved/dog.png"), None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_existing_download_is_not_overwritten(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_originals()
        .local_file("mine.png", b"mine")
        .start()
        .await?;
    let call = ObjectCall::get()
        .file(&holst.work_place("mine.png"))
        .with("object", holst.fixture_object("Plans", 1)?);
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).signalled_as(surface.already_there());
    assert_eq!(downloaded(&holst, "mine.png").as_deref(), Some("mine"));
    Ok(())
}

// Only MCP keeps the login and the keys out of what it writes.
#[rstest]
#[tokio::test]
async fn an_original_is_not_saved_over_the_login() -> Result<(), ScenarioError> {
    let holst = with_originals().start().await?;
    let call = ObjectCall::get()
        .file(&holst.work_place("../holst/session"))
        .with("object", holst.fixture_object("Plans", 1)?);
    let observation = holst.on(Surface::Mcp).object("Plans", call).await?;
    then(&observation).fails_saying("is a secret place");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_original_is_not_saved_into_the_keys_folder() -> Result<(), ScenarioError> {
    let holst = with_originals().start().await?;
    let call = ObjectCall::get()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("dir", holst.work_place(".ssh"));
    let observation = holst.on(Surface::Mcp).object("Plans", call).await?;
    then(&observation).fails_saying("is a secret place");
    assert_eq!(downloaded(&holst, ".ssh/cat.png"), None);
    Ok(())
}

const PAGE: &str = "https://example.com/spec";

/// The call that adds a `kind` of media from what `with_media` holds.
fn media_call(holst: &Holst, kind: &str) -> ObjectCall {
    match kind {
        "link" => ObjectCall::add("link").url(PAGE),
        "image" => ObjectCall::add("image").path(&holst.work_place("cat.png")),
        _ => ObjectCall::add("file").path(&holst.work_place("report.pdf")),
    }
}

/// The path `pointer` in the first object of `kind` on the board "Plans".
fn in_first(holst: &Holst, kind: &str, pointer: &str) -> Result<Value, ScenarioError> {
    Ok(holst
        .first_of_kind("Plans", kind)?
        .pointer(pointer)
        .cloned()
        .unwrap_or(Value::Null))
}

/// A board with no objects, and a picture and a pdf in the folder the
/// binaries run from.
fn with_media() -> Given {
    given()
        .saved_login()
        .workspace("Team")
        .board("Plans", "Team", None)
        .local_file("cat.png", &png(16, 16))
        .local_file("report.pdf", b"%PDF-1.4 report")
}

#[rstest]
#[tokio::test]
async fn a_link_card_takes_the_page_title(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().link_preview(PAGE, "Spec").start().await?;
    let call = media_call(&holst, "link");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[("link", "")]);
    assert_eq!(in_first(&holst, "link", "/link")?, json!(PAGE));
    assert_eq!(in_first(&holst, "link", "/linkInfo/title")?, json!("Spec"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_image_is_uploaded_and_placed(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let call = media_call(&holst, "image");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    uploads_of(&holst).holds_one_named("cat.png");
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[("image", "")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_file_is_uploaded_and_gets_a_card(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let call = media_call(&holst, "file");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    uploads_of(&holst).holds("report.pdf", "%PDF-1.4 report");
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[("file", "")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_uploaded_image_can_be_fetched_back(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let added = media_call(&holst, "image");
    then(&holst.on(surface).object("Plans", added).await?).succeeded();
    let call = ObjectCall::get()
        .with(
            "object",
            in_first(&holst, "image", "/id")?
                .as_str()
                .unwrap_or_default(),
        )
        .with("dir", holst.work_place("saved"));
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(downloaded_bytes(&holst, "saved/cat.png"), Some(png(16, 16)));
    Ok(())
}

#[rstest]
#[case::image("image")]
#[case::file("file")]
#[case::link("link")]
#[tokio::test]
async fn a_dry_run_uploads_and_places_nothing(
    #[case] kind: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let call = media_call(&holst, kind);
    holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    uploads_of(&holst).is_empty();
    board_of(&holst).got_no_update();
    assert_eq!(objects_on(&holst, "Plans"), 0);
    Ok(())
}

// Gap: the scenario asks for not found (66), core fails the read (74).
#[rstest]
#[tokio::test]
async fn a_missing_local_file_is_not_found(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let call = ObjectCall::add("image").path(&holst.work_place("nope.png"));
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).fails_saying("os error 2");
    uploads_of(&holst).is_empty();
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_read_only_board_is_refused_before_the_upload(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .saved_login()
        .workspace("Team")
        .read_only_board_with_objects("Plans", "Team", Vec::new())
        .local_file("cat.png", &png(16, 16))
        .start()
        .await?;
    let call = media_call(&holst, "image");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation).signalled_as(surface.permission_denied());
    uploads_of(&holst).is_empty();
    board_of(&holst).got_no_update();
    Ok(())
}

// The page cannot be previewed: the card is the bare address, with a warning.
#[rstest]
#[tokio::test]
async fn a_link_to_a_page_without_preview_gets_a_bare_card(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().failing_link_preview(PAGE).start().await?;
    let call = ObjectCall::add("link").url(PAGE);
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    board_of(&holst).got_one_update();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[("link", "")]);
    assert_eq!(in_first(&holst, "link", "/link")?, json!(PAGE));
    assert_eq!(in_first(&holst, "link", "/linkInfo/title")?, Value::Null);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_image_and_a_sticker_go_in_one_write(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let call = ObjectCall::add("sticker").items(vec![
        json!({ "text": "Note" }),
        json!({ "kind": "image", "path": holst.work_place("cat.png") }),
    ]);
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    board_of(&holst).got_one_update();
    uploads_of(&holst).holds_one_named("cat.png");
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation)
        .lists_objects_in_any_order(&[("sticker", "Note"), ("image", "")]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_image_is_placed_at_a_corner_and_size(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_media().start().await?;
    let call = ObjectCall::add("image")
        .path(&holst.work_place("cat.png"))
        .with("at", "10,10")
        .with("size", "200,100");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_object_at("", (10, 10));
    then(&observation).lists_object_sized("", "200x100");
    Ok(())
}

#[rstest]
#[case::link("link")]
#[case::image("image")]
#[case::file("file")]
#[tokio::test]
async fn media_commands_need_a_login(
    #[case] kind: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .board("Plans", "Team", None)
        .local_file("cat.png", &png(16, 16))
        .local_file("report.pdf", b"%PDF-1.4 report")
        .start()
        .await?;
    let call = media_call(&holst, kind);
    let observation = holst.on(surface).object("Plans", call).await?;
    asks_to_sign_in(&observation, surface);
    uploads_of(&holst).is_empty();
    Ok(())
}

/// A board with an image, a file and a sticker.
fn with_an_image_a_file_and_a_sticker() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![
            FixtureObject::Image("cat.png".into()),
            FixtureObject::File("plan.pdf".into(), 70),
            FixtureObject::Sticker("Buy milk".into()),
        ],
    )
}

#[rstest]
#[tokio::test]
async fn an_image_and_a_file_are_renamed_and_read_back(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let image = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("name", "Logo.png");
    then(&holst.on(surface).object("Plans", image).await?).succeeded();
    let file = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 2)?)
        .with("name", " Plan.pdf ");
    then(&holst.on(surface).object("Plans", file).await?).succeeded();
    assert_eq!(
        in_first(&holst, "image", "/originalName")?,
        json!("Logo.png")
    );
    assert_eq!(
        in_first(&holst, "file", "/displayFileName")?,
        json!("Plan.pdf")
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_sticker_takes_no_name(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 3)?)
        .with("name", "Logo");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("name")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_blank_name_of_an_image_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("name", "  ");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("an image or a file needs a visible name")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_a_rename_sends_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("name", "Logo.png");
    then(
        &holst
            .on(surface)
            .dry_running()
            .object("Plans", call)
            .await?,
    )
    .succeeded();
    board_of(&holst).got_no_update();
    assert_eq!(
        in_first(&holst, "image", "/originalName")?,
        json!("cat.png")
    );
    Ok(())
}

/// The numbers of a stored array, `null` as none.
fn numbers_at(holst: &Holst, pointer: &str) -> Result<Vec<Option<f64>>, ScenarioError> {
    Ok(in_first(holst, "image", pointer)?
        .as_array()
        .map(|values| values.iter().map(Value::as_f64).collect())
        .unwrap_or_default())
}

#[rstest]
#[tokio::test]
async fn an_image_is_cropped_to_a_circle_and_read_back(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let circle = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("crop", "circle");
    then(&holst.on(surface).object("Plans", circle).await?).succeeded();
    let cut = (
        numbers_at(&holst, "/cropTransform")?,
        in_first(&holst, "image", "/width")?.as_f64(),
        in_first(&holst, "image", "/position/x")?.as_f64(),
    );
    let whole = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("crop", "none");
    then(&holst.on(surface).object("Plans", whole).await?).succeeded();
    assert_eq!(
        (cut, numbers_at(&holst, "/cropTransform")?),
        (
            (
                vec![Some(0.125), Some(0.0), Some(0.75), Some(1.0), Some(1.0)],
                Some(150.0),
                Some(125.0)
            ),
            vec![Some(0.0), Some(0.0), Some(1.0), Some(1.0), None]
        )
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_image_gets_an_outline_and_round_corners(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let outline = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("stroke_color", "gray12")
        .with("stroke_width", "4")
        .with("stroke_style", "dashed")
        .with("radius", "20");
    then(&holst.on(surface).object("Plans", outline).await?).succeeded();
    let drawn = holst.on(surface).listed_media("Plans", "image").await?;
    let circle = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("crop", "circle");
    then(&holst.on(surface).object("Plans", circle).await?).succeeded();
    let round = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("radius", "10");
    then(&holst.on(surface).object("Plans", round).await?)
        .failed_saying("circle-cropped image objects have no radius to change")
        .signalled_as(surface.usage());
    assert_eq!(
        drawn,
        json!({
            "stroke_color": "gray12",
            "stroke_width": 4.0,
            "stroke_style": "dashed",
            "radius": 20.0,
        })
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_flip_is_set_not_toggled(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let mut seen = Vec::new();
    // imperative: each call is a write, read back before the next
    for axes in ["horizontal", "horizontal", "both", "none"] {
        let call = ObjectCall::edit()
            .with("object", holst.fixture_object("Plans", 1)?)
            .with("flip_image", axes);
        then(&holst.on(surface).object("Plans", call).await?).succeeded();
        seen.push(holst.on(surface).listed_media("Plans", "image").await?);
    }
    assert_eq!(
        seen,
        vec![
            json!({"flip": "horizontal"}),
            json!({"flip": "horizontal"}),
            json!({"flip": "both"}),
            Value::Null,
        ]
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_crop_beside_a_size_is_refused(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_image_a_file_and_a_sticker().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("crop", "square")
        .with("size", "100,100");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("left out with --crop")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

/// A board with an image, a file and a sticker, and a picture and a text on
/// disk.
fn with_a_new_picture() -> Given {
    with_an_image_a_file_and_a_sticker()
        .local_file("dog.png", &png(32, 16))
        .local_file("notes.txt", b"just words")
}

#[rstest]
#[tokio::test]
async fn an_image_takes_a_new_file(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_new_picture().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("path", holst.work_place("dog.png"));
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    uploads_of(&holst).holds_one_named("dog.png");
    let observation = holst.on(surface).objects("Plans").await?;
    then(&observation).lists_objects(&[
        ("image", ""),
        ("file", ""),
        ("sticker", "Buy milk"),
    ]);
    assert_eq!(
        (
            in_first(&holst, "image", "/originalName")?,
            in_first(&holst, "image", "/naturalWidth")?,
            numbers_at(&holst, "/cropTransform")?,
        ),
        (
            json!("dog.png"),
            json!(32),
            vec![Some(0.166_667), Some(0.0), Some(0.666_667), Some(1.0), None]
        )
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_replacement_uploads_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_new_picture().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("path", holst.work_place("dog.png"));
    then(
        &holst
            .on(surface)
            .dry_running()
            .object("Plans", call)
            .await?,
    )
    .succeeded();
    uploads_of(&holst).is_empty();
    board_of(&holst).got_no_update();
    assert_eq!(
        in_first(&holst, "image", "/originalName")?,
        json!("cat.png")
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_sticker_takes_no_path(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_new_picture().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 3)?)
        .with("path", holst.work_place("dog.png"));
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("path")
        .signalled_as(surface.usage());
    uploads_of(&holst).is_empty();
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_text_file_is_not_a_picture(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_new_picture().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("path", holst.work_place("notes.txt"));
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("a png, jpg, gif or webp picture")
        .signalled_as(surface.usage());
    uploads_of(&holst).is_empty();
    board_of(&holst).got_no_update();
    Ok(())
}

/// A board with a three-page pdf whose pages the client has counted.
fn with_a_counted_pdf() -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![FixtureObject::Set(
            Box::new(FixtureObject::File("plan.pdf".into(), 70)),
            "pagesInfo".into(),
            json!({"count": 3, "valid": true}),
        )],
    )
}

#[rstest]
#[tokio::test]
async fn a_pdf_is_shown_as_pages_from_page_two(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_counted_pdf().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("display", "content")
        .with("start_page", "2");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        holst.on(surface).listed_media("Plans", "file").await?,
        json!({"display": "content", "start_page": 2, "pages": 3})
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_start_page_past_the_last_page_is_refused_on_the_board(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_counted_pdf().start().await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("start_page", "4");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("the file has no page 4: it has 3")
        .signalled_as(surface.usage());
    board_of(&holst).got_no_update();
    Ok(())
}

/// A board with a pdf whose pages nobody has counted yet.
fn with_an_uncounted(file: &str) -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![FixtureObject::File(file.into(), 70)],
    )
}

#[rstest]
#[tokio::test]
async fn a_pdf_without_pages_is_counted_then_shown_as_pages(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_uncounted("plan.pdf")
        .page_count("plan.pdf", 3)
        .start()
        .await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("display", "content");
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        (
            holst.page_counts_asked(),
            holst.on(surface).listed_media("Plans", "file").await?,
        ),
        (1, json!({"display": "content", "pages": 3}))
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn an_unconvertible_file_says_why(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_uncounted("plan.docx")
        .unconvertible_file(
            "plan.docx",
            "office_conversion_unavailable",
            "no converter",
        )
        .start()
        .await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("display", "content");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("office_conversion_unavailable: no converter")
        .signalled_as(surface.data_error());
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_showing_pages_asks_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_uncounted("plan.pdf")
        .page_count("plan.pdf", 3)
        .start()
        .await?;
    let call = ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("display", "content");
    let observation = holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    then(&observation).succeeded();
    assert_eq!(holst.page_counts_asked(), 0);
    board_of(&holst).got_no_update();
    Ok(())
}

/// A board with a pdf of `pages` pages counted at 100,200; the page service
/// is told nothing yet.
fn with_a_pdf_of(pages: u32) -> Given {
    given().saved_login().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![FixtureObject::Set(
            Box::new(FixtureObject::Set(
                Box::new(FixtureObject::File("plan.pdf".into(), 70)),
                "pagesInfo".into(),
                json!({"count": pages, "valid": true}),
            )),
            "position".into(),
            json!({"x": 100, "y": 200}),
        )],
    )
}

/// The page objects of "Plans", as (page, x, y) in page order.
fn extracted_pages(holst: &Holst) -> Result<Vec<(u64, f64, f64)>, ScenarioError> {
    let mut found: Vec<(u64, f64, f64)> = holst
        .all_of_kind("Plans", "file")?
        .iter()
        .filter(|object| object.pointer("/extracted") == Some(&json!(true)))
        .map(|object| {
            let at = |pointer: &str| {
                object
                    .pointer(pointer)
                    .and_then(Value::as_f64)
                    .unwrap_or(f64::NAN)
            };
            (
                object
                    .pointer("/pinnedPage")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                at("/position/x"),
                at("/position/y"),
            )
        })
        .collect();
    found.sort_by_key(|(page, _, _)| *page);
    Ok(found)
}

fn extract_call(holst: &Holst, pages: &str) -> Result<ObjectCall, ScenarioError> {
    Ok(ObjectCall::edit()
        .with("object", holst.fixture_object("Plans", 1)?)
        .with("extract", pages))
}

fn runs(list: &[(u32, u32)]) -> Vec<(String, u32, u32)> {
    list.iter()
        .map(|(start, end)| ("plan.pdf".to_owned(), *start, *end))
        .collect()
}

#[rstest]
#[tokio::test]
async fn three_pages_of_a_pdf_are_extracted_in_a_row(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_pdf_of(3).page_count("plan.pdf", 3).start().await?;
    let call = extract_call(&holst, "all")?;
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        (
            holst.page_runs(),
            extracted_pages(&holst)?,
            holst.all_of_kind("Plans", "file")?.len(),
        ),
        (
            runs(&[(1, 3)]),
            vec![(1, 100.0, 328.0), (2, 404.0, 328.0), (3, 708.0, 328.0)],
            4,
        )
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn pages_1_to_25_take_two_runs(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_pdf_of(30).page_count("plan.pdf", 30).start().await?;
    let call = extract_call(&holst, "1-25")?;
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    let pages = extracted_pages(&holst)?;
    assert_eq!(
        (holst.page_runs(), pages.len(), pages.get(8).copied()),
        (runs(&[(1, 20), (21, 25)]), 25, Some((9, 100.0, 456.0)))
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_failed_run_adds_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_pdf_of(30)
        .unconvertible_file("plan.pdf", "conversion_failed", "no converter")
        .start()
        .await?;
    let call = extract_call(&holst, "1-25")?;
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("conversion_failed: no converter")
        .signalled_as(surface.data_error());
    assert_eq!(
        (holst.page_runs(), objects_on(&holst, "Plans")),
        (runs(&[(1, 20)]), 1)
    );
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_file_without_pages_is_counted_then_extracted(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_uncounted("plan.pdf")
        .page_count("plan.pdf", 3)
        .start()
        .await?;
    let call = extract_call(&holst, "all")?;
    then(&holst.on(surface).object("Plans", call).await?).succeeded();
    assert_eq!(
        (
            holst.page_counts_asked(),
            holst.page_runs(),
            extracted_pages(&holst)?.len(),
        ),
        (1, runs(&[(1, 3)]), 3)
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_file_the_service_cannot_count_refuses_extract(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_an_uncounted("plan.pdf")
        .unconvertible_file("plan.pdf", "office_conversion_unavailable", "no converter")
        .start()
        .await?;
    let call = extract_call(&holst, "all")?;
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("office_conversion_unavailable: no converter")
        .signalled_as(surface.data_error());
    assert_eq!(holst.page_runs(), runs(&[]));
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_dry_run_of_extract_converts_nothing(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_pdf_of(3).page_count("plan.pdf", 3).start().await?;
    let call = extract_call(&holst, "all")?;
    let observation = holst
        .on(surface)
        .dry_running()
        .object("Plans", call)
        .await?;
    then(&observation).succeeded();
    assert_eq!(holst.page_runs(), runs(&[]));
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn extract_takes_no_other_field(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_pdf_of(3).page_count("plan.pdf", 3).start().await?;
    let call = extract_call(&holst, "all")?.with("color", "yellow4");
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("left out with --extract")
        .signalled_as(surface.usage());
    assert_eq!(holst.page_runs(), runs(&[]));
    board_of(&holst).got_no_update();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn more_than_fifty_pages_are_refused_before_any_run(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_a_pdf_of(60).page_count("plan.pdf", 60).start().await?;
    let call = extract_call(&holst, "1-51")?;
    let observation = holst.on(surface).object("Plans", call).await?;
    then(&observation)
        .failed_saying("must be at most 50 pages")
        .signalled_as(surface.usage());
    assert_eq!(holst.page_runs(), runs(&[]));
    board_of(&holst).got_no_update();
    Ok(())
}
