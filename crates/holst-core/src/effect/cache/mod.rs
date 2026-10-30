pub mod meta;
pub mod plan;
pub mod trust;

use std::{iter::Sum, sync::Arc};

use crate::{
    domain::{account::Workspace, board::EditRight},
    effect::cache::meta::{CacheMeta, ReadCheck},
    error::Result,
};

/// What a store keeps of one board.
#[derive(Debug, Clone, Copy)]
pub struct CacheRecord<'a> {
    pub document: &'a [u8],
    pub state_vector: &'a [u8],
    pub index: Option<&'a [u8]>,
    pub right: EditRight,
    pub downloaded: bool,
    pub verified: bool,
}

/// How many boards and bytes a cache holds or frees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct CacheUsage {
    pub boards: usize,
    pub bytes: u64,
}

impl CacheUsage {
    /// Returns both totals together.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        Self {
            boards: self.boards.saturating_add(other.boards),
            bytes: self.bytes.saturating_add(other.bytes),
        }
    }
}

impl Sum for CacheUsage {
    fn sum<Parts: Iterator<Item = Self>>(parts: Parts) -> Self {
        parts.fold(Self::default(), Self::combine)
    }
}

/// A board cache of one account: what reads and writes need of it.
pub trait CachePort: Send + Sync {
    /// What the cache knows about a board; `None` when it is not cached.
    fn meta(&self, board: &str) -> Option<CacheMeta>;
    /// Whether the entry is recent enough to seed a load that skips the link.
    fn trusts(&self, meta: &CacheMeta) -> bool;
    /// The cached document as a whole Yjs v2 update.
    fn load_document(&self, board: &str) -> Option<Vec<u8>>;
    /// The cached board index, as its codec wrote it.
    fn load_index(&self, board: &str) -> Option<Vec<u8>>;
    /// Keep a board's document, then evict what is over the limit.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::FileWrite`],
    /// [`crate::error::local_data::LocalDataError::CacheBusy`].
    fn store(&self, board: &str, record: &CacheRecord<'_>) -> Result<()>;
    /// The board was read as `seen` says; rewritten at most hourly.
    ///
    /// # Errors
    /// As [`CachePort::store`].
    fn touch(&self, board: &str, seen: ReadCheck) -> Result<()>;
    /// Drop one board: its files do not decode, or the user lost access.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::FileRemove`].
    fn discard(&self, board: &str) -> Result<()>;
    /// Remove what is unused for a month or over the limit; what went.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::CacheBusy`] or a lock file
    /// that cannot be made.
    fn evict(&self) -> Result<CacheUsage>;
    /// What [`CachePort::evict`] would remove now.
    fn evictable(&self) -> CacheUsage;
    /// Remove every cached board of the account.
    ///
    /// # Errors
    /// As [`CachePort::discard`].
    fn wipe(&self) -> Result<()>;
    /// What the cache holds.
    fn usage(&self) -> CacheUsage;
    /// The same cache as a handle that may move to another thread.
    fn shared(&self) -> Arc<dyn CachePort>;
    /// Returns the cached bytes of a stored file by its storage name.
    fn load_asset(&self, _name: &str) -> Option<Vec<u8>> {
        None
    }
    /// Keeps stored files' bytes under one lock, then evicts stored files over
    /// their share once.
    ///
    /// # Errors
    /// As [`CachePort::store`]; the first failed write, the others are tried.
    fn store_assets(&self, _assets: &[(&str, &[u8])]) -> Result<()> {
        Ok(())
    }
}

/// A workspace's `tree` reply as it was kept, and when it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedTree {
    pub text: String,
    /// Unix milliseconds.
    pub read_at: u64,
}

/// What [`AccountCache::forget`] drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forget<'a> {
    Workspaces,
    Tree(&'a str),
    Trees,
    All,
}

/// The workspace list and trees of accounts, kept until logout.
pub trait AccountCache: Send + Sync {
    /// The kept workspace list; `None` when none is kept.
    fn workspaces(&self, account: &str) -> Option<Vec<Workspace>>;
    /// Keep the workspace list, replacing the old one.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::FileWrite`].
    fn keep_workspaces(&self, account: &str, workspaces: &[Workspace]) -> Result<()>;
    /// The kept tree of a workspace; `None` when none is kept.
    fn tree(&self, account: &str, workspace: &str) -> Option<CachedTree>;
    /// Keep a workspace's tree reply and when it was read.
    ///
    /// # Errors
    /// As [`AccountCache::keep_workspaces`].
    fn keep_tree(
        &self,
        account: &str,
        workspace: &str,
        text: &str,
        read_at: u64,
    ) -> Result<()>;
    /// Drop what `what` names.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::FileRemove`].
    fn forget(&self, account: &str, what: Forget<'_>) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use crate::effect::cache::CacheUsage;

    #[test]
    fn parts_combine_and_saturate() {
        let parts = [
            CacheUsage {
                boards: 1,
                bytes: 10,
            },
            CacheUsage {
                boards: 2,
                bytes: u64::MAX,
            },
        ];
        assert_eq!(
            parts.into_iter().sum::<CacheUsage>(),
            CacheUsage {
                boards: 3,
                bytes: u64::MAX
            }
        );
    }
}
