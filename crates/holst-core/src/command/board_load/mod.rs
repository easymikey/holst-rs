//! Board loads through the board cache, its writes off the runtime.

#[cfg(test)] pub(crate) mod tests;

use futures_util::future::{OptionFuture, try_join};
use serde::Deserialize;
use serde_json::Value;
use yrs::{
    Doc,
    ReadTxn as _,
    StateVector,
    Transact as _,
    updates::encoder::Encode as _,
};

use crate::{
    domain::{
        board::{BoardContent, EditRight, LoadedBoard},
        board_file::document,
        board_text::index::BoardIndex,
    },
    effect::{
        board_write::LoadSource,
        cache::{
            CachePort,
            CacheRecord,
            meta::ReadCheck,
            trust,
            trust::{CachedCopy, StaleCopy, Trusted, Unverified},
        },
        socket::SocketConnector,
    },
    error::{
        Error,
        Result,
        board_socket::BoardSocketError,
        local_data::LocalDataError,
    },
    protocol::channel::BoardFormat,
    remote::{
        client::Client,
        socket::load::{announced, load_seeded},
    },
};

/// How a load uses the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheUse {
    Use,
    Refresh,
    Offline,
}

/// The board to reach and the way to reach it.
#[derive(Clone, Copy)]
pub struct RemoteBoard<'a> {
    pub client: &'a Client,
    pub connector: &'a dyn SocketConnector,
    pub board: &'a str,
}

impl std::fmt::Debug for RemoteBoard<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoardLink")
            .field("board", &self.board)
            .finish_non_exhaustive()
    }
}

/// A loaded document to keep in the cache, and how it came.
#[derive(Debug, Clone, Copy)]
pub struct LoadedCopy<'a> {
    pub document: &'a Doc,
    pub right: EditRight,
    pub seed_state: Option<&'a StateVector>,
    pub source: LoadSource,
}

/// A document to store, and how far the copy can be trusted.
#[derive(Debug, Clone, Copy)]
pub(super) struct DocumentRecord<'a> {
    pub(super) doc: &'a Doc,
    pub(super) access: EditRight,
    pub(super) downloaded: bool,
    pub(super) read_check: ReadCheck,
}

