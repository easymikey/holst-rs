//! State between calls: workspaces, trees and boards (dropped narrowly), `@N`.

#![expect(
    clippy::disallowed_methods,
    reason = "entries are stamped with Instant::now(); a clock threaded through every store and get is not worth it yet"
)]

pub(crate) mod alias;
pub(crate) mod board_memory;
pub(crate) mod invalidation;
#[cfg(test)] mod tests;
pub(crate) mod tree_memory;

use std::{
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use arc_swap::ArcSwapOption;
use holst_core::{
    domain::{account::Workspace, tree::name_index::build::NameIndex},
    effect::cache::trust::Unverified,
};
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
    tree_memory::{
        KeptTree,
        KeptWorkspaces,
        TREE_MEMORY_BYTES,
        TreeMemory,
        heap_weight,
    },
};

const WORKSPACES_TRUST_WINDOW: Duration = Duration::from_mins(5);

#[derive(Default)]
pub(crate) struct McpMemory {
    generation: AtomicU64,
    workspaces_floor: AtomicU64,
    workspaces: ArcSwapOption<KeptWorkspaces>,
    trees: Mutex<TreeMemory>,
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
    pub(crate) fn workspaces(&self) -> Option<Arc<[Workspace]>> {
        let floor = self.workspaces_floor.load(Ordering::Acquire);
        self.workspaces
            .load()
            .as_ref()
            .filter(|w| {
                w.generation >= floor && w.at.elapsed() < WORKSPACES_TRUST_WINDOW
            })
            .map(|w| Arc::clone(&w.list))
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn next_generation(&self) -> u64 {
        self.generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    }

    pub(crate) fn store_workspaces(
        &self,
        list: Vec<Workspace>,
        generation: u64,
    ) -> Arc<[Workspace]> {
        let list = Arc::<[Workspace]>::from(list);
        // A reset meanwhile is harmless: the entry keeps the old generation.
        let floor = self.workspaces_floor.load(Ordering::Acquire);
        (generation >= floor).then(|| {
            self.workspaces.store(Some(Arc::new(KeptWorkspaces {
                generation,
                at: Instant::now(),
                list: Arc::clone(&list),
            })));
        });
        list
    }

    pub(crate) fn tree(&self, workspace_id: &str) -> Option<Arc<NameIndex>> {
        lock(&self.trees).get(workspace_id)
    }

    pub(crate) fn store_tree(
        &self,
        workspace_id: &str,
        index: NameIndex,
        generation: u64,
    ) -> Arc<NameIndex> {
        let weight = heap_weight(&index);
        let index = Arc::new(index);
        let tree = KeptTree {
            generation,
            at: Instant::now(),
            used: 0,
            weight,
            index: Arc::clone(&index),
        };
        // Under the lock the floors move with: a stale tree never stays.
        lock(&self.trees).insert(workspace_id, tree, TREE_MEMORY_BYTES);
        index
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
