//! Changing the objects of a board against the fake Holst server.

use holst_core::{
    command::{
        board::{EditArgs as EditBoardArgs, settings},
        board_write::{WriteRequest, write_cached},
        login::connect,
        lookup,
        object::{
            AddArgs,
            CopyArgs,
            EditArgs as EditObjectArgs,
            add as object_add,
            change as object_change,
            copy as object_copy,
        },
        table,
    },
    domain::{
        board::objects,
        change::value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
        geometry::Point,
        session::Session,
    },
    effect::{confirmation::ALWAYS, settings::CacheSettings},
    error::{Error, board_change::BoardChangeError},
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
    remote::{
        client::Client,
        socket::{
            load::load,
            write::{WriteOver, connector},
        },
    },
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;
use serde_json::{Value, json};

async fn started(seed: Fixture) -> holst_core::Result<(Fake, Client, String)> {
    let fake = Fake::start(seed).await?;
    let session = Session::with_cookies(
        SecretString::from(ACCEPTED_TOKEN),
        ACCEPTED_COOKIES.parse().unwrap_or_default(),
    );
    let client = connect(
        Some(session),
        fake.base_url().parse()?,
        SystemClock,
        RandomIds,
    )?;
    let board = lookup::board_id(&client, "Plans", None).await?;
    Ok((fake, client, board))
}

fn roadmap() -> Vec<FixtureObject> {
    vec![FixtureObject::FreeText("Roadmap".into())]
}

async fn change(
    client: &Client,
    board: &str,
    change: BoardChange,
) -> holst_core::Result<()> {
    let user = holst_core::command::account::account_id(client).await?;
    write_cached(
        client,
        &WriteOver::sender(connector()),
        WriteRequest {
            board,
            change: &change,
            send_if: ALWAYS,
            account_id: &user,
            kept: None,
        },
        None,
    )
    .await
    .map(|_| ())
    .map_err(Error::from)
}

/// What the next read shows: the id, kind, corner and text of each object.
async fn read(
    client: &Client,
    board: &str,
) -> holst_core::Result<Vec<(String, String, (f64, f64), String)>> {
    let loaded = load(client, board).await?;
    let mut found: Vec<_> = objects(&loaded.document)
        .into_iter()
        .map(|object| {
            (
                object.id,
                object.kind.as_str().to_owned(),
                (object.x, object.y),
                object.text,
            )
        })
        .collect();
    found.sort_by(|left, right| left.3.cmp(&right.3));
    Ok(found)
}

fn sticker_texts(shown: &[(String, String, (f64, f64), String)]) -> Vec<&str> {
    shown
        .iter()
        .filter(|(_, kind, _, _)| kind == "sticker")
        .map(|(_, _, _, text)| text.as_str())
        .collect()
}

#[tokio::test]
async fn a_sticker_is_added_moved_retexted_and_removed() -> holst_core::Result<()> {
    let (_fake, client, board) =
        started(Fixture::default().workspace("Team").board_with_objects(
            "Plans",
            "Team",
            roadmap(),
        ))
        .await?;

    change(
        &client,
        &board,
        BoardChange::AddAll(vec![NewEntry {
            new: NewObject::Sticker {
                text: "Buy milk".into(),
                at: Point::new(10.0, 20.0)?,
                size: None,
                color: None,
            },
            frame: None,
            format: holst_core::domain::rich_text::TextFormat::Plain,
        }]),
    )
    .await?;
    let shown = read(&client, &board).await?;
    assert_eq!(sticker_texts(&shown), ["Buy milk"]);
    let sticker = shown
        .iter()
        .find(|(_, kind, _, _)| kind == "sticker")
        .map(|(id, _, at, _)| (id.clone(), *at))
        .unwrap_or_default();
    assert_eq!(sticker.1, (10.0, 20.0));

    let patch = |text: Option<&str>, at: Option<Point>| {
        BoardChange::EditAll(vec![ObjectEdit {
            object: sticker.0.clone(),
            patch: ObjectPatch {
                text: text.map(str::to_owned),
                at,
                size: None,
                color: None,
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }])
    };
    change(&client, &board, patch(None, Some(Point::new(400.0, 50.0)?))).await?;
    let moved = read(&client, &board).await?;
    assert_eq!(
        moved
            .iter()
            .find(|(id, ..)| *id == sticker.0)
            .map(|(_, _, at, _)| *at),
        Some((400.0, 50.0))
    );

    change(&client, &board, patch(Some("Buy oat milk"), None)).await?;
    let retexted = read(&client, &board).await?;
    assert_eq!(sticker_texts(&retexted), ["Buy oat milk"]);

    change(
        &client,
        &board,
        BoardChange::RemoveAll(vec![sticker.0.clone()]),
    )
    .await?;
    let removed = read(&client, &board).await?;
    assert_eq!(
        removed
            .iter()
            .map(|(_, _, _, text)| text.as_str())
            .collect::<Vec<_>>(),
        ["Roadmap"]
    );
    Ok(())
}

fn seen(fake: &Fake, prefix: &str) -> usize {
    fake.requests()
        .iter()
        .filter(|request| request.starts_with(prefix))
        .count()
}

fn edited_object(board: &str, id: &str) -> holst_core::Result<EditObjectArgs> {
    table::parse(
        json!({ "board": board, "object": [id], "hide_content": true })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    )
}

fn board_edit(given: &Value) -> holst_core::Result<EditBoardArgs> {
    table::parse(given.as_object().cloned().unwrap_or_default())
}

async fn frame_of(client: &Client, board: &str) -> holst_core::Result<String> {
    Ok(read(client, board)
        .await?
        .into_iter()
        .find(|(_, kind, _, _)| kind == "frame")
        .map(|(id, ..)| id)
        .unwrap_or_default())
}

fn with_frame() -> Fixture {
    Fixture::default().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![FixtureObject::Frame("Q4".into())],
    )
}

#[tokio::test]
async fn hiding_a_frame_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    let frame = frame_of(&client, &board).await?;
    let (channels, sessions) = (
        seen(&fake, &format!("GET /gapi/channel/{board}")),
        seen(&fake, "WS open"),
    );
    object_change::edit(
        &client,
        &CacheSettings::default(),
        &edited_object(&board, &frame)?,
    )
    .await?;
    assert_eq!(
        (
            seen(&fake, &format!("GET /gapi/channel/{board}")) - channels,
            seen(&fake, "WS open") - sessions
        ),
        (1, 1)
    );
    Ok(())
}

