use std::sync::Arc;

use futures_util::{StreamExt as _, stream};

use crate::{
    command::{
        board::show::Loading,
        board_load::{self, RemoteBoard},
    },
    domain::board_text::{
        BoardLoads,
        BoardTextHits,
        BoardTextQuery,
        found_text,
        index::BoardIndex,
        split_loaded,
    },
    error::Result,
    remote::client::Client,
};

pub const LOAD_CONCURRENCY: usize = 6;

/// One board's index: cache if nothing newer is announced, else its socket.
///
/// # Errors
/// As [`crate::remote::socket::load::load`].
pub(in crate::command) async fn load_index(
    client: &Client,
    loading: Loading<'_>,
    board: &str,
) -> Result<BoardIndex> {
    let (_, index) = board_load::index(
        RemoteBoard {
            client,
            connector: loading.connector,
            board,
        },
        loading.cache,
        loading.cache_use,
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
    Loading: Future<Output = Result<Arc<BoardIndex>>>,
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
    Key: Clone,
    Load: Fn(Key) -> Loading,
    Loading: Future<Output = Read>,
{
    stream::iter(keys)
        .map(|key| {
            let loading = load(key.clone());
            async move { (key, loading.await) }
        })
        .buffered(limit)
        .collect()
        .await
}
