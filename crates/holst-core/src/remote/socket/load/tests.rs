#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests measure and wait on real time"
)]

use std::{sync::PoisonError, time::Duration};

use serde_json::json;
use yrs::{
    Any,
    Doc,
    Map as _,
    ReadTxn as _,
    StateVector,
    Transact as _,
    updates::encoder::Encode as _,
};

use crate::{
    domain::{
        board::{EditRight, ObjectKind},
        id_source::SequentialIds,
    },
    effect::{
        board_write::LoadSource,
        http::BoxFuture,
        socket::{BoardSocket, SocketConnector, SocketSession},
    },
    error::{
        Error,
        Result,
        board_socket::{BoardSocketError, SocketErrorCode},
    },
    protocol::{
        hud,
        hud::{
            DocumentResponse,
            ServerUpdate as HudUpdate,
            SocketFrame,
            SocketMessage as HudMessage,
            UpdateEncoding,
            UpdatePayload,
        },
    },
    remote::{
        client::fake,
        socket::{
            fake::{
                BOARD,
                Scripted,
                answer,
                ids_of,
                linked_board,
                on_board,
                script,
                sticker_by,
                update,
            },
            load::{load_seeded, load_with},
            write::open_with,
        },
    },
};

fn update_of(
    document: &str,
    left: i64,
    bytes: Vec<u8>,
    server: &StateVector,
) -> SocketFrame {
    SocketFrame {
        document: document.to_owned(),
        message: HudMessage::Update(HudUpdate {
            encoding: UpdateEncoding::V2,
            server_time: 1,
            left,
            id: 7,
            state_vector: server.encode_v1(),
            payload: UpdatePayload::Inline(bytes),
        }),
    }
}

fn joined(mut left: StateVector, right: StateVector) -> StateVector {
    left.merge(right);
    left
}

fn raised(mut vector: StateVector, client: yrs::ClientID, clock: u32) -> StateVector {
    vector.set_max(client, clock);
    vector
}

fn refusal_reason<T>(loaded: Result<T>) -> Option<SocketErrorCode> {
    match loaded {
        Err(Error::BoardSocket(BoardSocketError::NotSubscribed { code: refusal })) => {
            Some(refusal)
        }
        _ => None,
    }
}

fn left_in(socket: &Scripted) -> usize {
    socket
        .0
        .incoming
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len()
}

#[tokio::test]
async fn only_the_boards_own_updates_load_it() -> Result<()> {
    let (own, _) = sticker_by(1, "own");
    let (theirs, _) = sticker_by(2, "theirs");
    let (shared, _) = sticker_by(3, "shared");
    let none = StateVector::default();
    let socket = script([
        Some(vec![answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        })]),
        Some(vec![
            update_of(&format!("{BOARD}$state"), 0, theirs, &none),
            update_of(hud::ANY_DOCUMENT, 0, shared, &none),
        ]),
        Some(vec![update_of(BOARD, 0, own, &none)]),
        Some(vec![update(0, UpdatePayload::Inline(Vec::new()))]),
    ]);
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let loaded = load_with(&client, &socket, BOARD).await?;
    assert_eq!(ids_of(&loaded), ["own"]);
    assert_eq!(left_in(&socket), 1, "ends on the board's own last update");
    Ok(())
}

#[tokio::test]
async fn a_live_update_mid_load_does_not_end_it() -> Result<()> {
    let (first, first_sv) = sticker_by(1, "first");
    let (second, second_sv) = sticker_by(2, "second");
    let (live, live_sv) = sticker_by(3, "live");
    let server = joined(first_sv, second_sv);
    let after_live = joined(server.clone(), live_sv);
    let socket = script([
        Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update_of(BOARD, 1, first, &server),
        ]),
        Some(vec![update_of(BOARD, 0, live, &after_live)]),
        Some(vec![update_of(BOARD, 0, second, &server)]),
    ]);
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let loaded = load_with(&client, &socket, BOARD).await?;
    assert_eq!(ids_of(&loaded), ["first", "live", "second"]);
    Ok(())
}

#[tokio::test]
async fn a_document_with_gaps_is_refused() -> Result<()> {
    let doc = Doc::with_client_id(5);
    let objects = doc.get_or_insert_map("objects");
    objects.insert(&mut doc.transact_mut(), "a", 1.0);
    let seen = doc.transact().state_vector();
    objects.insert(&mut doc.transact_mut(), "b", 2.0);
    let later = doc.transact().encode_state_as_update_v2(&seen);
    let socket = script([Some(vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        update(0, UpdatePayload::Inline(later)),
    ])]);
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    assert!(matches!(
        load_with(&client, &socket, BOARD).await,
        Err(Error::BoardSocket(BoardSocketError::Incomplete))
    ));
    Ok(())
}

