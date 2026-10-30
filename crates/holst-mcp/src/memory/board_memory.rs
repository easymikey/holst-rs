//! Boards between calls: objects and the packed document, LRU by bytes.

use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};

use holst_core::{
    domain::board_text::index::BoardIndex,
    effect::{
        cache::trust::{StaleCopy, Trusted, Unverified, Verified},
        settings::DEFAULT_CACHE_TRUST_MILLIS,
    },
};

use super::least_used;

pub(crate) const BOARD_MEMORY_BYTES: u64 = 128 << 20;

const VERIFIED_TRUST: Duration = Duration::from_mins(10);

#[derive(Debug, Clone)]
pub(crate) struct KeptBoard {
    pub(crate) index: Arc<BoardIndex>,
    pub(crate) document: Option<Arc<[u8]>>,
    pub(crate) downloaded_at: Instant,
}

#[derive(Debug)]
pub(crate) struct BoardRead {
    pub(crate) kept: KeptBoard,
    pub(crate) generation: u64,
    pub(crate) verified_at: Option<Instant>,
}

/// A kept board read from memory in the trust state `Trust`.
///
/// ```text
/// get ─> Unverified ─trusted─> Trusted ─verify─> Verified ─> served as kept
///              │                   └─into_seed─> a load that checks it
///              └─> Stale::Expired: loaded again without a seed
/// ```
#[derive(Debug)]
pub(crate) struct KeptCopy<Trust> {
    kept: KeptBoard,
    verified_at: Option<Instant>,
    read_at: Instant,
    download_trust: DownloadTrust,
    trust: PhantomData<Trust>,
}

impl<Trust> KeptCopy<Trust> {
    fn into_state<Next>(self) -> KeptCopy<Next> {
        KeptCopy {
            kept: self.kept,
            verified_at: self.verified_at,
            read_at: self.read_at,
            download_trust: self.download_trust,
            trust: PhantomData,
        }
    }
}

impl KeptCopy<Unverified> {
    /// Returns the copy as a seed while its full download is in the trust
    /// window.
    ///
    /// # Errors
    /// [`StaleCopy::Expired`] past the window.
    pub(crate) fn trusted(self) -> Result<KeptCopy<Trusted>, StaleCopy> {
        self.download_trust
            .can_seed(&self.kept, self.read_at)
            .then(|| self.into_state())
            .ok_or(StaleCopy::Expired)
    }
}

impl KeptCopy<Trusted> {
    /// Returns the copy as verified while the server confirmed it within
    /// [`VERIFIED_TRUST`].
    ///
    /// # Errors
    /// The seed back when older or never confirmed.
    pub(crate) fn verify(self) -> Result<KeptCopy<Verified>, Self> {
        match self.verified_at {
            Some(at) if self.read_at.saturating_duration_since(at) < VERIFIED_TRUST => {
                Ok(self.into_state())
            }
            Some(_) | None => Err(self),
        }
    }

    pub(crate) const fn can_seed(&self) -> bool {
        self.kept.document.is_some()
    }

    pub(crate) fn into_seed(self) -> Option<KeptBoard> {
        self.can_seed().then_some(self.kept)
    }
}

impl KeptCopy<Verified> {
    pub(crate) fn into_whole(self) -> Option<KeptBoard> {
        self.kept.document.is_some().then_some(self.kept)
    }

