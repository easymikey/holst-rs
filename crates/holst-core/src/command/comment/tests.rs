#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};

use crate::{
    command::{
        change::ChangeReport,
        comment::{
            AddArgs,
            CommentAction,
            CommentPlan,
            CommentReport,
            ListedThread,
            Placement,
            ThreadList,
            found,
            free_corner,
            millis,
            placement,
        },
        table::parse,
    },
    domain::board::comment::{Thread, ThreadAnchor, ThreadMessage},
    effect::board_write::WriteMode,
};

fn thread(id: &str, anchor: ThreadAnchor) -> Thread {
    Thread {
        id: id.to_owned(),
        anchor,
        color: None,
        resolved: false,
        messages: vec![ThreadMessage {
            id: id.to_owned(),
            author: 1,
            created_at: 1.0,
            text: "Why?".to_owned(),
        }],
    }
}

fn add(given: &Value) -> crate::Result<AddArgs> {
    parse(given.as_object().cloned().unwrap_or_default())
}

fn placed(args: &AddArgs) -> crate::Result<Placement<'_>> {
    placement("comment add", args.at.as_deref(), args.object.as_deref())
}

fn changed(action: CommentAction, outcome: ChangeReport<CommentPlan>) -> CommentReport {
    CommentReport {
        action,
        board: "b".to_owned(),
        thread: "weVza".to_owned(),
        message: Some("m1".to_owned()),
        read_at: None,
        edited: vec!["color", "place"],
        outcome,
    }
}

#[test]
fn the_json_carries_the_status_at_the_top() -> serde_json::Result<()> {
    let dry = ChangeReport::DryRun {
        request: CommentPlan {
            action: CommentAction::Delete,
            thread: "weVza".to_owned(),
        },
    };
    let json = serde_json::to_value(changed(CommentAction::Delete, dry))?;
    assert_eq!(
        (json.pointer("/status"), json.pointer("/outcome")),
        (Some(&json!("dry_run")), None)
    );
    Ok(())
}

#[rstest]
#[case::add(CommentAction::Add, "started thread weVza", "would start thread weVza")]
#[case::reply(
    CommentAction::Reply,
    "replied in thread weVza",
    "would reply in thread weVza"
)]
#[case::resolve(
    CommentAction::Resolve,
    "resolved thread weVza",
    "would resolve thread weVza"
)]
#[case::reopen(
    CommentAction::Reopen,
    "reopened thread weVza",
    "would reopen thread weVza"
)]
#[case::delete(
    CommentAction::Delete,
    "deleted thread weVza",
    "would delete thread weVza"
)]
#[case::delete_reply(
    CommentAction::DeleteReply,
    "deleted reply m1 in thread weVza",
    "would delete reply m1 in thread weVza"
)]
#[case::edit(
    CommentAction::Edit,
    "changed thread weVza: color, place",
    "would change thread weVza: color, place"
)]
#[case::read(
    CommentAction::Read,
    "marked thread weVza read",
    "would mark thread weVza read"
)]
fn a_change_is_phrased_as_done_or_as_planned(
    #[case] action: CommentAction,
    #[case] done: &str,
    #[case] planned: &str,
) {
    assert_eq!(changed(action, ChangeReport::Sent).phrase(), done);
    assert_eq!(
        changed(
            action,
            ChangeReport::DryRun {
                request: CommentPlan {
                    action,
                    thread: "weVza".to_owned(),
                }
            }
        )
        .phrase(),
        planned
    );
}

#[test]
fn a_thread_line_shows_number_state_start_replies_and_read_mark() {
    let mut long = thread("B9ckv", ThreadAnchor::Point { x: 0.0, y: 0.0 });
    long.resolved = true;
    long.messages = vec![
        ThreadMessage {
            id: "B9ckv".to_owned(),
            author: 1,
            created_at: 1.0,
            text: "x".repeat(70),
        };
        2
    ];
    let list = ThreadList {
        board: "b".to_owned(),
        threads: vec![
            ListedThread {
                thread: thread("weVza", ThreadAnchor::Point { x: 0.0, y: 0.0 }),
                read_at: None,
            },
            ListedThread {
                thread: thread(
                    "Zz9ab",
                    ThreadAnchor::Area {
                        x: 10.0,
                        y: 50.5,
                        width: 20.0,
                        height: 30.0,
                    },
                ),
                read_at: None,
            },
            ListedThread {
                thread: long,
                read_at: Some("2026-10-27T09:30:00Z".to_owned()),
            },
        ],
    };
    assert_eq!(
        list.lines(),
        [
            "@1  weVza  open  Why?  (no replies, unread)".to_owned(),
            "@2  Zz9ab  open  area 10,50.5 20×30  Why?  (no replies, unread)"
                .to_owned(),
            format!(
                "@3  B9ckv  resolved  {}…  (1 reply, read 2026-10-27T09:30:00Z)",
                "x".repeat(60)
            ),
        ]
    );
}

