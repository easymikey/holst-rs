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
    fake::{
        ACCEPTED_COOKIES,
        ACCEPTED_TOKEN,
        Fake,
        Fixture,
        FixtureThread,
        ThreadAnchor,
    },
    remote::{client::Client, socket::comments::open},
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
    assert_eq!(fake.threads_of("Plans"), Vec::<FixtureThread>::new());
    assert_eq!(threads(&client, &board).await?, Vec::<Thread>::new());
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
    use std::fs;

    use holst_core::{
        command::{
            account::account_id,
            change::ChangeReport,
            comment::{AddArgs, CommentReport, LsArgs, ReplyArgs},
            table::{CommandArgs, parse},
        },
        effect::{cache::CachePort as _, settings::CacheSettings},
        error::local_data::LocalDataError,
        fake::{Fixture, FixtureObject, FixtureThread, ThreadAnchor},
        storage::cache::Cache,
        system::clock::SystemClock,
    };
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
        holst_core::command::comment::add(
            client,
            &CacheSettings::default(),
            &args::<AddArgs>(&given)?,
        )
        .await
    }

    fn rect_and_resolved() -> Fixture {
        seeded()
            .thread(
                "Plans",
                FixtureThread::new("RcT01", 13, "Here").in_area(10.0, 50.0, 20.0, 30.0),
            )
            .thread("Plans", FixtureThread::new("ReS01", 11, "Fixed").resolved())
    }

    #[tokio::test]
    async fn a_thread_pinned_to_a_rectangle_is_listed_with_it() -> holst_core::Result<()>
    {
        let (_fake, client, _) = started(rect_and_resolved()).await?;
        let listed = holst_core::command::comment::ls(
            &client,
            &args::<LsArgs>(&json!({ "board": "Plans" }))?,
        )
        .await?;
        assert_eq!(listed.threads.len(), 4);
        let line = listed
            .lines()
            .into_iter()
            .find(|line| line.contains("RcT01"))
            .unwrap_or_default();
        assert!(line.contains("area 10,50 20×30"), "{line}");
        let value = serde_json::to_value(&listed)?;
        assert_eq!(
            value.pointer("/threads/2/anchor"),
            Some(
                &json!({ "area": { "x": 10.0, "y": 50.0, "width": 20.0, "height": 30.0 } })
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_reply_opens_a_resolved_thread_again() -> holst_core::Result<()> {
        let (fake, client, _) = started(rect_and_resolved()).await?;
        let replied = holst_core::command::comment::reply(
            &client,
            &args::<ReplyArgs>(
                &json!({ "board": "Plans", "thread": "ReS01", "text": "Not yet" }),
            )?,
        )
        .await?;
        assert!(sent(&replied));
        let held = fake.threads_of("Plans");
        let thread = held.iter().find(|thread| thread.id == "ReS01");
        assert_eq!(thread.map(|thread| thread.resolved), Some(false));
        assert_eq!(thread.map(|thread| thread.messages.len()), Some(2));
        Ok(())
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test removes its own temp directory"
    )]
    async fn a_thread_on_an_object_keeps_the_board_it_loaded_in_the_cache()
    -> holst_core::Result<()> {
        let (fake, client, _) = started(seeded()).await?;
        let root = std::env::temp_dir()
            .join(format!("holst-comment-cache-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let settings = CacheSettings {
            root: Some(root.clone()),
            roots: vec![root.clone()],
            host: "app.holst.so".into(),
            max_bytes: 1 << 24,
            trust_millis: 1_000,
        };
        let object = fake.object_id("Plans", 1).unwrap_or_default();
        let given = json!({ "board": "Plans", "text": "Here?", "object": object });
        let started = holst_core::command::comment::add(
            &client,
            &settings,
            &args::<AddArgs>(&given)?,
        )
        .await?;
        let user = account_id(&client).await?;
        let cache = Cache::from_settings(&settings, &user, SystemClock)
            .ok_or(holst_core::Error::LocalData(LocalDataError::NoHome))?;
        let kept = cache.usage().boards;
        fs::remove_dir_all(&root)?;
        assert!(sent(&started));
        assert_eq!(kept, 1);
        Ok(())
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
