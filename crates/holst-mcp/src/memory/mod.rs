//! State between calls: boards (dropped narrowly) and `@N`.

#![expect(
    clippy::disallowed_methods,
    reason = "entries are stamped with Instant::now(); a clock threaded through every store and get is not worth it yet"
)]

pub(crate) mod alias;
pub(crate) mod board_memory;
pub(crate) mod invalidation;
#[cfg(test)] mod tests;

use std::{
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use holst_core::effect::cache::trust::Unverified;
use itertools::Itertools as _;

use crate::memory::{
    alias::{Aliases, UnknownAlias, alias_text},
    board_memory::{
        BOARD_MEMORY_BYTES,
        BoardMemory,
        BoardRead,
        DownloadTrust,
        KeptBoard,
        KeptCopy,
    },
};

#[derive(Default)]
pub(crate) struct McpMemory {
    generation: AtomicU64,
    boards: Mutex<BoardMemory>,
    aliases: Aliases,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The ids to evict, least recently used first, until `total` fits `capacity`;
/// never `keep`.
fn least_used<'a>(
    entries: impl Iterator<Item = (&'a String, u64, u64)>,
    keep: &str,
    total: u64,
    capacity: u64,
) -> Vec<String> {
    entries
        .filter(|&(id, ..)| id != keep)
        .sorted_unstable_by_key(|&(_, used, _)| used)
        .scan(total, |total, (id, _, weight)| {
            (*total > capacity).then(|| {
                *total = total.saturating_sub(weight);
                id.clone()
            })
        })
        .collect()
}

impl McpMemory {
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn next_generation(&self) -> u64 {
        self.generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    }

    pub(crate) fn with_download_trust(download_trust: DownloadTrust) -> Self {
        Self {
            boards: Mutex::new(BoardMemory::with_download_trust(download_trust)),
            ..Self::default()
        }
    }

    pub(crate) fn board(&self, board: &str) -> Option<KeptCopy<Unverified>> {
        lock(&self.boards).get(board, Instant::now())
    }

    pub(crate) fn store_board(&self, board: &str, kept: KeptBoard, generation: u64) {
        let read = BoardRead {
            kept,
            generation,
            verified_at: Some(Instant::now()),
        };
        lock(&self.boards).insert(board, read, BOARD_MEMORY_BYTES);
    }

    /// Stores a board the server has not confirmed lately, which ten minutes
    /// of waiting would make.
    #[cfg(test)]
    pub(crate) fn store_unconfirmed_board(
        &self,
        board: &str,
        kept: KeptBoard,
        generation: u64,
    ) {
        let read = BoardRead {
            kept,
            generation,
            verified_at: None,
        };
        lock(&self.boards).insert(board, read, BOARD_MEMORY_BYTES);
    }

    pub(crate) fn store_index(
        &self,
        board: &str,
        index: Arc<holst_core::domain::board_text::index::BoardIndex>,
        generation: u64,
    ) {
        let read = BoardRead {
            kept: KeptBoard {
                index,
                document: None,
                downloaded_at: Instant::now(),
            },
            generation,
            verified_at: Some(Instant::now()),
        };
        lock(&self.boards).insert(board, read, BOARD_MEMORY_BYTES);
    }

    pub(crate) fn take_board(&self, board: &str) -> Option<KeptBoard> {
        let generation = self.next_generation();
        lock(&self.boards).forget(board, generation)
    }

    pub(crate) fn alias(&self, id: &str) -> String {
        let n = self.aliases.intern(id);
        format!("@{n}")
    }

    pub(crate) fn expand(&self, reference: &str) -> Result<String, UnknownAlias> {
        let alias = reference.trim();
        holst_core::domain::lookup::rules::is_alias(alias)
            .then(|| {
                alias
                    .strip_prefix('@')
                    .and_then(|n| n.parse::<usize>().ok())
            })
            .flatten()
            .map_or_else(
                || Ok(reference.to_string()),
                |n| {
                    self.aliases.id(n).ok_or_else(|| UnknownAlias {
                        alias: alias.to_owned(),
                    })
                },
            )
    }

    pub(crate) fn alias_text(&self, text: &str) -> String {
        alias_text(text, |id| self.alias(id))
    }
}
