use std::sync::Arc;

use itertools::Itertools as _;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{
    command::{board_load, board_load::CacheUse},
    domain::board_text::{
        BoardLoads,
        BoardTextHits,
        BoardTextQuery,
        found_text,
        index::BoardIndex,
        split_loaded,
    },
    effect::cache::CachePort,
    error::Result,
    remote::{client::Client, socket::stream::TungsteniteConnector},
};

pub const LOAD_CONCURRENCY: usize = 6;

/// The index of one board through `cache`: a cached index the server announces
/// nothing beyond, else read over the socket; nothing is written to the board.
///
/// # Errors
/// As [`crate::remote::socket::load::load`].
pub async fn load_index(
    client: &Client,
    board: &str,
    cache: Option<&dyn CachePort>,
) -> Result<BoardIndex> {
    let (_, index) = board_load::index(
        board_load::RemoteBoard {
            client,
            connector: &TungsteniteConnector,
            board,
        },
        cache,
        CacheUse::Use,
    )
    .await?;
    Ok(index)
}

/// Searches the text of `boards` (id, name), loading each with `load`.
/// Boards that fail to load are reported in the result, not fatal.
///
/// # Errors
/// The first fatal load error ([`crate::Error::is_fatal`]).
pub async fn search_text<Load, Loading>(
    boards: Vec<(String, String)>,
    load: Load,
    search: &BoardTextQuery<'_>,
) -> Result<BoardTextHits>
where
    Load: Fn(String) -> Loading,
    Loading: Future<Output = Result<Arc<BoardIndex>>> + Send + 'static,
{
    let ids = boards.iter().map(|(id, _)| id.clone()).collect();
    let loaded = load_all_with(ids, LOAD_CONCURRENCY, load).await;
    let BoardLoads { read, unread } = split_loaded(loaded)?;
    // With no board read there is nothing to check the kind against.
    search
        .filter
        .kind
        .as_ref()
        .filter(|_| !read.is_empty())
        .map_or(Ok(()), |kind| {
            kind.check_among(read.iter().flat_map(|index| index.kinds()))
        })?;
    Ok(found_text(&read, boards, unread, search))
}

/// What `load` reads of each of `keys`, `limit` at a time, in the order given;
/// errors stay per key.
pub async fn load_all_with<Key, Load, Loading, Read>(
    keys: Vec<Key>,
    limit: usize,
    load: Load,
) -> Vec<(Key, Read)>
where
    Key: Clone + Send + 'static,
    Load: Fn(Key) -> Loading,
    Loading: Future<Output = Read> + Send + 'static,
    Read: Send + 'static,
{
    let limit = Arc::new(Semaphore::new(limit));
    let set: JoinSet<_> = keys
        .into_iter()
        .enumerate()
        .map(|(position, key)| {
            let limit = Arc::clone(&limit);
            let loading = load(key.clone());
            async move {
                // The semaphore is never closed, so a permit always comes.
                let _permit = limit.acquire_owned().await.ok();
                let read = loading.await;
                (position, key, read)
            }
        })
        .collect();
    set.join_all()
        .await
        .into_iter()
        .sorted_by_key(|(position, ..)| *position)
        .map(|(_, key, read)| (key, read))
        .collect()
}