#[tokio::test]
async fn a_withheld_hide_is_refused_with_nothing_sent() -> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    let frame = frame_of(&client, &board).await?;
    fake.withhold(
        "Plans",
        "who_can_manage_frame_hide_content",
        "co_owner",
        "editor",
    );
    let sessions = seen(&fake, "WS open");
    let done = object_change::edit(
        &client,
        &CacheSettings::default(),
        &edited_object(&board, &frame)?,
    )
    .await;
    assert_eq!(
        (
            matches!(
                done,
                Err(Error::BoardChange(BoardChangeError::HideWithheld { .. }))
            ),
            seen(&fake, "WS open") - sessions
        ),
        (true, 0)
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_channel_read_is_an_error_not_permission_to_remove()
-> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    let frame = frame_of(&client, &board).await?;
    fake.fail(&format!("channel/{board}"), 502);
    let sessions = seen(&fake, "WS Update");
    let done = change(&client, &board, BoardChange::RemoveAll(vec![frame])).await;
    assert_eq!(
        (done.is_err(), seen(&fake, "WS Update") - sessions),
        (true, 0)
    );
    Ok(())
}

#[tokio::test]
async fn a_note_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    let given = board_edit(&json!({ "board": board, "note": "hello" }))?;
    settings::edit(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(
        (
            seen(&fake, &format!("GET /gapi/channel/{board}")),
            seen(&fake, "WS open")
        ),
        (1, 1)
    );
    Ok(())
}

#[tokio::test]
async fn a_cover_reset_reads_the_board_once_before_the_upload() -> holst_core::Result<()>
{
    let (fake, client, board) = started(with_frame()).await?;
    let given = board_edit(&json!({ "board": board, "cover": "reset" }))?;
    drop(settings::edit(&client, &CacheSettings::default(), &given).await);
    assert_eq!(
        (
            seen(&fake, &format!("GET /gapi/channel/{board}")),
            seen(&fake, "WS open"),
            seen(&fake, "POST /gapi/upload_file")
        ),
        (1, 1, UPLOADS_WITH_PNG)
    );
    Ok(())
}

/// The preview is drawn and uploaded only in a build that draws PNG.
#[cfg(feature = "png")]
const UPLOADS_WITH_PNG: usize = 1;
#[cfg(not(feature = "png"))]
const UPLOADS_WITH_PNG: usize = 0;

#[cfg(feature = "png")]
#[tokio::test]
async fn a_cover_reset_on_a_read_only_board_is_refused_before_the_upload()
-> holst_core::Result<()> {
    let (fake, client, board) = started(read_only()).await?;
    let given = board_edit(&json!({ "board": board, "cover": "reset" }))?;
    let done = settings::edit(&client, &CacheSettings::default(), &given).await;
    assert_eq!(
        (
            matches!(done, Err(Error::BoardChange(BoardChangeError::ReadOnly))),
            seen(&fake, &format!("GET /gapi/channel/{board}")),
            seen(&fake, "WS open"),
            seen(&fake, "POST /gapi/upload_file")
        ),
        (true, 1, 1, 0)
    );
    Ok(())
}

#[tokio::test]
async fn a_note_on_a_read_only_board_is_refused_before_an_update()
-> holst_core::Result<()> {
    let (fake, client, board) = started(read_only()).await?;
    let given = board_edit(&json!({ "board": board, "note": "hello" }))?;
    let done = settings::edit(&client, &CacheSettings::default(), &given).await;
    assert_eq!(
        (
            matches!(done, Err(Error::BoardChange(BoardChangeError::ReadOnly))),
            seen(&fake, "WS Update")
        ),
        (true, 0)
    );
    Ok(())
}

