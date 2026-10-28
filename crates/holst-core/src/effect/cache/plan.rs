//! What the board cache drops: the eviction plan; pure.

use std::cmp::Ordering;

use itertools::Itertools;

pub const UNUSED_MILLIS: u64 = 30 * 24 * 60 * 60 * 1000;

/// One board on disk as eviction sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBoard {
    pub board: String,
    pub last_used: u64,
    pub state: u64,
    pub rest: u64,
}

impl CachedBoard {
    /// All its bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.state.saturating_add(self.rest)
    }
}

/// What eviction removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eviction {
    Document(String),
    Board(String),
}

/// What to remove so `copies` fits in `max_bytes` bytes at `at`.
#[must_use]
pub fn evictions(copies: &[CachedBoard], max_bytes: u64, at: u64) -> Vec<Eviction> {
    let (expired, kept): (Vec<&CachedBoard>, Vec<&CachedBoard>) = copies
        .iter()
        .sorted_unstable_by(|a, b| least_recent(a, b))
        .partition(|entry| at.saturating_sub(entry.last_used) >= UNUSED_MILLIS);
    let total: u64 = kept.iter().map(|entry| entry.bytes()).sum();
    let (states, total) = shed(&kept, total, max_bytes, |entry| entry.state);
    let (boards, _) = shed(&kept, total, max_bytes, |entry| entry.rest);
    expired
        .iter()
        .map(|entry| Eviction::Board(entry.board.clone()))
        .chain(states.into_iter().map(Eviction::Document))
        .chain(boards.into_iter().map(Eviction::Board))
        .collect()
}

pub const ASSET_SHARE: u64 = 4;

const ASSET_KEY_MAX: usize = 200;

/// One stored file on disk as eviction sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAsset {
    pub key: String,
    pub last_used: u64,
    pub bytes: u64,
}

/// Returns the stored files to remove: those unused for [`UNUSED_MILLIS`], then
/// the least recently used until the rest fits in `max_bytes` bytes.
#[must_use]
pub fn asset_evictions(copies: &[CachedAsset], max_bytes: u64, at: u64) -> Vec<String> {
    copies
        .iter()
        .sorted_unstable_by(|a, b| {
            b.last_used
                .cmp(&a.last_used)
                .then_with(|| a.key.cmp(&b.key))
        })
        .scan(0_u64, |kept, asset| {
            let total = kept.saturating_add(asset.bytes);
            let gone = total > max_bytes
                || at.saturating_sub(asset.last_used) >= UNUSED_MILLIS;
            *kept = kept.saturating_add(asset.bytes.saturating_mul(u64::from(!gone)));
            Some((asset, gone))
        })
        .filter(|(_, gone)| *gone)
        .map(|(asset, _)| asset.key.clone())
        .collect()
}

