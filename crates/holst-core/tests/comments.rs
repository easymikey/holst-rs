//! Comment threads over the socket against the fake Holst server.

use holst_core::{
    command::{account::account_id, login::connect, lookup},
    domain::{
        board::comment::{
            CommentChange,
            NewThreadMessage,
            Thread,
            ThreadAnchor as CoreAnchor,
        },
        change::build::author_number,
        session::Session,
    },
    remote::{client::Client, socket::comments::open},
    system::{clock::SystemClock, id::RandomIds},
};
use holst_fake::{
    ACCEPTED_COOKIES,
    ACCEPTED_TOKEN,
    Fake,
    Fixture,
    FixtureThread,
    ThreadAnchor,
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

fn plans() -> Fixture {
    Fixture::default()
        .workspace("Team")
        .board("Plans", "Team", None)
}

async fn threads(client: &Client, board: &str) -> holst_core::Result<Vec<Thread>> {
    open(client, board).await.map(|room| room.threads())
}

async fn send(
    client: &Client,
    board: &str,
    change: CommentChange,
) -> holst_core::Result<()> {
    open(client, board).await?.send(client, &change).await
}

fn message(id: &str, author: &str, text: &str) -> NewThreadMessage {
    NewThreadMessage {
        id: id.to_owned(),
        author: author.to_owned(),
        created_at: 1_793_100_000_000.5,
        text: text.to_owned(),
    }
}

#[tokio::test]
async fn seeded_threads_are_read_with_their_replies() -> holst_core::Result<()> {
    let (_fake, client, board) = started(
        plans()
            .thread(
                "Plans",
                FixtureThread::new("9YSRw", 11, "Why?")
                    .at(10.0, 20.0)
                    .colored("red10")
                    .reply(12, "Because"),
            )
            .thread(
                "Plans",
                FixtureThread::new("ZTaaJ", 12, "Done")
                    .on_object(2)
                    .resolved(),
            ),
    )
    .await?;
    let found = threads(&client, &board).await?;
    assert_eq!(
        found
            .iter()
            .map(|thread| {
                (
                    thread.id.as_str(),
                    thread.color.as_deref(),
                    thread.resolved,
                    thread
                        .messages
                        .iter()
                        .map(|message| (message.author, message.text.as_str()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
        [
            (
                "9YSRw",
                Some("red10"),
                false,
                vec![(11, "Why?"), (12, "Because")]
            ),
            ("ZTaaJ", None, true, vec![(12, "Done")]),
        ]
    );
    assert_eq!(
        found.first().map(|thread| thread.anchor.clone()),
        Some(CoreAnchor::Point { x: 10.0, y: 20.0 })
    );
    assert!(matches!(
        found.get(1).map(|thread| &thread.anchor),
        Some(CoreAnchor::Object { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn every_change_reaches_the_server() -> holst_core::Result<()> {
    let (fake, client, board) = started(plans()).await?;
    let user = account_id(&client).await?;
    let me = author_number(&user, 0);
    send(
        &client,
        &board,
        CommentChange::Start {
            message: message("B9ckv", &user, "zz first\nsecond line"),
            anchor: CoreAnchor::Point {
                x: 169.5,
                y: 698.25,
            },
        },
    )
    .await?;
    send(
        &client,
        &board,
        CommentChange::Reply {
            thread: "B9ckv".to_owned(),
            message: message("3fBIc", &user, "zz reply"),
        },
    )
    .await?;
    send(
        &client,
        &board,
        CommentChange::Color {
            thread: "B9ckv".to_owned(),
            color: Some("indigo6".to_owned()),
        },
    )
    .await?;
    send(
        &client,
        &board,
        CommentChange::Resolve {
            thread: "B9ckv".to_owned(),
            by: user.clone(),
            at: 1_793_100_001_000.5,
        },
    )
    .await?;
    let seen = fake.threads_of("Plans");
    let thread = seen.first();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        thread.map(|thread| thread.anchor.clone()),
        Some(ThreadAnchor::Point(169.5, 698.25))
    );
    assert_eq!(
        thread.map(|thread| (thread.color.clone(), thread.resolved)),
        Some((Some("indigo6".to_owned()), true))
    );
    assert_eq!(
        thread.map(|thread| {
            thread
                .messages
                .iter()
                .map(|message| (message.author, message.text.clone()))
                .collect::<Vec<_>>()
        }),
        Some(vec![
            (me, "zz first\nsecond line".to_owned()),
            (me, "zz reply".to_owned())
        ])
    );
    assert_eq!(fake.updates_received(), 4);
    send(
        &client,
        &board,
        CommentChange::Reopen {
            thread: "B9ckv".to_owned(),
        },
    )
    .await?;
    assert_eq!(
        fake.threads_of("Plans")
            .first()
            .map(|reopened| reopened.resolved),
        Some(false)
    );
    assert_eq!(threads(&client, &board).await?.len(), 1);
    send(
        &client,
        &board,
        CommentChange::Delete {
            thread: "B9ckv".to_owned(),
        },
    )
    .await?;
    assert!(fake.threads_of("Plans").is_empty());
    assert!(threads(&client, &board).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_change_to_an_unknown_thread_sends_nothing() -> holst_core::Result<()> {
    let (fake, client, board) = started(plans()).await?;
    let outcome = send(
        &client,
        &board,
        CommentChange::Reopen {
            thread: "nope1".to_owned(),
        },
    )
    .await;
    assert!(outcome.is_err());
    assert_eq!(fake.updates_received(), 0);
    Ok(())
}

mod verbs {
    use holst_core::command::{
        change::ChangeReport,
        comment::{AddArgs, CommentReport},
        table::{CommandArgs, parse},
    };
    use holst_fake::{Fixture, FixtureObject, FixtureThread, ThreadAnchor};
    use serde_json::{Value, json};

    use super::{Client, started};

    fn args<A: CommandArgs>(given: &Value) -> holst_core::Result<A> {
        parse(given.as_object().cloned().unwrap_or_default())
    }

    fn seeded() -> Fixture {
        Fixture::default()
            .workspace("Team")
            .board_with_objects(
                "Plans",
                "Team",
                vec![FixtureObject::Sticker("Idea".into())],
            )
            .thread(
                "Plans",
                FixtureThread::new("9YSRw", 11, "Why?")
                    .reply(12, "Because")
                    .read_at("2026-10-27T12:27:28.401Z"),
            )
            .thread(
                "Plans",
                FixtureThread::new("ZTaaJ", 12, "Done").at(40.0, 40.0),
            )
    }

    async fn add(client: &Client, given: Value) -> holst_core::Result<CommentReport> {
        holst_core::command::comment::add(client, &args::<AddArgs>(&given)?).await
    }

    fn sent(changed: &CommentReport) -> bool {
        matches!(changed.outcome, ChangeReport::Sent)
    }

    #[tokio::test]
    async fn a_thread_starts_in_a_free_corner() -> holst_core::Result<()> {
        let (fake, client, _) = started(seeded()).await?;
        let free = add(&client, json!({ "board": "Plans", "text": "Hmm" })).await?;
        assert!(sent(&free));
        let threads = fake.threads_of("Plans");
        let anchor = threads
            .iter()
            .find(|thread| Some(&thread.id) == free.message.as_ref())
            .map(|thread| thread.anchor.clone());
        // (0,0) and (40,40) are held by the seeded threads.
        assert_eq!(anchor, Some(ThreadAnchor::Point(80.0, 80.0)));
        assert_eq!(threads.len(), 3);
        Ok(())
    }
}
