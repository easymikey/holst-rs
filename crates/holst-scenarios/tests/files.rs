//! Backups and uploaded files.

use holst_fake::FixtureObject;
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
            "FROM-DRIVE",
            ("2026-10-02T00:00:00Z", "drive"),
        )
        .start()
        .await?;
    let on = holst.on(surface);
    then(
        &on.download_backup("snap-2", "from-drive.ydoc", Some("drive"), false)
            .await?,
    )
    .succeeded();
    assert_eq!(
        downloaded(&holst, "from-drive.ydoc").as_deref(),
        Some("FROM-DRIVE")
    );
    let observation = on
        .download_backup("snap-1", "from-elsewhere.ydoc", Some("elsewhere"), false)
        .await?;
    then(&observation)
        .failed_saying("not found")
        .signalled_as(surface.not_found());
    assert_eq!(downloaded(&holst, "from-elsewhere.ydoc"), None);
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

/// The header of a png of `width` by `height` pixels, which is all the CLI
/// and the tool read of it.
fn png(width: u32, height: u32) -> Vec<u8> {
    [
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13][..],
        b"IHDR",
        &width.to_be_bytes(),
        &height.to_be_bytes(),
        &[8, 2, 0, 0, 0],
    ]
    .concat()
}

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
    then(&observation).fails_saying("No such file");
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
        .with("size", "200x100");
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