#[tokio::test]
#[expect(clippy::disallowed_methods, reason = "the test removes its temp file")]
async fn a_cover_file_on_a_read_only_board_uploads_nothing() -> holst_core::Result<()> {
    let (fake, client, board) = started(read_only()).await?;
    let file = std::env::temp_dir()
        .join(format!("holst-write-cover-{}.png", std::process::id()));
    std::fs::write(&file, b"\x89PNG\r\n\x1a\n....")?;
    let given = board_edit(&json!({ "board": board, "cover": file }))?;
    let done = settings::edit(&client, &CacheSettings::default(), &given).await;
    drop(std::fs::remove_file(&file));
    assert_eq!(
        (
            matches!(done, Err(Error::BoardChange(BoardChangeError::ReadOnly))),
            seen(&fake, "POST /gapi/upload_file")
        ),
        (true, 0)
    );
    Ok(())
}

fn read_only() -> Fixture {
    Fixture::default()
        .workspace("Team")
        .read_only_board_with_objects("Plans", "Team", roadmap())
}

fn parsed<A: table::CommandArgs>(given: &Value) -> holst_core::Result<A> {
    table::parse(given.as_object().cloned().unwrap_or_default())
}

fn channel_reads(fake: &Fake, board: &str) -> usize {
    seen(fake, &format!("GET /gapi/channel/{board}"))
}

fn two_boards() -> Fixture {
    with_frame().board("Ideas", "Team", None)
}

#[tokio::test]
async fn adding_a_sticker_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    let given = parsed::<AddArgs>(&json!({ "board": board, "text": "Buy milk" }))?;
    object_add::add(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(channel_reads(&fake, &board), 1, "{:?}", fake.requests());
    Ok(())
}

#[tokio::test]
async fn adding_a_hidden_frame_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    let given = parsed::<AddArgs>(
        &json!({ "board": board, "kind": "frame", "text": "Q5", "hide_content": true }),
    )?;
    object_add::add(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(channel_reads(&fake, &board), 1, "{:?}", fake.requests());
    Ok(())
}

#[tokio::test]
async fn a_withheld_hide_is_refused_by_add_with_nothing_written()
-> holst_core::Result<()> {
    let (fake, client, board) = started(with_frame()).await?;
    fake.withhold(
        "Plans",
        "who_can_manage_frame_hide_content",
        "co_owner",
        "editor",
    );
    let given = parsed::<AddArgs>(
        &json!({ "board": board, "kind": "frame", "text": "Q5", "hide_content": true }),
    )?;
    let done = object_add::add(&client, &CacheSettings::default(), &given).await;
    assert_eq!(
        (
            matches!(
                done,
                Err(Error::BoardChange(BoardChangeError::HideWithheld { .. }))
            ),
            seen(&fake, "WS Update"),
            fake.uploads().len(),
            channel_reads(&fake, &board)
        ),
        (true, 0, 0, 1)
    );
    Ok(())
}

fn copy_of(
    board: &str,
    object: &str,
    into: Option<&str>,
) -> holst_core::Result<CopyArgs> {
    parsed(&json!({ "board": board, "object": [object], "into": into }))
}

#[tokio::test]
async fn copying_on_the_same_board_reads_it_once() -> holst_core::Result<()> {
    let (fake, client, board) = started(two_boards()).await?;
    let frame = frame_of(&client, &board).await?;
    let given = copy_of(&board, &frame, None)?;
    object_copy::cp(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(channel_reads(&fake, &board), 1, "{:?}", fake.requests());
    Ok(())
}

#[tokio::test]
async fn copying_into_another_board_reads_each_board_once() -> holst_core::Result<()> {
    let (fake, client, board) = started(two_boards()).await?;
    let frame = frame_of(&client, &board).await?;
    let ideas = fake.id_of("Ideas").unwrap_or_default();
    let given = copy_of(&board, &frame, Some("Ideas"))?;
    object_copy::cp(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(
        (channel_reads(&fake, &board), channel_reads(&fake, &ideas)),
        (1, 1),
        "{:?}",
        fake.requests()
    );
    Ok(())
}

#[tokio::test]
async fn a_withheld_copy_is_refused_with_nothing_written() -> holst_core::Result<()> {
    let (fake, client, board) = started(two_boards()).await?;
    let frame = frame_of(&client, &board).await?;
    fake.withhold("Plans", "who_can_copy_content", "co_owner", "editor");
    let given = copy_of(&board, &frame, Some("Ideas"))?;
    let done = object_copy::cp(&client, &CacheSettings::default(), &given).await;
    assert_eq!(
        (
            matches!(
                done,
                Err(Error::BoardChange(BoardChangeError::CopyWithheld { .. }))
            ),
            seen(&fake, "WS Update"),
            channel_reads(&fake, &board)
        ),
        (true, 0, 1)
    );
    Ok(())
}
