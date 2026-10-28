//! Boards kept between calls: memory, then the disk cache, then the server.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use holst_core::{
    command::{board_load, board_load::CacheUse},
    domain::{
        board::LoadedBoard,
        board_file::{compression, compression::Compression, document},
        board_text::index::BoardIndex,
    },
    effect::{board_write::LoadSource, cache::CachePort},
    remote::socket::write::connector,
};
use yrs::{Doc, ReadTxn as _, Transact as _};

use crate::{
    login::{HandedClient, when},
    memory::board_memory::{KeptBoard, KeptCopy},
    server::McpServer,
};

/// What reading a board again gave.
struct ReloadedBoard {
    loaded: LoadedBoard,
    source: LoadSource,
    downloaded_at: Option<Instant>,
    document: Option<Arc<[u8]>>,
}

impl McpServer {
    pub(crate) async fn kept_board(
        &self,
        client: &HandedClient,
        board: &str,
    ) -> holst_core::Result<KeptBoard> {
        let trusted = self
            .state
            .board(board)
            .and_then(|memory| memory.trusted().ok());
        let (served, previous) = match trusted.map(KeptCopy::verify) {
            Some(Ok(verified)) => (verified.into_whole(), None),
            Some(Err(seed)) => (None, seed.into_seed()),
            None => (None, None),
        };
        match (served, previous) {
            (Some(kept), _) => Ok(kept),
            (None, previous) => {
                let generation = self.state.generation();
                let cache = self.cache(client).await;
                let reloaded = match previous {
                    Some(previous) => {
                        from_memory(client, board, previous, cache.as_deref()).await
                    }
                    None => from_cache(client, board, cache.as_deref()).await,
                };
                let ReloadedBoard {
                    loaded,
                    source: fetched,
                    downloaded_at,
                    document: reused,
                } = self
                    .guarded(client, cache.as_deref(), board, reloaded)
                    .await?;
                let origin = self::downloaded_at(downloaded_at, fetched);
                let kept = kept(&loaded.document, board, origin, reused)?;
                self.state.store_board(board, kept.clone(), generation);
                Ok(kept)
            }
        }
    }

    async fn guarded<T>(
        &self,
        client: &HandedClient,
        cache: Option<&dyn CachePort>,
        board: &str,
        loaded: holst_core::Result<T>,
    ) -> holst_core::Result<T> {
        let (loaded, lost) =
            board_load::guarded_loss(client, cache, board, loaded).await;
        if lost {
            drop(self.state.take_board(board));
        }
        loaded
    }

    pub(crate) async fn kept_index(
        &self,
        client: &HandedClient,
        board: &str,
    ) -> holst_core::Result<Arc<BoardIndex>> {
        let trusted = self
            .state
            .board(board)
            .and_then(|memory| memory.trusted().ok());
        match trusted.map(KeptCopy::verify) {
            Some(Ok(verified)) => Ok(verified.into_index()),
            Some(Err(seed)) if seed.can_seed() => {
                Ok(self.kept_board(client, board).await?.index)
            }
            Some(Err(_)) | None => self.cached_index(client, board).await,
        }
    }

    async fn cached_index(
        &self,
        client: &HandedClient,
        board: &str,
    ) -> holst_core::Result<Arc<BoardIndex>> {
        match self.cache(client).await {
            None => Ok(self.kept_board(client, board).await?.index),
            Some(cache) => {
                let generation = self.state.generation();
                let read = board_load::index(
                    board_load::RemoteBoard {
                        client,
                        connector: connector(),
                        board,
                    },
                    Some(&*cache),
                    CacheUse::Use,
                )
                .await;
                let (_, index) = self.guarded(client, None, board, read).await?;
                let index = Arc::new(index);
                self.state
                    .store_index(board, Arc::clone(&index), generation);
                Ok(index)
            }
        }
    }
}

async fn from_memory(
    client: &HandedClient,
    board: &str,
    previous: KeptBoard,
    cache: Option<&dyn CachePort>,
) -> holst_core::Result<ReloadedBoard> {
    let seed = previous.document.as_deref().and_then(seed);
    let downloaded_at = seed.as_ref().map(|_| previous.downloaded_at);
    let (loaded, fetched, before) =
        board_load::seeded(client, connector(), board, seed).await?;
    let downloaded_at = downloaded_at.filter(|_| before.is_some());
    let unchanged = matches!(fetched, LoadSource::Skipped)
        && before
            .as_ref()
            .is_some_and(|before| *before == loaded.document.transact().state_vector());
    let reused = previous.document.filter(|_| unchanged);
    let following = cache.map(|cache| {
        board_load::remember(
            cache,
            board,
            board_load::LoadedCopy {
                document: &loaded.document,
                right: loaded.right,
                seed_state: before.as_ref(),
                source: fetched,
            },
        )
    });
    when(following).await;
    Ok(ReloadedBoard {
        loaded,
        source: fetched,
        downloaded_at,
        document: reused,
    })
}

async fn from_cache(
    client: &HandedClient,
    board: &str,
    cache: Option<&dyn CachePort>,
) -> holst_core::Result<ReloadedBoard> {
    let (loaded, fetched) = board_load::load(
        board_load::RemoteBoard {
            client,
            connector: connector(),
            board,
        },
        cache,
        CacheUse::Use,
    )
    .await?;
    let downloaded_at = cache_downloaded_at(cache, board, client.unix_millis());
    Ok(ReloadedBoard {
        loaded,
        source: fetched,
        downloaded_at,
        document: None,
    })
}

/// Returns when the disk cache's copy of `board` was last downloaded whole.
pub(super) fn cache_downloaded_at(
    cache: Option<&dyn CachePort>,
    board: &str,
    at_millis: u64,
) -> Option<Instant> {
    cache.and_then(|cache| cache.meta(board)).and_then(|meta| {
        Instant::now().checked_sub(Duration::from_millis(meta.age_millis(at_millis)))
    })
}

pub(super) fn seed(packed: &[u8]) -> Option<Doc> {
    document::decoded(&compression::uncompressed(packed).ok()?).ok()
}

pub(super) fn downloaded_at(
    downloaded_at: Option<Instant>,
    fetched: LoadSource,
) -> Instant {
    downloaded_at
        .filter(|_| matches!(fetched, LoadSource::Skipped))
        .unwrap_or_else(Instant::now)
}

pub(super) fn kept(
    document: &Doc,
    board: &str,
    downloaded: Instant,
    reused: Option<Arc<[u8]>>,
) -> holst_core::Result<KeptBoard> {
    let state = reused.map_or_else(
        || {
            compression::compressed(&document::encoded(document), Compression::Gzip)
                .map(|packed| Arc::<[u8]>::from(packed.as_ref()))
        },
        Ok,
    )?;
    Ok(KeptBoard {
        index: Arc::new(BoardIndex::from_document(document, board)),
        document: Some(state),
        downloaded_at: downloaded,
    })
}
