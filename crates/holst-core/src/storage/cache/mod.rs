//! The board cache on disk: per server, per account, per board, size-capped.

pub mod account;
pub mod disk;
pub mod files;
#[cfg(test)] pub(crate) mod memory;
#[cfg(test)] pub(crate) mod tests;

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    domain::id::is_uuid,
    effect::{
        cache::{CachePort, CacheUsage},
        clock::{Clock, FixedClock},
        settings::CacheSettings,
    },
    error::Result,
    storage::{
        cache::files::{LAYOUT, is_host_key, is_layout, layout},
        file::{name_of, remove_dir, subdirs},
        lock::LOCK_WAIT_MILLIS,
    },
};

const STATE_FILE: &str = "state.ydoc.gz";
const INDEX_FILE: &str = "index.bin";
const META_FILE: &str = "meta.json";
const TALLY_FILE: &str = "tally.json";
const BOARDS_DIR: &str = "boards";
const ASSETS_DIR: &str = "assets";
const WORKSPACES_FILE: &str = "workspaces.json";
const TREES_DIR: &str = "trees";

/// The board cache of one account on disk.
#[derive(Clone)]
pub struct Cache {
    root: PathBuf,
    dir: PathBuf,
    account_id: String,
    max_bytes: u64,
    trust_millis: u64,
    lock_wait_millis: u64,
    clock: Arc<dyn Clock>,
    #[cfg(test)]
    scans: Arc<std::sync::atomic::AtomicUsize>,
}

impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cache")
            .field("dir", &self.dir)
            .field("max_bytes", &self.max_bytes)
            .field("trust_ms", &self.trust_millis)
            .finish_non_exhaustive()
    }
}

impl Cache {
    /// The cache of account `account_id` on the settings' server; `None` when
    /// the cache is off or `account_id` is not a UUID.
    #[must_use]
    pub fn from_settings(
        settings: &CacheSettings,
        account_id: &str,
        clock: impl Clock + 'static,
    ) -> Option<Self> {
        let root = settings.root.as_deref()?;
        Self::at(
            root,
            &settings.host,
            account_id,
            (settings, Arc::new(clock)),
        )
    }

    fn at(
        root: &Path,
        host: &str,
        account_id: &str,
        (settings, clock): (&CacheSettings, Arc<dyn Clock>),
    ) -> Option<Self> {
        let account_id = account_id.to_ascii_lowercase();
        (is_uuid(&account_id) && is_host_key(host)).then(|| Self {
            dir: layout(root).join(host).join(&account_id),
            root: root.to_path_buf(),
            account_id,
            max_bytes: settings.max_bytes,
            trust_millis: settings.trust_millis,
            lock_wait_millis: LOCK_WAIT_MILLIS,
            clock,
            #[cfg(test)]
            scans: Arc::default(),
        })
    }

    /// The caches of every account on every server under every known root.
    #[must_use]
    pub fn accounts(settings: &CacheSettings, clock: &Arc<dyn Clock>) -> Vec<Self> {
        settings
            .roots
            .iter()
            .flat_map(|root| {
                subdirs(&layout(root)).into_iter().flat_map(move |host| {
                    let clock = Arc::clone(clock);
                    subdirs(&host).into_iter().filter_map(move |account_id| {
                        Self::at(
                            root,
                            name_of(&host)?,
                            name_of(&account_id)?,
                            (settings, Arc::clone(&clock)),
                        )
                    })
                })
            })
            .collect()
    }

    /// Wipe `account_id`'s caches on every server and root, every account's
    /// when `account_id` is unknown, and every other layout; what was
    /// wiped.
    ///
    /// # Errors
    /// The first removal that failed; the others are tried still.
    pub fn forget(
        settings: &CacheSettings,
        account_id: Option<&str>,
    ) -> Result<CacheUsage> {
        let account_id = account_id.map(str::to_ascii_lowercase);
        let clock: Arc<dyn Clock> = Arc::new(FixedClock { unix_millis: 0 });
        let chosen: Vec<Self> = Self::accounts(settings, &clock)
            .into_iter()
            .filter(|cache| {
                account_id
                    .as_deref()
                    .is_none_or(|account_id| account_id == cache.account_id)
            })
            .collect();
        let wiped = chosen.iter().map(CachePort::usage).sum::<CacheUsage>();
        let old = settings
            .roots
            .iter()
            .flat_map(|root| Self::other_layouts(root));
        let failed = chosen
            .iter()
            .map(CachePort::wipe)
            .chain(old.map(|dir| remove_dir(&dir)))
            .filter_map(Result::err)
            .reduce(|first, _| first);
        failed.map_or(Ok(wiped), Err)
    }

    /// Directories of other layouts under `root`: left by other versions.
    #[must_use]
    pub fn other_layouts(root: &Path) -> Vec<PathBuf> {
        subdirs(root)
            .into_iter()
            .filter(|path| {
                name_of(path).is_some_and(|name| is_layout(name) && name != LAYOUT)
            })
            .collect()
    }

    /// The same cache on another clock: tests.
    #[cfg(test)]
    #[must_use]
    pub fn with_clock(self, clock: impl Clock + 'static) -> Self {
        Self {
            clock: Arc::new(clock),
            ..self
        }
    }

    /// The same cache waiting `lock_wait_ms` for another process's lock.
    #[cfg(test)]
    #[must_use]
    pub fn with_lock_wait(self, lock_wait_ms: u64) -> Self {
        Self {
            lock_wait_millis: lock_wait_ms,
            ..self
        }
    }

    /// The account's directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The account's id.
    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// This cache as the port loads and changes go through.
    #[must_use]
    pub fn as_port(&self) -> &dyn CachePort {
        self
    }

    /// This cache as a port shared by loads running at once.
    #[must_use]
    pub fn into_shared(self) -> Arc<dyn CachePort> {
        Arc::new(self)
    }

    fn board_dir(&self, board: &str) -> Option<PathBuf> {
        let board = board.to_ascii_lowercase();
        is_uuid(&board).then(|| self.dir.join(BOARDS_DIR).join(board))
    }
}
