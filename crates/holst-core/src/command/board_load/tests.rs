#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::sync::{Arc, Mutex, PoisonError};

use itertools::Itertools as _;
use serde_json::json;
use yrs::{
    Any,
    Doc,
    Map as _,
    ReadTxn as _,
    StateVector,
    Transact as _,
    updates::{decoder::Decode as _, encoder::Encode as _},
};

use crate::{
    command::{
        board_load::CacheUse,
        fixture::{after_channel, channel, channel_with},
    },
    domain::{
        board::{EditRight, LoadedBoard, state_vector::ahead},
        board_text::index::BoardIndex,
    },
    effect::{
        board_write::LoadSource,
        cache::CachePort,
        clock::FixedClock,
        socket::SocketConnector,
    },
    error::{
        Error,
        Result,
        board_socket::{BoardSocketError, SocketErrorCode},
    },
    protocol::hud::{DocumentResponse, SocketFrame, UpdatePayload},
    remote::{
        client::{Client, fake},
        socket::{
            fake::{
                BOARD,
                answer,
                ids_of,
                link_update,
                linked_board,
                script,
                sticker_by,
                update,
            },
            load::{load_seeded, load_with},
        },
    },
    storage::cache::memory::MemoryCache,
};

#[tokio::test]
async fn a_second_load_skips_the_link() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let (http, client) = fake::on_fake(
        [channel(), body(state), channel()],
        Some(fake::stored("tok", "s=1")),
    )?;
    let first = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert!(matches!(first.1, LoadSource::Downloaded { .. }));
    assert!(memory.meta(BOARD).is_some(), "the load was not cached");
    let second = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert_eq!(second.1, LoadSource::Skipped);
    assert_eq!(downloads(&http), 1, "the link was downloaded twice");
    assert_eq!(ids_of(&second.0), ["cached"]);
    let offline =
        load_cached(&client, &script([]), Some(&memory), CacheUse::Offline).await?;
    assert_eq!(ids_of(&offline.0), ["cached"]);
    assert_eq!(offline.0.right, EditRight::Editable);
    Ok(())
}

#[tokio::test]
async fn a_corrupt_copy_loads_whole() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    memory.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: b"not a yjs update",
            state_vector: &server.encode_v1(),
            index: None,
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    let (http, client) = fake::on_fake(
        after_channel([body(state)]),
        Some(fake::stored("tok", "s=1")),
    )?;
    let loaded = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert!(matches!(loaded.1, LoadSource::Downloaded { .. }));
    assert_eq!(downloads(&http), 1, "the corrupt copy skipped the link");
    assert_eq!(ids_of(&loaded.0), ["cached"]);
    assert_eq!(cached_ids(&memory)?, ["cached"], "the cache did not heal");
    Ok(())
}

#[cfg(feature = "logging")]
#[tokio::test]
async fn the_age_of_a_copy_is_told_by_the_client_clock() -> Result<()> {
    use tracing::instrument::WithSubscriber as _;
    struct Lines(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    const YEAR_MS: u64 = 31_536_000_000;
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let (http, client) = fake::on_fake(
        [channel(), body(state.clone()), channel(), body(state)],
        Some(fake::stored("tok", "s=1")),
    )?;
    load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    let later = Client::new(
        Some(fake::stored("tok", "s=1")),
        "https://app.holst.so".parse()?,
        FixedClock {
            unix_millis: YEAR_MS,
        },
        crate::domain::id_source::SequentialIds::new("id"),
    )?
    .with_transport(Arc::clone(&http));
    let cache = memory.with_clock(FixedClock {
        unix_millis: YEAR_MS,
    });
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&buffer);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || Lines(Arc::clone(&shared)))
        .finish();
    load_cached(&later, &linked_board(&server), Some(&cache), CacheUse::Use)
        .with_subscriber(subscriber)
        .await?;
    let text =
        String::from_utf8_lossy(&buffer.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned();
    assert!(
        text.contains("age_s=31535999"),
        "the age came from another clock: {text}"
    );
    Ok(())
}

