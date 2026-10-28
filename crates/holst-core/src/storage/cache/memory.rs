//! A board cache in memory with the eviction of the disk one: tests.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use crate::{
    effect::{
        cache::{
            CachePort,
            CacheRecord,
            CacheUsage,
            meta,
            meta::{CacheMeta, CopySizes, ReadCheck},
            plan,
            plan::{CachedBoard, Eviction},
        },
        clock::Clock,
        settings::DEFAULT_CACHE_TRUST_MILLIS,
    },
    error::Result,
};

#[derive(Debug, Clone)]
struct MemoryRecord {
    meta: CacheMeta,
    state: Option<Vec<u8>>,
    index: Option<Vec<u8>>,
}

/// The cache of one account in memory: the fake behind [`CachePort`].
#[derive(Clone)]
pub(crate) struct MemoryCache {
    boards: Arc<Mutex<BTreeMap<String, MemoryRecord>>>,
    max_bytes: u64,
    trust_millis: u64,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for MemoryCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Memory")
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}

impl MemoryCache {
    #[must_use]
    pub(crate) fn new(max_bytes: u64, clock: impl Clock + 'static) -> Self {
        Self {
            boards: Arc::default(),
            max_bytes,
            trust_millis: DEFAULT_CACHE_TRUST_MILLIS,
            clock: Arc::new(clock),
        }
    }

    /// The same cache trusting a full download for `trust_ms` milliseconds.
    #[must_use]
    pub(crate) fn with_trust(self, trust_ms: u64) -> Self {
        Self {
            trust_millis: trust_ms,
            ..self
        }
    }

    /// The same cache on another clock.
    #[must_use]
    pub(crate) fn with_clock(self, clock: impl Clock + 'static) -> Self {
        Self {
            clock: Arc::new(clock),
            ..self
        }
    }

    fn boards(&self) -> MutexGuard<'_, BTreeMap<String, MemoryRecord>> {
        self.boards.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn copies(boards: &BTreeMap<String, MemoryRecord>) -> Vec<CachedBoard> {
        boards
            .iter()
            .map(|(board, kept)| CachedBoard {
                board: board.clone(),
                last_used: kept.meta.last_used,
                state: kept.meta.sizes.document,
                rest: kept.meta.sizes.index,
            })
            .collect()
    }

    fn evict_in(
        &self,
        boards: &mut BTreeMap<String, MemoryRecord>,
        at: u64,
    ) -> CacheUsage {
        let copies = Self::copies(boards);
        let evictions = plan::evictions(&copies, self.max_bytes, at);
        let freed = crate::storage::cache::files::freed(&copies, &evictions);
        let evicted = |board: &str, part: fn(String) -> Eviction| {
            evictions.contains(&part(board.to_owned()))
        };
        boards.retain(|board, _| !evicted(board, Eviction::Board));
        boards
            .iter_mut()
            .filter(|(board, _)| evicted(board, Eviction::Document))
            .for_each(|(_, kept)| {
                kept.state = None;
                kept.meta.sizes.document = 0;
            });
        freed
    }
}

impl CachePort for MemoryCache {
    fn meta(&self, board: &str) -> Option<CacheMeta> {
        self.boards().get(board).map(|kept| kept.meta.clone())
    }

    fn trusts(&self, meta: &CacheMeta) -> bool {
        meta.is_trusted(self.clock.unix_millis(), self.trust_millis)
    }

    fn load_document(&self, board: &str) -> Option<Vec<u8>> {
        self.boards().get(board).and_then(|kept| kept.state.clone())
    }

    fn load_index(&self, board: &str) -> Option<Vec<u8>> {
        self.boards().get(board).and_then(|kept| kept.index.clone())
    }

    fn store(&self, board: &str, record: &CacheRecord<'_>) -> Result<()> {
        let at = self.clock.unix_millis();
        // imperative: the guard is mutated by the insert and the eviction
        let mut boards = self.boards();
        let previous = boards.get(board).map(|kept| kept.meta.clone());
        let meta = CacheMeta {
            format: meta::FORMAT,
            state_vector: meta::hex(record.state_vector),
            right: Some(record.right),
            downloaded_at: match (record.downloaded, &previous) {
                (false, Some(previous)) => previous.downloaded_at,
                (true, _) | (false, None) => at,
            },
            verified_at: match (record.verified, &previous) {
                (true, _) => at,
                (false, previous) => {
                    previous.as_ref().map_or(0, |meta| meta.verified_at)
                }
            },
            last_used: at,
            sizes: CopySizes {
                document: crate::storage::file::byte_len(record.document),
                index: record.index.map_or(0, crate::storage::file::byte_len),
            },
        };
        boards.insert(
            board.to_owned(),
            MemoryRecord {
                meta,
                state: Some(record.document.to_vec()),
                index: record.index.map(<[u8]>::to_vec),
            },
        );
        self.evict_in(&mut boards, at);
        Ok(())
    }

    fn touch(&self, board: &str, seen: ReadCheck) -> Result<()> {
        let at = self.clock.unix_millis();
        self.boards().get_mut(board).into_iter().for_each(|kept| {
            kept.meta
                .touched(at, seen)
                .into_iter()
                .for_each(|meta| kept.meta = meta);
        });
        Ok(())
    }

    fn discard(&self, board: &str) -> Result<()> {
        self.boards().remove(board);
        Ok(())
    }

    fn evict(&self) -> Result<CacheUsage> {
        let at = self.clock.unix_millis();
        Ok(self.evict_in(&mut self.boards(), at))
    }

    fn evictable(&self) -> CacheUsage {
        let boards = self.boards();
        let copies = Self::copies(&boards);
        let evictions =
            plan::evictions(&copies, self.max_bytes, self.clock.unix_millis());
        crate::storage::cache::files::freed(&copies, &evictions)
    }

    fn wipe(&self) -> Result<()> {
        self.boards().clear();
        Ok(())
    }

    fn usage(&self) -> CacheUsage {
        let boards = self.boards();
        CacheUsage {
            boards: boards.len(),
            bytes: Self::copies(&boards).iter().map(CachedBoard::bytes).sum(),
        }
    }

    fn shared(&self) -> Arc<dyn CachePort> {
        Arc::new(self.clone())
    }
}
