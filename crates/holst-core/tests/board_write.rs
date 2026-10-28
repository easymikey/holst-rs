//! Changing the objects of a board against the fake Holst server.

use holst_core::{
    command::{
        board_write::{WriteRequest, write_cached},
        login::connect,
        lookup,
    },
    domain::{
        board::objects,
        change::value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
        geometry::Point,
        session::Session,
    },
    effect::{board_write::WriteMode, confirmation::ALWAYS},
    error::Error,
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
    remote::{
        client::Client,
        socket::{load::load, write::interpreter},
    },
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;

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
        interpreter(WriteMode::Send),
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
                format: holst_core::domain::rich_text::TextFormat::Plain,
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