#[tokio::test]
async fn a_copy_past_the_trust_age_is_downloaded_again() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let trust = crate::effect::settings::DEFAULT_CACHE_TRUST_MILLIS;
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let at = |unix_millis| memory.clone().with_clock(FixedClock { unix_millis });
    // A channel reply for each load, a download for each load not skipped.
    let download = || body(state.clone());
    let replies = [
        channel(),
        download(),
        channel(),
        channel(),
        download(),
        channel(),
        channel(),
        download(),
        channel(),
        download(),
    ];
    let (http, client) = fake::on_fake(replies, Some(fake::stored("tok", "s=1")))?;
    let load = async |cache: &MemoryCache, cache_use| {
        load_cached(&client, &linked_board(&server), Some(cache), cache_use)
            .await
            .map(|(_, fetched)| fetched)
    };
    let fetched_at = || memory.meta(BOARD).map(|meta| meta.downloaded_at);
    assert!(matches!(
        load(&memory, CacheUse::Use).await?,
        LoadSource::Downloaded { .. }
    ));
    assert_eq!(load(&at(trust), CacheUse::Use).await?, LoadSource::Skipped);
    assert_eq!(fetched_at(), Some(1), "a skipped link is no full download");
    assert!(matches!(
        load(&at(trust + 1), CacheUse::Use).await?,
        LoadSource::Downloaded { .. }
    ));
    assert_eq!(downloads(&http), 2, "an old copy skipped the link");
    assert_eq!(fetched_at(), Some(trust + 1));
    assert_eq!(
        load(&at(trust + 2), CacheUse::Use).await?,
        LoadSource::Skipped
    );
    let never = at(trust + 2).with_trust(0);
    assert!(matches!(
        load(&never, CacheUse::Use).await?,
        LoadSource::Downloaded { .. }
    ));
    let index = index_cached(
        &client,
        &linked_board(&server),
        Some(&at(3 * trust)),
        CacheUse::Use,
    )
    .await?;
    assert_eq!(index.1.len(), 1);
    assert_eq!(downloads(&http), 4, "an old index skipped the download");
    let offline = load(&at(3 * trust), CacheUse::Offline).await?;
    assert_eq!(
        offline,
        LoadSource::Skipped,
        "offline still serves the old copy"
    );
    Ok(())
}

#[tokio::test]
async fn a_current_index_needs_no_document() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let (news, news_sv) = sticker_by(9, "news");
    let both = crate::domain::board_file::document::decoded(&state)?;
    both.transact_mut()
        .apply_update(yrs::Update::decode_v2(&news)?)?;
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let replies = [
        channel(),
        body(state.clone()),
        channel(),
        body(crate::domain::board_file::document::encoded(&both)),
    ];
    let (http, client) = fake::on_fake(replies, Some(fake::stored("tok", "s=1")))?;
    let socket = || linked_board(&server);
    let (_, first) =
        index_cached(&client, &socket(), Some(&memory), CacheUse::Use).await?;
    assert_eq!(first.len(), 1);
    assert!(memory.load_index(BOARD).is_some(), "no index was cached");
    // The state goes: only the index can serve the next call.
    memory.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: &[],
            state_vector: &server.encode_v1(),
            index: memory.load_index(BOARD).as_deref(),
            right: EditRight::Editable,
            downloaded: false,
            verified: true,
        },
    )?;
    let (access, warm) =
        index_cached(&client, &socket(), Some(&memory), CacheUse::Use).await?;
    assert_eq!((access, warm.len()), (EditRight::Editable, 1));
    assert_eq!(downloads(&http), 1, "the link was downloaded again");
    let ahead = std::iter::once(news_sv).fold(server.clone(), |mut merged, other| {
        merged.merge(other);
        merged
    });
    // The check and the load that follows read a connection each.
    let announced = || {
        vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            link_update(&ahead),
        ]
    };
    let (_, behind) = index_cached(
        &client,
        &script([Some(announced()), Some(announced())]),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert_eq!(behind.len(), 2, "an index behind the server was used");
    assert_eq!(downloads(&http), 2, "a board with news did not load");
    Ok(())
}