    pub(crate) fn into_index(self) -> Arc<BoardIndex> {
        self.kept.index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DownloadTrust(pub(crate) Duration);

impl Default for DownloadTrust {
    fn default() -> Self {
        Self(Duration::from_millis(DEFAULT_CACHE_TRUST_MILLIS))
    }
}

impl DownloadTrust {
    pub(crate) fn can_seed(self, kept: &KeptBoard, read_at: Instant) -> bool {
        read_at.saturating_duration_since(kept.downloaded_at) < self.0
    }
}

#[derive(Debug)]
struct MemoryEntry {
    kept: KeptBoard,
    generation: u64,
    verified_at: Option<Instant>,
    used: u64,
    weight: u64,
}

#[derive(Debug, Default)]
pub(crate) struct BoardMemory {
    boards: HashMap<String, MemoryEntry>,
    total: u64,
    tick: u64,
    floor: u64,
    floors: HashMap<String, u64>,
    download_trust: DownloadTrust,
}

impl BoardMemory {
    pub(crate) fn with_download_trust(download_trust: DownloadTrust) -> Self {
        Self {
            download_trust,
            ..Self::default()
        }
    }

    fn floor(&self, board: &str) -> u64 {
        self.floors
            .get(board)
            .copied()
            .unwrap_or_default()
            .max(self.floor)
    }

    pub(crate) fn get(
        &mut self,
        board: &str,
        read_at: Instant,
    ) -> Option<KeptCopy<Unverified>> {
        let floor = self.floor(board);
        self.tick = self.tick.saturating_add(1);
        let tick = self.tick;
        let download_trust = self.download_trust;
        self.boards
            .get_mut(board)
            .filter(|entry| entry.generation >= floor)
            .map(|entry| {
                entry.used = tick;
                KeptCopy {
                    kept: entry.kept.clone(),
                    verified_at: entry.verified_at,
                    read_at,
                    download_trust,
                    trust: PhantomData,
                }
            })
    }

    pub(crate) fn insert(&mut self, board: &str, read: BoardRead, capacity: u64) {
        let BoardRead {
            kept,
            generation,
            verified_at: at,
        } = read;
        let current = generation >= self.floor(board);
        self.tick = self.tick.saturating_add(1);
        let entry = MemoryEntry {
            weight: weight(&kept),
            kept,
            generation,
            verified_at: at,
            used: self.tick,
        };
        if current {
            drop(self.remove(board));
            if entry.weight <= capacity {
                self.total = self.total.saturating_add(entry.weight);
                self.boards.insert(board.to_owned(), entry);
            }
        }
        self.evict(board, capacity);
    }

    fn remove(&mut self, board: &str) -> Option<KeptBoard> {
        self.boards.remove(board).map(|entry| {
            self.total = self.total.saturating_sub(entry.weight);
            entry.kept
        })
    }

    pub(crate) fn forget(&mut self, board: &str, generation: u64) -> Option<KeptBoard> {
        self.floors.insert(board.to_owned(), generation);
        self.remove(board)
    }

    pub(crate) fn forget_all(&mut self, generation: u64) {
        self.boards.clear();
        self.floors.clear();
        self.total = 0;
        self.floor = generation;
    }

    #[cfg(test)]
    const fn total(&self) -> u64 {
        self.total
    }

    fn evict(&mut self, keep: &str, capacity: u64) {
        let entries = self
            .boards
            .iter()
            .map(|(id, entry)| (id, entry.used, entry.weight));
        least_used(entries, keep, self.total, capacity)
            .into_iter()
            .map(|id| self.remove(&id))
            .for_each(drop);
    }
}

const INDEX_BYTES_PER_OBJECT: usize = 96;

fn weight(kept: &KeptBoard) -> u64 {
    let index = &kept.index;
    let texts: usize = (0..index.len()).map(|at| index.text(at).len()).sum();
    let columns = index.len().saturating_mul(INDEX_BYTES_PER_OBJECT);
    u64::try_from(
        kept.document
            .as_ref()
            .map_or(0, |state| state.len())
            .saturating_add(columns)
            .saturating_add(texts),
    )
    .unwrap_or(u64::MAX)
    .max(1)
}

#[cfg(test)]
mod tests {

    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use holst_core::{
        domain::board_text::index::BoardIndex,
        effect::cache::trust::StaleCopy,
    };
    use itertools::Itertools as _;

    use crate::memory::board_memory::{
        BoardMemory,
        BoardRead,
        DownloadTrust,
        KeptBoard,
        KeptCopy,
        VERIFIED_TRUST,
    };

    fn kept(bytes: usize) -> KeptBoard {
        KeptBoard {
            index: Arc::new(BoardIndex::from_objects(&[], "b", Vec::new())),
            document: Some(Arc::from(vec![0; bytes])),
            downloaded_at: Instant::now(),
        }
    }

    fn ids(boards: &BoardMemory) -> Vec<&str> {
        boards
            .boards
            .keys()
            .map(String::as_str)
            .sorted_unstable()
            .collect()
    }

    fn read(kept: KeptBoard, generation: u64, at: Option<Instant>) -> BoardRead {
        BoardRead {
            kept,
            generation,
            verified_at: at,
        }
    }

    #[test]
    fn the_least_recently_read_goes_over_the_limit() {
        let now = Instant::now();
        // imperative: the memory is changed through `&mut self` methods.
        let mut boards = BoardMemory::default();
        boards.insert("a", read(kept(40), 0, Some(now)), 100);
        boards.insert("b", read(kept(40), 0, Some(now)), 100);
        assert!(boards.get("a", now).is_some());
        boards.insert("c", read(kept(40), 0, Some(now)), 100);
        assert_eq!(ids(&boards), ["a", "c"]);
        assert!(boards.total() <= 100);
        boards.insert("huge", read(kept(101), 0, Some(now)), 100);
        assert_eq!(
            ids(&boards),
            ["a", "c"],
            "a board over the limit is not kept"
        );
    }

    fn use_at(boards: &mut BoardMemory, read_at: Instant) -> Option<&'static str> {
        let read = boards.get("a", read_at)?;
        Some(match read.trusted().map(KeptCopy::verify) {
            Ok(Ok(verified)) => verified.into_whole().map_or("index", |_| "served"),
            Ok(Err(seed)) => seed.into_seed().map_or("disk", |_| "seed"),
            Err(StaleCopy::Expired) => "expired",
            Err(StaleCopy::Corrupt | StaleCopy::Ahead | StaleCopy::Behind) => "other",
        })
    }

    #[test]
    fn trust_expires_into_a_check() {
        let at = Instant::now();
        // imperative: the memory is changed through `&mut self` methods.
        let mut boards = BoardMemory::default();
        boards.insert("a", read(kept(1), 0, Some(at)), 100);
        assert_eq!(use_at(&mut boards, at), Some("served"));
        assert_eq!(use_at(&mut boards, at + VERIFIED_TRUST), Some("seed"));
        boards.insert("a", read(kept(1), 0, None), 100);
        assert_eq!(use_at(&mut boards, at), Some("seed"), "a seed is checked");
    }

    #[test]
    fn a_stale_board_is_reloaded() {
        let at = Instant::now();
        let later = at + VERIFIED_TRUST;
        // imperative: the memory is changed through `&mut self` methods.
        let mut boards = BoardMemory::default();
        boards.insert("a", read(kept(1), 0, Some(at)), 100);
        let seed = boards
            .get("a", later)
            .and_then(|read| read.trusted().ok())
            .map(KeptCopy::verify);
        let reloaded = seed.and_then(Result::err).and_then(KeptCopy::into_seed);
        assert!(reloaded.is_some(), "a stale copy seeds no reload");
        boards.insert("a", read(kept(1), 0, Some(later)), 100);
        assert_eq!(use_at(&mut boards, later), Some("served"));
    }

    #[test]
    fn an_index_alone_is_no_board() {
        let at = Instant::now();
        // imperative: the memory is changed through `&mut self` methods.
        let mut boards = BoardMemory::default();
        let alone = KeptBoard {
            document: None,
            ..kept(1)
        };
        boards.insert("a", read(alone, 0, Some(at)), 100);
        assert_eq!(use_at(&mut boards, at), Some("index"));
        assert_eq!(use_at(&mut boards, at + VERIFIED_TRUST), Some("disk"));
    }

    #[test]
    fn an_old_download_expires_the_seed() {
        let at = Instant::now();
        let trust = Duration::from_mins(1);
        // imperative: the memory is changed through `&mut self` methods.
        let mut boards = BoardMemory::with_download_trust(DownloadTrust(trust));
        let old = KeptBoard {
            downloaded_at: at,
            ..kept(1)
        };
        boards.insert("a", read(old.clone(), 0, Some(at + trust)), 100);
        let before = at + Duration::from_secs(59);
        assert_eq!(use_at(&mut boards, before), Some("served"));
        assert_eq!(use_at(&mut boards, at + trust), Some("expired"));
        assert!(DownloadTrust(trust).can_seed(&old, before));
        assert!(
            !DownloadTrust(Duration::ZERO).can_seed(&old, at),
            "0 never seeds"
        );
    }

    #[test]
    fn a_read_older_than_a_write_is_dropped() {
        let now = Instant::now();
        // imperative: the memory is changed through `&mut self` methods.
        let mut boards = BoardMemory::default();
        boards.forget("a", 5);
        boards.insert("a", read(kept(1), 4, Some(now)), 100);
        assert!(boards.get("a", now).is_none());
        boards.insert("a", read(kept(1), 5, Some(now)), 100);
        assert!(boards.get("a", now).is_some());
        boards.forget_all(6);
        assert!(boards.get("a", now).is_none());
        assert_eq!(boards.total(), 0);
    }
}