/// The board loaded through `cache` as `cache_use` says, and kept there.
///
/// # Errors
/// As [`load_seeded`]; offline,
/// [`crate::error::local_data::LocalDataError::CacheOff`]
/// or [`crate::error::local_data::LocalDataError::NotCached`].
pub async fn load(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(LoadedBoard, LoadSource)> {
    let (loaded, source, _) = load_with_channel(link, cache, cache_use).await?;
    Ok((loaded, source))
}

/// As [`load`], with the format check's channel reply; none when offline.
///
/// # Errors
/// As [`load`].
pub async fn load_with_channel(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(LoadedBoard, LoadSource, Option<Value>)> {
    let RemoteBoard {
        client,
        connector,
        board,
    } = link;
    match (cache_use, cache) {
        (CacheUse::Offline, None) => Err(Error::LocalData(LocalDataError::CacheOff)),
        (CacheUse::Offline, Some(cache)) => offline(cache, board).await,
        (CacheUse::Use, Some(cache)) => {
            let seed =
                trusted_seed(cache, board, client.unix_millis(), document::decoded);
            through(link, cache, seed).await
        }
        (CacheUse::Refresh, Some(cache)) => through(link, cache, None).await,
        (CacheUse::Use | CacheUse::Refresh, None) => {
            let (loaded, source, _, channel) =
                seeded_channel(client, connector, board, None).await?;
            Ok((loaded, source, Some(channel)))
        }
    }
}

async fn channel_reply(client: &Client, board: &str) -> Result<Value> {
    crate::remote::api::channel::get(client, board).await
}

async fn format(client: &Client, board: &str) -> Result<BoardFormat> {
    Ok(serde_json::from_value(channel_reply(client, board).await?)?)
}

/// Objects-document root for a format this program reads; no socket call.
///
/// # Errors
/// The API's refusal, or [`BoardSocketError::UnsupportedFormat`].
pub async fn supported(client: &Client, board: &str) -> Result<&'static str> {
    format(client, board).await?.supported()
}

/// `Ok` when the board is stored in the format of a whole-board file (v2).
///
/// # Errors
/// As [`supported`], and for a version 3 board.
pub async fn whole_file(client: &Client, board: &str) -> Result<()> {
    format(client, board).await?.whole_file()
}

/// The roles of the web client's scale, the lowest first.
const SCALE: [&str; 5] = ["viewer", "commentator", "editor", "co_owner", "owner"];

/// The role each permit gives, the top one first; a user with none is a viewer.
const BY_PERMIT: [(&str, &str); 3] = [
    ("set_permits", "co_owner"),
    ("write", "editor"),
    ("write_comments", "commentator"),
];

fn rank(role: &str) -> Option<usize> {
    SCALE.iter().position(|known| *known == role)
}

/// The role the board asks for in its `who_can_*` field `setting` and the
/// role the caller's permits give there, when the first stands above the
/// second, as the web client compares them.
pub(crate) fn withheld(
    channel: &Value,
    setting: &str,
) -> Option<(String, &'static str)> {
    let held = BY_PERMIT
        .iter()
        .find(|(permit, _)| {
            channel
                .get("permits")
                .and_then(Value::as_array)
                .is_some_and(|permits| {
                    permits.iter().any(|held| held.as_str() == Some(permit))
                })
        })
        .map_or("viewer", |(_, role)| *role);
    channel
        .get(setting)
        .and_then(Value::as_str)
        .filter(|needs| rank(needs) > rank(held))
        .map(|needs| (needs.to_owned(), held))
}

/// The board loaded into `seed`, if any; a seed ahead of the server goes and
/// the board loads whole. Also the state vector of the seed that was used.
///
/// # Errors
/// As [`load_seeded`].
pub async fn seeded(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    seed: Option<Doc>,
) -> Result<(LoadedBoard, LoadSource, Option<StateVector>)> {
    let (loaded, source, used, _) =
        seeded_channel(client, connector, board, seed).await?;
    Ok((loaded, source, used))
}

async fn seeded_channel(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    seed: Option<Doc>,
) -> Result<(LoadedBoard, LoadSource, Option<StateVector>, Value)> {
    let before = seed.as_ref().map(|doc| doc.transact().state_vector());
    let loading = async {
        match load_seeded(client, connector, board, seed.unwrap_or_default()).await {
            Err(Error::BoardSocket(BoardSocketError::CopyAhead)) => {
                load_seeded(client, connector, board, Doc::new())
                    .await
                    .map(|loaded| (loaded, None))
            }
            other => other.map(|loaded| (loaded, before)),
        }
    };
    // Registered, so an empty version 3 board writes under `objects_v3`.
    let (channel, ((loaded, source), used)) =
        try_join(channel_reply(client, board), loading).await?;
    let root = BoardFormat::deserialize(&channel)?.supported()?;
    loaded.document.get_or_insert_map(root);
    Ok((loaded, source, used, channel))
}

/// The board's objects loaded through `cache`.
///
/// # Errors
/// As [`load`].
pub async fn content(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<BoardContent> {
    let (content, _) = content_with_channel(link, cache, cache_use).await?;
    Ok(content)
}

/// As [`content`], with the `channel/{id}` reply the load read, if any.
///
/// # Errors
/// As [`load`].
pub async fn content_with_channel(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(BoardContent, Option<Value>)> {
    let (loaded, _, channel) = load_with_channel(link, cache, cache_use).await?;
    let content = BoardContent {
        edit_right: loaded.right,
        objects: crate::domain::board::objects(&loaded.document),
    };
    Ok((content, channel))
}

/// The `channel/{id}` reply a load read, or the board's own when it read none.
///
/// # Errors
/// The API's refusal of the board.
pub async fn channel_or_get(
    client: &Client,
    board: &str,
    read: Option<Value>,
) -> Result<Value> {
    match read {
        Some(reply) => Ok(reply),
        None => channel_reply(client, board).await,
    }
}

async fn offline(
    cache: &dyn CachePort,
    board: &str,
) -> Result<(LoadedBoard, LoadSource, Option<Value>)> {
    let cached = CachedCopy::read(cache, board)
        .map(CachedCopy::offline)
        .and_then(|cached| cached.with_document(cache, document::decoded).ok())
        .ok_or_else(|| {
            Error::LocalData(LocalDataError::NotCached {
                board: board.to_owned(),
            })
        })?;
    touched(cache, board, cached.read_check()).await;
    let access = cached.right();
    Ok((
        LoadedBoard {
            right: access,
            document: cached.into_body(),
        },
        LoadSource::Skipped,
        None,
    ))
}

pub(super) fn trusted_seed(
    cache: &dyn CachePort,
    board: &str,
    now_ms: u64,
    restore: impl FnOnce(&[u8]) -> Result<Doc>,
) -> Option<Doc> {
    let cached = CachedCopy::read(cache, board)?;
    let age_s = cached.age_millis(now_ms) / 1000;
    let seed = cached
        .trusted(cache)
        .inspect_err(|_| {
            tracing::debug!(
                board,
                age_s,
                "board cache: the last full download is past the trust age; downloading the board"
            );
        })
        .ok()?
        .with_document(cache, restore)
        .ok()?;
    tracing::debug!(board, age_s, "board cache: seeded from the cached copy");
    Some(seed.into_seed())
}

async fn through(
    link: RemoteBoard<'_>,
    cache: &dyn CachePort,
    seed: Option<Doc>,
) -> Result<(LoadedBoard, LoadSource, Option<Value>)> {
    let RemoteBoard {
        client,
        connector,
        board,
    } = link;
    let had_seed = seed.is_some();
    let (result, before) = match seeded_channel(client, connector, board, seed).await {
        Ok((loaded, source, before, channel)) => {
            if had_seed && before.is_none() {
                tracing::debug!(board, "board cache: the copy is ahead of the server");
                forget(cache, board).await;
            }
            (Ok((loaded, source, Some(channel))), before)
        }
        Err(error) => (Err(error), None),
    };
    let (loaded, source, channel) = guarded(client, Some(cache), board, result).await?;
    tracing::debug!(board, ?source, "board cache: loaded");
    remember(
        cache,
        board,
        LoadedCopy {
            document: &loaded.document,
            right: loaded.right,
            seed_state: before.as_ref(),
            source,
        },
    )
    .await;
    Ok((loaded, source, channel))
}

async fn guarded<T>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    board: &str,
    result: Result<T>,
) -> Result<T> {
    guarded_loss(client, cache, board, result).await.0
}

/// As `guarded`, with whether the board turned out closed to the user. A
/// failed `result` costs one request to tell; a closed board leaves `cache`.
pub async fn guarded_loss<T>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    board: &str,
    result: Result<T>,
) -> (Result<T>, bool) {
    let failed = result.as_ref().err();
    let lost = OptionFuture::from(failed.map(|err| is_closed(client, board, err)))
        .await
        .unwrap_or_default();
    let dropping = cache.filter(|_| lost).map(|cache| {
        tracing::debug!(board, "board cache: no access to the board; dropping it");
        forget(cache, board)
    });
    OptionFuture::from(dropping).await;
    lost.then(|| super::lookup::forget_trees_holding(client, board));
    (result, lost)
}

/// Whether `err` shows the board closed to the user: 403, 404 or `forbidden`
/// at once; another refusal of the board server (`internal-error` is how it
/// answers for a deleted board) asks the API, which tells a 404.
async fn is_closed(client: &Client, board: &str, err: &Error) -> bool {
    let refused = matches!(
        err,
        Error::BoardSocket(BoardSocketError::NotSubscribed { .. })
    );
    err.is_board_closed() || (refused && is_denied_by_api(client, board).await)
}

async fn is_denied_by_api(client: &Client, board: &str) -> bool {
    crate::remote::api::channel::get(client, board)
        .await
        .err()
        .is_some_and(|lost| lost.is_board_closed())
}

/// Keep `doc` loaded from a seed at `before`, or only touch it when it and its
/// index are current; off the runtime, never failing the caller.
pub async fn remember(cache: &dyn CachePort, board: &str, loaded: LoadedCopy<'_>) {
    let LoadedCopy {
        document: doc,
        right: access,
        seed_state: before,
        source,
    } = loaded;
    let after = doc.transact().state_vector();
    let skipped = matches!(source, LoadSource::Skipped);
    let whole = before.is_none() || !skipped;
    let unchanged = skipped && before.is_some_and(|before| *before == after);
    // A missing or stale index is written again, the document changed or not.
    let current = unchanged && trust::is_indexed(cache, board, &after);
    let touching = current.then(|| touched(cache, board, ReadCheck::Verified));
    let storing = (!current).then(|| {
        let snapshot = DocumentRecord {
            doc,
            access,
            downloaded: whole,
            read_check: ReadCheck::Verified,
        };
        stored(cache, board, snapshot)
    });
    OptionFuture::from(touching).await;
    OptionFuture::from(storing).await;
}

pub(super) async fn stored(
    cache: &dyn CachePort,
    board: &str,
    snapshot: DocumentRecord<'_>,
) {
    let DocumentRecord {
        doc,
        access,
        downloaded: whole,
        read_check: seen,
    } = snapshot;
    let (board, doc) = (board.to_owned(), doc.clone());
    off_thread(cache, move |cache| {
        let state = document::encoded(&doc);
        let state_vector = doc.transact().state_vector().encode_v1();
        let index = BoardIndex::from_document(&doc, &board).encode()?;
        cache.store(
            &board,
            &CacheRecord {
                document: &state,
                state_vector: &state_vector,
                index: Some(&index),
                right: access,
                downloaded: whole,
                verified: matches!(seen, ReadCheck::Verified),
            },
        )
    })
    .await;
}

async fn touched(cache: &dyn CachePort, board: &str, seen: ReadCheck) {
    let board = board.to_owned();
    off_thread(cache, move |cache| cache.touch(&board, seen)).await;
}

/// Drop `board` from the cache off the runtime, e.g. when access was lost.
pub(super) async fn forget(cache: &dyn CachePort, board: &str) {
    let board = board.to_owned();
    off_thread(cache, move |cache| cache.discard(&board)).await;
}

async fn off_thread(
    cache: &dyn CachePort,
    work: impl FnOnce(&dyn CachePort) -> Result<()> + Send + 'static,
) {
    let handle = cache.shared();
    let done = tokio::task::spawn_blocking(move || work(&*handle)).await;
    match done {
        Ok(result) => logged(result),
        Err(err) => tracing::debug!(%err, "board cache: the write stopped"),
    }
}

/// The board's index through `cache`: a cached index that holds all the server
/// announces is used as it is, without the document; else the board loads.
///
/// # Errors
/// As [`load`].
pub async fn index(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(EditRight, BoardIndex)> {
    let (right, index, _) = index_with_channel(link, cache, cache_use).await?;
    Ok((right, index))
}

/// As [`index`], with the format check's channel reply; none if cached.
///
/// # Errors
/// As [`load`].
pub async fn index_with_channel(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(EditRight, BoardIndex, Option<Value>)> {
    let board = link.board;
    let found = cache
        .filter(|_| cache_use != CacheUse::Refresh)
        .and_then(|cache| cached_index(cache, board).map(|found| (cache, found)));
    match (found, cache_use) {
        (Some((held, cached)), CacheUse::Offline) => {
            let served = cached.offline();
            touched(held, board, served.read_check()).await;
            Ok((served.right(), served.into_body(), None))
        }
        (Some((held, cached)), CacheUse::Use) => match cached.trusted(held) {
            Ok(trusted) => checked(link, held, trusted).await,
            Err(_) => loaded_index(link, Some(held), cache_use).await,
        },
        (Some(_), CacheUse::Refresh) | (None, _) => {
            loaded_index(link, cache, cache_use).await
        }
    }
}

async fn checked(
    link: RemoteBoard<'_>,
    cache: &dyn CachePort,
    cached: CachedCopy<Trusted, BoardIndex>,
) -> Result<(EditRight, BoardIndex, Option<Value>)> {
    let RemoteBoard {
        client,
        connector,
        board,
    } = link;
    let result = announced(client, connector, board).await;
    let (access, server) = guarded(client, Some(cache), board, result).await?;
    let verified = cached.verify(server.as_ref());
    tracing::debug!(
        board,
        stale = ?verified.as_ref().err(),
        "board cache: checked the cached index"
    );
    match verified {
        Ok(verified) => {
            touched(cache, board, verified.read_check()).await;
            Ok((access, verified.into_body(), None))
        }
        Err(StaleCopy::Ahead) => {
            forget(cache, board).await;
            loaded_index(link, Some(cache), CacheUse::Refresh).await
        }
        Err(StaleCopy::Behind | StaleCopy::Expired | StaleCopy::Corrupt) => {
            loaded_index(link, Some(cache), CacheUse::Use).await
        }
    }
}

async fn loaded_index(
    link: RemoteBoard<'_>,
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<(EditRight, BoardIndex, Option<Value>)> {
    let board = link.board;
    let (loaded, _, channel) = load_with_channel(link, cache, cache_use).await?;
    let rebuilt = cache
        .filter(|_| cache_use == CacheUse::Offline)
        .map(|cache| {
            let snapshot = DocumentRecord {
                doc: &loaded.document,
                access: loaded.right,
                downloaded: false,
                read_check: ReadCheck::Read,
            };
            stored(cache, board, snapshot)
        });
    OptionFuture::from(rebuilt).await;
    Ok((
        loaded.right,
        BoardIndex::from_document(&loaded.document, board),
        channel,
    ))
}

fn cached_index(
    cache: &dyn CachePort,
    board: &str,
) -> Option<CachedCopy<Unverified, BoardIndex>> {
    let index = CachedCopy::read(cache, board)?.with_index(cache);
    if index.is_none() {
        tracing::debug!(
            board,
            "board cache: the index is missing or stale; rebuilding it"
        );
    }
    index
}

fn logged(result: Result<()>) {
    result
        .inspect_err(|err| tracing::debug!(%err, "board cache: not updated"))
        .unwrap_or_default();
}