#[tokio::test]
async fn an_unannounced_state_is_never_trusted() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let (news, _) = sticker_by(9, "news");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let index = BoardIndex::from_document(
        &crate::domain::board_file::document::decoded(&state)?,
        BOARD,
    );
    memory.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: &state,
            state_vector: &server.encode_v1(),
            index: Some(&index.encode()?),
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    let blind = || {
        vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update(0, UpdatePayload::Inline(news.clone())),
        ]
    };
    let (_, client) = fake::on_fake([channel()], Some(fake::stored("tok", "s=1")))?;
    let socket = script([Some(blind()), Some(blind())]);
    let (_, loaded) =
        index_cached(&client, &socket, Some(&memory), CacheUse::Use).await?;
    assert_eq!(loaded.len(), 2, "an unchecked cached index was served");
    Ok(())
}

#[tokio::test]
async fn refresh_ignores_the_cache() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let seeded = crate::domain::board_file::document::encoded(
        &crate::domain::board_file::document::decoded(&state)?,
    );
    memory.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: &seeded,
            state_vector: &server.encode_v1(),
            index: None,
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    let (http, client) = fake::on_fake(
        after_channel([body(state)]),
        Some(fake::stored("tok", "s=1")),
    )?;
    let (_, fetched) = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Refresh,
    )
    .await?;
    assert!(matches!(fetched, LoadSource::Downloaded { .. }));
    assert_eq!(downloads(&http), 1);
    Ok(())
}

#[tokio::test]
async fn offline_without_a_copy_fails_clearly() -> Result<()> {
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let missing =
        load_cached(&client, &script([]), Some(&memory), CacheUse::Offline).await;
    let not_cached =
        Error::LocalData(crate::error::local_data::LocalDataError::NotCached {
            board: BOARD.to_owned(),
        });
    assert_eq!(
        missing.as_ref().err().map(ToString::to_string),
        Some(not_cached.to_string())
    );
    let hint = missing.err().and_then(|err| err.hint());
    assert_eq!(hint, Some(crate::error::hint::Hint::LoadOnline));
    let off = load_cached(&client, &script([]), None, CacheUse::Offline).await;
    assert!(
        matches!(
            off,
            Err(Error::LocalData(
                crate::error::local_data::LocalDataError::CacheOff
            ))
        ),
        "{off:?}"
    );
    Ok(())
}