/// Returns the cache file name of a stored file: letters, digits, `-`, `_` and
/// inner dots kept, the rest `%XX`; none when empty or too long.
///
/// ```
/// use holst_core::effect::cache::plan::asset_key;
///
/// assert_eq!(asset_key("a1.png").as_deref(), Some("a1.png"));
/// assert_eq!(
///     asset_key("stickers/x.png").as_deref(),
///     Some("stickers%2Fx.png")
/// );
/// assert_eq!(
///     asset_key(".holst-1-2.partial").as_deref(),
///     Some("%2Eholst-1-2.partial")
/// );
/// assert_eq!(asset_key(""), None);
/// ```
#[must_use]
pub fn asset_key(name: &str) -> Option<String> {
    let key: String = name
        .bytes()
        .enumerate()
        .map(|(at, byte)| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => {
                char::from(byte).to_string()
            }
            b'.' if at > 0 => ".".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect();
    Some(key).filter(|key| !key.is_empty() && key.len() <= ASSET_KEY_MAX)
}

fn least_recent(a: &CachedBoard, b: &CachedBoard) -> Ordering {
    a.last_used
        .cmp(&b.last_used)
        .then_with(|| a.board.cmp(&b.board))
}

fn shed(
    by_use: &[&CachedBoard],
    total: u64,
    max_bytes: u64,
    part: impl Fn(&CachedBoard) -> u64,
) -> (Vec<String>, u64) {
    by_use.iter().filter(|entry| part(entry) > 0).fold(
        (Vec::new(), total),
        |(mut gone, total), entry| {
            let over = total > max_bytes;
            gone.extend(over.then(|| entry.board.clone()));
            (gone, total.saturating_sub(part(entry) * u64::from(over)))
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::{
        collection::vec,
        prelude::{prop_assert, proptest},
    };

    use crate::{
        effect::cache::plan::{
            CachedAsset,
            CachedBoard,
            Eviction,
            UNUSED_MILLIS,
            asset_evictions,
            evictions,
        },
        laws::config,
    };

    fn held(board: &str, last_used: u64, state: u64, rest: u64) -> CachedBoard {
        CachedBoard {
            board: board.to_owned(),
            last_used,
            state,
            rest,
        }
    }

    fn left(held: &[CachedBoard], evicted: &[Eviction]) -> u64 {
        held.iter()
            .map(|entry| {
                let whole = evicted.contains(&Eviction::Board(entry.board.clone()));
                let state = evicted.contains(&Eviction::Document(entry.board.clone()));
                whole
                    .then_some(0)
                    .or_else(|| state.then_some(entry.rest))
                    .unwrap_or_else(|| entry.bytes())
            })
            .sum()
    }

    #[test]
    fn state_files_go_before_whole_boards() {
        let boards = [
            held("a", 10, 100, 5),
            held("b", 20, 100, 5),
            held("c", 30, 100, 5),
        ];
        assert_eq!(evictions(&boards, 1_000, 40), []);
        assert_eq!(
            evictions(&boards, 120, 40),
            [
                Eviction::Document("a".into()),
                Eviction::Document("b".into())
            ]
        );
        assert_eq!(
            evictions(&boards, 12, 40),
            [
                Eviction::Document("a".into()),
                Eviction::Document("b".into()),
                Eviction::Document("c".into()),
                Eviction::Board("a".into()),
            ]
        );
    }

    #[test]
    fn boards_unused_for_a_month_go_whatever_the_size() {
        let now = UNUSED_MILLIS + 100;
        let boards = [held("old", 100, 1, 1), held("new", now, 1, 1)];
        assert_eq!(
            evictions(&boards, u64::MAX, now),
            [Eviction::Board("old".into())]
        );
    }

    proptest! {
        #![proptest_config(config())]

        /// After eviction the rest fits the cap, and the least recently used
        /// went first: no kept state is older than an evicted one.
        #[test]
        fn eviction_fits_the_cap_in_lru_order(
            entries in vec((0u64..1_000, 0u64..10_000, 0u64..500), 0..40),
            cap in 0u64..200_000,
        ) {
            let now = 1_000;
            let held: Vec<CachedBoard> = entries
                .iter()
                .enumerate()
                .map(|(n, &(used, state, rest))| held(&format!("b{n:03}"), used, state, rest))
                .collect();
            let evicted = evictions(&held, cap, now);
            prop_assert!(left(&held, &evicted) <= cap);
            let states: BTreeSet<&str> = held
                .iter()
                .filter(|entry| evicted.contains(&Eviction::Document(entry.board.clone())))
                .map(|entry| entry.board.as_str())
                .collect();
            let order = |entry: &&CachedBoard| (entry.last_used, entry.board.clone());
            let newest_gone = held
                .iter()
                .filter(|entry| states.contains(entry.board.as_str()))
                .map(|entry| order(&entry))
                .max();
            let oldest_kept = held
                .iter()
                .filter(|entry| entry.state > 0 && !states.contains(entry.board.as_str()))
                .map(|entry| order(&entry))
                .min();
            let in_order = newest_gone
                .zip(oldest_kept)
                .is_none_or(|(gone, kept)| gone < kept);
            prop_assert!(in_order, "{evicted:?}");
            let boards = evicted
                .iter()
                .filter(|eviction| matches!(eviction, Eviction::Board(_)))
                .count();
            prop_assert!(boards == 0 || states.len() == held.iter().filter(|e| e.state > 0).count());
        }
    }

    #[test]
    fn stored_files_are_evicted_by_age_then_by_use() {
        let held = |key: &str, last_used: u64, bytes: u64| CachedAsset {
            key: key.into(),
            last_used,
            bytes,
        };
        let now = UNUSED_MILLIS + 1_000;
        let files = [
            held("old", 0, 1),
            held("recent", now - 10, 60),
            held("older", now - 20, 50),
            held("newest", now, 30),
        ];
        assert_eq!(asset_evictions(&files, 100, now), ["older", "old"]);
        assert_eq!(asset_evictions(&files, 1_000, now), ["old"]);
    }
}