#[tokio::test]
async fn an_unknown_frame_of_another_document_is_skipped() -> Result<()> {
    let unknown = |document: &str| SocketFrame {
        document: document.to_owned(),
        message: HudMessage::Unknown {
            kind: 9,
            dropped: 3,
        },
    };
    let load = |document: &str| {
        script([Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update(0, UpdatePayload::Inline(Vec::new())),
            unknown(document),
        ])])
    };
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let other = format!("{BOARD}$comments");
    assert_eq!(
        load_with(&client, &load(&other), BOARD).await?.right,
        EditRight::Editable
    );
    assert!(matches!(
        load_with(&client, &load(BOARD), BOARD).await,
        Err(Error::BoardSocket(BoardSocketError::MalformedFrame { .. }))
    ));
    Ok(())
}

#[tokio::test]
async fn a_taken_client_id_is_redrawn() -> Result<()> {
    let taken = crate::domain::change::build::client_id("c-1", |_| false);
    let (board, _) = sticker_by(taken, "theirs");
    let socket = script([Some(vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        update(0, UpdatePayload::Inline(board)),
    ])]);
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let client = client.with_ids(SequentialIds::new("c"));
    let synced = open_with(&client, &socket, BOARD).await?;
    let mine = synced.document().client_id().get();
    assert_ne!(mine, taken);
    assert!(mine < 1 << 31);
    assert_eq!(crate::domain::board::objects(synced.document()).len(), 1);
    Ok(())
}

fn sticker_updates() -> (Vec<u8>, Vec<u8>) {
    let doc = Doc::new();
    let objects = doc.get_or_insert_map("objects");
    let before = doc
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    let seen = doc.transact().state_vector();
    let fields =
        json!({"id": "s1", "type": "sticker", "position": {"x": 100, "y": 100}});
    let sticker = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
    objects.insert(&mut doc.transact_mut(), "s1", sticker);
    let after = doc.transact().encode_state_as_update_v2(&seen);
    (before, after)
}

#[tokio::test]
async fn load_reads_until_the_last_update_and_sends_only_the_request() -> Result<()> {
    let (empty, sticker) = sticker_updates();
    let link = "https://cdns-edge.app.holst.so/snapshots/1";
    let foreign = SocketFrame {
        document: "other".into(),
        ..update(0, UpdatePayload::Inline(Vec::new()))
    };
    let socket = script([
        Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update(1, UpdatePayload::Inline(empty)),
        ]),
        Some(vec![foreign, update(0, UpdatePayload::Link(link.into()))]),
        Some(vec![update(0, UpdatePayload::Inline(vec![0xff]))]),
    ]);
    let reply = fake::reply(200, &[], "");
    let (http, client) = fake::on_fake(
        [crate::effect::http::HttpResponse {
            body: sticker,
            ..reply
        }],
        Some(fake::stored("tok", "s=1")),
    )?;
    let loaded = load_with(&client, &socket, BOARD).await?;
    let objects = crate::domain::board::objects(&loaded.document);
    assert_eq!(loaded.right, EditRight::Editable);
    assert_eq!(
        objects
            .iter()
            .map(|o| (o.kind.clone(), o.x))
            .collect::<Vec<_>>(),
        [(ObjectKind::Sticker, 100.0)]
    );
    let sent = socket
        .0
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        sent,
        [hud::request_document(BOARD, "tok")],
        "nothing but the request"
    );
    assert_eq!(left_in(&socket), 1, "stops at the last update of the load");
    let fetched = http.requests();
    assert!(
        fetched
            .iter()
            .all(|r| r.url.as_str() == link && r.headers.is_empty()),
        "links go out without the login: {fetched:?}"
    );
    Ok(())
}

#[tokio::test]
async fn refusal_and_close_are_errors() -> Result<()> {
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "")))?;
    let refused = script([Some(vec![answer(DocumentResponse::NotSubscribed {
        code: SocketErrorCode::Forbidden,
    })])]);
    let closed = script([
        Some(vec![update(3, UpdatePayload::Inline(Vec::new()))]),
        None,
    ]);
    assert_eq!(
        refusal_reason(load_with(&client, &refused, BOARD).await),
        Some(SocketErrorCode::Forbidden)
    );
    assert!(matches!(
        load_with(&client, &closed, BOARD).await,
        Err(Error::BoardSocket(BoardSocketError::Closed))
    ));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_silent_server_times_out() -> Result<()> {
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "")))?;
    let silent = script([]);
    assert!(matches!(
        load_with(&client, &silent, BOARD).await,
        Err(Error::BoardSocket(BoardSocketError::Timeout {
            seconds: 30
        }))
    ));
    Ok(())
}

/// A server that keeps `gap` between its frames.
struct Slow {
    inner: Scripted,
    gap: Duration,
}

impl BoardSocket for Slow {
    fn send(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        self.inner.send(bytes)
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(async move {
            tokio::time::sleep(self.gap).await;
            self.inner.receive().await
        })
    }
}