#[test]
fn at_and_object_are_not_both_taken_and_at_is_two_numbers() -> crate::Result<()> {
    let both = add(&json!({ "board": "b", "text": "t", "at": "1,2", "object": "o" }))?;
    assert!(placed(&both).is_err());
    let bad = add(&json!({ "board": "b", "text": "t", "at": "1;2" }))?;
    assert!(placed(&bad).is_err());
    let good = add(&json!({ "board": "b", "text": "t", "at": " 120, 80.5 " }))?;
    assert_eq!(placed(&good)?, Placement::Point(120.0, 80.5));
    assert_eq!(good.dry_run, WriteMode::Send);
    Ok(())
}

#[test]
fn a_thread_is_found_by_id_or_by_number() -> crate::Result<()> {
    let threads = [
        thread("aaaaa", ThreadAnchor::Point { x: 0.0, y: 0.0 }),
        thread("bbbbb", ThreadAnchor::Point { x: 1.0, y: 1.0 }),
    ];
    assert_eq!(found(&threads, "bbbbb")?.id, "bbbbb");
    assert_eq!(found(&threads, "@1")?.id, "aaaaa");
    let misses: Vec<_> = ["@0", "@3", "zzzzz", "@"]
        .into_iter()
        .map(|missing| found(&threads, missing).err().is_some())
        .collect();
    assert_eq!(misses, [true; 4]);
    Ok(())
}

#[test]
fn a_free_corner_steps_along_the_diagonal() {
    let held = [
        thread("aaaaa", ThreadAnchor::Point { x: 0.0, y: 0.0 }),
        thread("bbbbb", ThreadAnchor::Point { x: 40.0, y: 40.0 }),
    ];
    assert_eq!(free_corner(&[]), ThreadAnchor::Point { x: 0.0, y: 0.0 });
    assert_eq!(free_corner(&held), ThreadAnchor::Point { x: 80.0, y: 80.0 });
}

#[test]
fn milliseconds_keep_their_fraction_digits() {
    assert!((millis(1_793_105_593_418) - 1_793_105_593_418.0).abs() < f64::EPSILON);
}

mod object_references {
    use secrecy::SecretString;
    use serde_json::json;

    use super::add;
    use crate::{
        command::login::connect,
        domain::session::Session,
        effect::settings::CacheSettings,
        error::{
            Error,
            lookup::{LookupError, LookupKind, LookupMiss},
        },
        fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
        system::{clock::SystemClock, id::RandomIds},
    };

    async fn refused(reference: &str) -> crate::Result<(Error, usize, usize)> {
        let sticker = |text: &str| FixtureObject::Sticker(text.into());
        let fake =
            Fake::start(Fixture::default().workspace("Team").board_with_objects(
                "Plans",
                "Team",
                vec![sticker("One"), sticker("Two")],
            ))
            .await?;
        let client = connect(
            Some(Session::with_cookies(
                SecretString::from(ACCEPTED_TOKEN),
                ACCEPTED_COOKIES.parse().unwrap_or_default(),
            )),
            fake.base_url().parse()?,
            SystemClock,
            RandomIds,
        )?;
        let args = add(&json!({
            "board": "Plans", "text": "Here?", "object": reference
        }))?;
        let refusal = super::super::add(&client, &CacheSettings::default(), &args)
            .await
            .err();
        Ok((
            refusal.ok_or(crate::error::input::InputError::NoWorkspaces)?,
            fake.updates_received(),
            fake.threads_of("Plans").len(),
        ))
    }

    #[tokio::test]
    async fn a_reference_that_matches_no_object_starts_no_thread() -> crate::Result<()>
    {
        let (refusal, updates, threads) = refused("deadbeef").await?;
        assert_eq!(
            lookup_of(&refusal).map(|lookup| (
                lookup.kind,
                lookup.reference.as_str(),
                &lookup.miss
            )),
            Some((LookupKind::Object, "deadbeef", &LookupMiss::not_found())),
            "{refusal:?}"
        );
        assert_eq!((updates, threads), (0, 0));
        Ok(())
    }

    #[tokio::test]
    async fn a_reference_that_matches_several_objects_lists_them() -> crate::Result<()>
    {
        let (refusal, updates, threads) = refused("20000000").await?;
        assert_eq!(
            lookup_of(&refusal).map(|lookup| (lookup.kind, listed(&lookup.miss))),
            Some((LookupKind::Object, 2)),
            "{refusal:?}"
        );
        assert_eq!((updates, threads), (0, 0));
        Ok(())
    }

    /// Returns the lookup an error tells of, when it is one.
    fn lookup_of(error: &Error) -> Option<&LookupError> {
        if let Error::Lookup(lookup) = error {
            Some(lookup)
        } else {
            None
        }
    }

    /// Returns how many candidates an ambiguous lookup lists, else 0.
    fn listed(miss: &LookupMiss) -> usize {
        if let LookupMiss::Ambiguous { candidates, .. } = miss {
            candidates.listed.len()
        } else {
            0
        }
    }
}