async fn load_cached(
    client: &Client,
    socket: &dyn SocketConnector,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(LoadedBoard, LoadSource)> {
    crate::command::board_load::load(
        crate::command::board_load::RemoteBoard {
            client,
            connector: socket,
            board: BOARD,
        },
        cache,
        cache_use,
    )
    .await
}

async fn index_cached(
    client: &Client,
    socket: &dyn SocketConnector,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(EditRight, BoardIndex)> {
    crate::command::board_load::index(
        crate::command::board_load::RemoteBoard {
            client,
            connector: socket,
            board: BOARD,
        },
        cache,
        cache_use,
    )
    .await
}

fn stickers_by(client: u64, ids: &[&str]) -> (Vec<u8>, StateVector) {
    stickers_in("objects", client, ids)
}

fn stickers_in(root: &str, client: u64, ids: &[&str]) -> (Vec<u8>, StateVector) {
    let doc = Doc::with_client_id(client);
    let objects = doc.get_or_insert_map(root);
    ids.iter().fold((), |(), id| {
        let fields = json!({"id": id, "type": "sticker", "position": {"x": 1, "y": 2}});
        let sticker = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
        objects.insert(&mut doc.transact_mut(), *id, sticker);
    });
    let txn = doc.transact();
    (
        txn.encode_state_as_update_v2(&StateVector::default()),
        txn.state_vector(),
    )
}

pub(crate) fn cached_copy(cache: &dyn CachePort, state: &[u8]) -> Result<()> {
    let doc = crate::domain::board_file::document::decoded(state)?;
    let index = BoardIndex::from_document(&doc, BOARD).encode()?;
    cache.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: state,
            state_vector: &doc.transact().state_vector().encode_v1(),
            index: Some(&index),
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )
}

pub(crate) fn cached_ids(cache: &dyn CachePort) -> Result<Vec<String>> {
    let state = cache.load_document(BOARD).unwrap_or_default();
    let doc = crate::domain::board_file::document::decoded(&state)?;
    Ok(crate::domain::board::objects(&doc)
        .into_iter()
        .map(|object| object.id)
        .sorted()
        .collect())
}

pub(crate) fn restored_server() -> (Vec<u8>, StateVector, Vec<u8>) {
    let (copy, copy_sv) = stickers_by(4, &["mine", "more"]);
    let (server, server_sv) = stickers_by(4, &["restored"]);
    assert!(ahead(&copy_sv, &server_sv), "the fixture is not ahead");
    (copy, server_sv, server)
}

/// The requests that are not for the board's channel: the link downloads.
fn downloads(http: &fake::Fake) -> usize {
    http.requests()
        .iter()
        .filter(|request| !request.url.path().contains("/channel/"))
        .count()
}

pub(crate) fn body(body: Vec<u8>) -> crate::effect::http::HttpResponse {
    crate::effect::http::HttpResponse {
        body,
        ..fake::reply(200, &[], "")
    }
}

pub(crate) fn linked_times(
    server: &StateVector,
    times: usize,
) -> Vec<Option<Vec<SocketFrame>>> {
    (0..times)
        .map(|_| {
            Some(vec![
                answer(DocumentResponse::Subscribed {
                    right: EditRight::Editable,
                }),
                link_update(server),
            ])
        })
        .collect()
}

#[tokio::test]
async fn a_seed_ahead_of_the_server_is_refused() -> Result<()> {
    let (copy, server_sv, _) = restored_server();
    let seed = crate::domain::board_file::document::decoded(&copy)?;
    let (http, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let loaded = load_seeded(&client, &linked_board(&server_sv), BOARD, seed).await;
    assert!(
        matches!(loaded, Err(Error::BoardSocket(BoardSocketError::CopyAhead))),
        "{loaded:?}"
    );
    assert!(http.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn a_copy_ahead_of_the_server_heals() -> Result<()> {
    let (copy, server_sv, server) = restored_server();
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let stored = || Some(fake::stored("tok", "s=1"));
    cached_copy(&memory, &copy)?;
    let (_, loading) = fake::on_fake(after_channel([body(server.clone())]), stored())?;
    let (loaded, fetched) = load_cached(
        &loading,
        &script(linked_times(&server_sv, 2)),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert!(matches!(fetched, LoadSource::Downloaded { .. }));
    assert_eq!(ids_of(&loaded), ["restored"]);
    assert_eq!(cached_ids(&memory)?, ["restored"], "the cache did not heal");

    cached_copy(&memory, &copy)?;
    let (_, indexing) = fake::on_fake(after_channel([body(server)]), stored())?;
    let (_, index) = index_cached(
        &indexing,
        &script(linked_times(&server_sv, 2)),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    let ids: Vec<String> = index
        .objects()
        .into_iter()
        .map(|object| object.id)
        .collect();
    assert_eq!(ids, ["restored"], "an index ahead of the server was used");
    assert_eq!(cached_ids(&memory)?, ["restored"]);

    Ok(())
}

#[tokio::test]
async fn a_board_closed_to_the_user_leaves_the_cache() -> Result<()> {
    let (state, _) = sticker_by(4, "secret");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let refused = |code: SocketErrorCode| {
        script([Some(vec![answer(DocumentResponse::NotSubscribed { code })])])
    };
    // A channel reply for each load and for the check of a refusal.
    let (_, client) = fake::on_fake(
        [channel(), channel(), channel()],
        Some(fake::stored("tok", "s=1")),
    )?;
    cached_copy(&memory, &state)?;
    let failed = load_cached(
        &client,
        &refused(SocketErrorCode::InternalError),
        Some(&memory),
        CacheUse::Use,
    )
    .await;
    assert!(failed.is_err());
    assert!(memory.meta(BOARD).is_some(), "a server error dropped it");
    // A deleted board: the board server says `internal-error`, the API 404.
    let gone = fake::reply(404, &[], r#"{"errCode":"channel-not-found"}"#);
    let (_, asking) =
        fake::on_fake([channel(), gone], Some(fake::stored("tok", "s=1")))?;
    let deleted = load_cached(
        &asking,
        &refused(SocketErrorCode::InternalError),
        Some(&memory),
        CacheUse::Use,
    )
    .await;
    assert!(deleted.is_err());
    assert_eq!(memory.meta(BOARD), None, "a deleted board stayed cached");
    cached_copy(&memory, &state)?;
    let forbidden = load_cached(
        &client,
        &refused(SocketErrorCode::Forbidden),
        Some(&memory),
        CacheUse::Use,
    )
    .await;
    assert!(forbidden.is_err());
    assert_eq!(memory.meta(BOARD), None, "a lost board stayed cached");
    cached_copy(&memory, &state)?;
    let index = index_cached(
        &client,
        &refused(SocketErrorCode::Forbidden),
        Some(&memory),
        CacheUse::Use,
    )
    .await;
    assert!(index.is_err());
    assert_eq!(memory.meta(BOARD), None, "a lost board's index stayed");
    let offline =
        load_cached(&client, &script([]), Some(&memory), CacheUse::Offline).await;
    assert!(
        matches!(
            offline,
            Err(Error::LocalData(
                crate::error::local_data::LocalDataError::NotCached { .. }
            ))
        ),
        "{offline:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_offline_read_is_not_a_verification() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    cached_copy(&memory, &state)?;
    let later = 1 + crate::effect::cache::meta::TOUCH_EVERY_MILLIS;
    let at = |unix_millis| memory.clone().with_clock(FixedClock { unix_millis });
    let (_, client) = fake::on_fake([], Some(fake::stored("tok", "s=1")))?;
    let offline = CacheUse::Offline;
    load_cached(&client, &script([]), Some(&at(later)), offline).await?;
    let meta = memory
        .meta(BOARD)
        .ok_or(crate::error::local_data::LocalDataError::NoHome)?;
    assert_eq!((meta.last_used, meta.verified_at), (later, 1));
    index_cached(&client, &script([]), Some(&at(later)), offline).await?;
    let checked = at(later + 9);
    let socket = linked_board(&server);
    index_cached(&client, &socket, Some(&checked), CacheUse::Use).await?;
    let checked_meta = memory
        .meta(BOARD)
        .ok_or(crate::error::local_data::LocalDataError::NoHome)?;
    assert_eq!(
        checked_meta.verified_at,
        later + 9,
        "a server check did not count"
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_or_stale_index_is_rebuilt() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    memory.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: &state,
            state_vector: &server.encode_v1(),
            index: Some(b"not an index"),
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    let (http, client) = fake::on_fake([channel()], Some(fake::stored("tok", "s=1")))?;
    let (_, fetched) = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert_eq!(fetched, LoadSource::Skipped);
    assert_eq!(downloads(&http), 0);
    let indexed = || {
        memory
            .load_index(BOARD)
            .map(|bytes| BoardIndex::decode(&bytes))
            .transpose()
            .map(|index| index.map(|index| index.len()))
    };
    assert_eq!(indexed()?, Some(1), "the index was not rebuilt");

    let (two, two_sv) = stickers_by(4, &["cached", "newer"]);
    let old = BoardIndex::from_document(
        &crate::domain::board_file::document::decoded(&state)?,
        BOARD,
    );
    memory.store(
        BOARD,
        &crate::effect::cache::CacheRecord {
            document: &two,
            state_vector: &two_sv.encode_v1(),
            index: Some(&old.encode()?),
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    let (_, offline) =
        index_cached(&client, &script([]), Some(&memory), CacheUse::Offline).await?;
    assert_eq!(
        offline.len(),
        2,
        "an index older than its document was used"
    );
    assert_eq!(indexed()?, Some(2), "the rebuilt index was not kept");
    Ok(())
}

#[derive(Clone)]
struct Spy {
    memory: MemoryCache,
    threads: Arc<Mutex<Vec<std::thread::ThreadId>>>,
}

impl Spy {
    fn wrote(&self) {
        self.threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(std::thread::current().id());
    }
}

impl CachePort for Spy {
    fn meta(&self, board: &str) -> Option<crate::effect::cache::meta::CacheMeta> {
        self.memory.meta(board)
    }

    fn trusts(&self, meta: &crate::effect::cache::meta::CacheMeta) -> bool {
        self.memory.trusts(meta)
    }

    fn load_document(&self, board: &str) -> Option<Vec<u8>> {
        self.memory.load_document(board)
    }

    fn load_index(&self, board: &str) -> Option<Vec<u8>> {
        self.memory.load_index(board)
    }

    fn store(
        &self,
        board: &str,
        record: &crate::effect::cache::CacheRecord<'_>,
    ) -> Result<()> {
        self.wrote();
        self.memory.store(board, record)
    }

    fn touch(
        &self,
        board: &str,
        seen: crate::effect::cache::meta::ReadCheck,
    ) -> Result<()> {
        self.wrote();
        self.memory.touch(board, seen)
    }

    fn discard(&self, board: &str) -> Result<()> {
        self.wrote();
        self.memory.discard(board)
    }

    fn evict(&self) -> Result<crate::effect::cache::CacheUsage> {
        self.memory.evict()
    }

    fn evictable(&self) -> crate::effect::cache::CacheUsage {
        self.memory.evictable()
    }

    fn wipe(&self) -> Result<()> {
        self.memory.wipe()
    }

    fn usage(&self) -> crate::effect::cache::CacheUsage {
        self.memory.usage()
    }

    fn shared(&self) -> Arc<dyn CachePort> {
        Arc::new(self.clone())
    }
}

#[tokio::test]
async fn cache_writes_leave_the_runtime_thread() -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let spy = Spy {
        memory: MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 }),
        threads: Arc::default(),
    };
    let stored = Some(fake::stored("tok", "s=1"));
    let (_, client) = fake::on_fake(after_channel([body(state)]), stored)?;
    let socket = linked_board(&server);
    load_cached(&client, &socket, Some(&spy), CacheUse::Use).await?;
    let here = std::thread::current().id();
    let threads = spy
        .threads
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert!(!threads.is_empty(), "nothing was cached");
    assert!(threads.iter().all(|thread| *thread != here), "{threads:?}");
    Ok(())
}

#[tokio::test]
async fn no_login_no_socket() -> Result<()> {
    let (_, client) = fake::on_fake([], None)?;
    let socket = script([]);
    assert!(load_with(&client, &socket, BOARD).await.is_err());
    assert!(
        socket
            .0
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    Ok(())
}

#[rstest::rstest]
#[case::no_version(json!({"server_provider_type": "hud"}), 1, "hud")]
#[case::hocuspocus(
    json!({"schema_version": 2, "server_provider_type": "hocuspocus"}),
    2,
    "hocuspocus"
)]
#[tokio::test]
async fn a_board_in_another_format_is_refused_unread(
    #[case] format: serde_json::Value,
    #[case] schema: u32,
    #[case] provider: &str,
) -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let (_, client) = fake::on_fake(
        [channel_with(&format), body(state)],
        Some(fake::stored("tok", "s=1")),
    )?;
    let refused = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await;
    let unsupported = Error::BoardSocket(BoardSocketError::UnsupportedFormat {
        schema,
        provider: provider.to_owned(),
    });
    assert_eq!(
        refused.err().map(|error| error.to_string()),
        Some(unsupported.to_string())
    );
    assert!(memory.meta(BOARD).is_none(), "a refused board was cached");
    Ok(())
}

#[rstest::rstest]
#[case::v2_on_hud(json!({"schema_version": 2, "server_provider_type": "hud"}))]
#[case::no_provider(json!({"schema_version": 2}))]
#[tokio::test]
async fn a_board_in_format_v2_on_hud_loads(
    #[case] format: serde_json::Value,
) -> Result<()> {
    let (state, server) = sticker_by(4, "cached");
    let (_, client) = fake::on_fake(
        [channel_with(&format), body(state)],
        Some(fake::stored("tok", "s=1")),
    )?;
    let loaded =
        load_cached(&client, &linked_board(&server), None, CacheUse::Use).await?;
    assert_eq!(ids_of(&loaded.0), ["cached"]);
    Ok(())
}

#[tokio::test]
async fn a_board_in_format_v3_on_hud_loads_and_reads_offline() -> Result<()> {
    let (state, server) = stickers_in("objects_v3", 4, &["cached"]);
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let format = json!({"schema_version": 3, "server_provider_type": "hud"});
    let (_, client) = fake::on_fake(
        [channel_with(&format), body(state)],
        Some(fake::stored("tok", "s=1")),
    )?;
    let loaded = load_cached(
        &client,
        &linked_board(&server),
        Some(&memory),
        CacheUse::Use,
    )
    .await?;
    assert_eq!(ids_of(&loaded.0), ["cached"]);
    let offline =
        load_cached(&client, &script([]), Some(&memory), CacheUse::Offline).await?;
    assert_eq!(ids_of(&offline.0), ["cached"]);
    Ok(())
}

async fn root_loaded(
    client: &Client,
    server: &StateVector,
    memory: &MemoryCache,
) -> Result<&'static str> {
    let (loaded, _) =
        load_cached(client, &linked_board(server), Some(memory), CacheUse::Use).await?;
    Ok(crate::domain::board::objects_root(&loaded.document))
}

#[rstest::rstest]
#[case::v2(2, "objects")]
#[case::v3(3, "objects_v3")]
#[tokio::test]
async fn every_load_registers_the_objects_root_of_the_format(
    #[case] schema: u32,
    #[case] root: &str,
) -> Result<()> {
    let (state, server) = stickers_in(root, 4, &[]);
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let format = json!({"schema_version": schema, "server_provider_type": "hud"});
    let (_, client) = fake::on_fake(
        [
            channel_with(&format),
            body(state.clone()),
            channel_with(&format),
            body(state),
        ],
        Some(fake::stored("tok", "s=1")),
    )?;
    let first = root_loaded(&client, &server, &memory).await?;
    let second = root_loaded(&client, &server, &memory).await?;
    assert_eq!([first, second], [root, root]);
    Ok(())
}

#[rstest::rstest]
#[case::editor_under_co_owner(&["write"], "co_owner", Some(("co_owner", "editor")))]
#[case::co_owner_under_editor(&["set_permits", "write"], "editor", None)]
#[case::editor_under_editor(&["write"], "editor", None)]
#[case::viewer_under_editor(&["read"], "editor", Some(("editor", "viewer")))]
#[case::unknown_role(&["read"], "", None)]
fn a_setting_above_the_callers_role_is_withheld(
    #[case] permits: &[&str],
    #[case] needs: &str,
    #[case] expected: Option<(&str, &str)>,
) {
    let channel = json!({ "permits": permits, "who_can_delete_content": needs });
    let got = crate::command::board_load::withheld(&channel, "who_can_delete_content");
    assert_eq!(
        got.as_ref().map(|(least, holds)| (least.as_str(), *holds)),
        expected
    );
}