impl SocketConnector for Slow {
    fn open<'a>(
        &'a self,
        _: &'a SocketSession,
    ) -> BoxFuture<'a, Result<Box<dyn BoardSocket>>> {
        Box::pin(async move {
            let inner = Scripted(std::sync::Arc::clone(&self.inner.0));
            let socket: Box<dyn BoardSocket> = Box::new(Slow {
                inner,
                gap: self.gap,
            });
            Ok(socket)
        })
    }
}

fn frames_of_a_long_load(count: usize) -> Scripted {
    script(
        [Some(vec![answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        })])]
        .into_iter()
        .chain(
            (1..count)
                .map(|_| Some(vec![update(1, UpdatePayload::Inline(Vec::new()))])),
        )
        .chain([Some(vec![update(0, UpdatePayload::Inline(Vec::new()))])]),
    )
}

#[tokio::test(start_paused = true)]
async fn a_slow_but_steady_server_does_not_time_out() -> Result<()> {
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "")))?;
    let slow = Slow {
        inner: frames_of_a_long_load(5),
        gap: Duration::from_secs(20),
    };
    let started = tokio::time::Instant::now();
    let loaded = load_with(&client, &slow, BOARD).await?;
    assert!(
        started.elapsed() > super::IDLE_TIMEOUT,
        "the load outlasts one idle period"
    );
    assert_eq!(loaded.right, EditRight::Editable);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_server_that_goes_silent_mid_load_times_out() -> Result<()> {
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "")))?;
    // Two frames, then nothing: the third wait is silent.
    let slow = Slow {
        inner: script([
            Some(vec![answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            })]),
            Some(vec![update(1, UpdatePayload::Inline(Vec::new()))]),
        ]),
        gap: Duration::ZERO,
    };
    assert!(matches!(
        load_with(&client, &slow, BOARD).await,
        Err(Error::BoardSocket(BoardSocketError::Timeout {
            seconds: 30
        }))
    ));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_load_that_never_ends_hits_the_total_cap() -> Result<()> {
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "")))?;
    let endless = Slow {
        inner: frames_of_a_long_load(100),
        gap: Duration::from_secs(20),
    };
    assert!(matches!(
        load_with(&client, &endless, BOARD).await,
        Err(Error::BoardSocket(BoardSocketError::TotalTimeout {
            seconds: 600
        }))
    ));
    Ok(())
}

#[tokio::test]
async fn a_covered_link_is_not_downloaded() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let seed = crate::domain::board_file::document::decoded(&state)?;
    let socket = linked_board(&server);
    // No reply scripted: a download would fail the load.
    let (http, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let (loaded, fetched) = load_seeded(&client, &socket, BOARD, seed).await?;
    assert_eq!(fetched, LoadSource::Skipped);
    assert!(http.requests().is_empty(), "{:?}", http.requests());
    assert_eq!(ids_of(&loaded), ["cached"]);
    assert_eq!(loaded.right, EditRight::Editable);
    Ok(())
}

#[tokio::test]
async fn a_link_with_news_is_downloaded() -> Result<()> {
    let (old, _) = sticker_by(4, "old");
    let (news, news_sv) = sticker_by(5, "new");
    let (_, old_sv) = sticker_by(4, "old");
    let server = joined(old_sv, news_sv);
    let seed = crate::domain::board_file::document::decoded(&old)?;
    let reply = fake::reply(200, &[], "");
    let body = crate::effect::http::HttpResponse {
        body: news.clone(),
        ..reply
    };
    let (http, client) = fake::on_fake([body], Some(fake::stored("tok", "s=1")))?;
    let (loaded, fetched) =
        load_seeded(&client, &linked_board(&server), BOARD, seed).await?;
    let bytes = u64::try_from(news.len()).unwrap_or_default();
    assert_eq!(fetched, LoadSource::Downloaded { bytes });
    assert_eq!(http.requests().len(), 1);
    assert_eq!(ids_of(&loaded), ["new", "old"]);
    Ok(())
}

#[tokio::test]
async fn a_seed_short_of_the_server_keeps_reading() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let seed = crate::domain::board_file::document::decoded(&state)?;
    let ahead = raised(server.clone(), yrs::ClientID::new(6), 3);
    let socket = script([
        Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update_of(BOARD, 0, Vec::new(), &ahead),
        ]),
        None,
    ]);
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let loaded = load_seeded(&client, &socket, BOARD, seed).await;
    assert!(
        matches!(loaded, Err(Error::BoardSocket(BoardSocketError::Closed))),
        "{loaded:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_update_frame_without_a_state_vector_still_carries_yjs_v2() -> Result<()> {
    let (own, _) = sticker_by(1, "own");
    let socket = script([
        Some(vec![answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        })]),
        Some(vec![on_board(HudMessage::Update(HudUpdate {
            encoding: UpdateEncoding::V1,
            server_time: 1,
            left: 0,
            id: 7,
            state_vector: Vec::new(),
            payload: UpdatePayload::Inline(own),
        }))]),
    ]);
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let loaded = load_with(&client, &socket, BOARD).await?;
    assert_eq!(ids_of(&loaded), ["own"]);
    Ok(())
}
