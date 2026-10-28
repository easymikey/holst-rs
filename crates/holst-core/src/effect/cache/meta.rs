//! What `meta.json` and the tally file record about a cached board; pure.

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::domain::board::EditRight;

pub const FORMAT: u32 = 2;

pub const TOUCH_EVERY_MILLIS: u64 = 60 * 60 * 1000;

pub const SWEEP_EVERY_MILLIS: u64 = 24 * 60 * 60 * 1000;

/// What `meta.json` says about one cached board; times are Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMeta {
    pub format: u32,
    pub state_vector: String,
    pub right: Option<EditRight>,
    pub downloaded_at: u64,
    pub verified_at: u64,
    pub last_used: u64,
    pub sizes: CopySizes,
}

/// The bytes of one entry's files on disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopySizes {
    pub document: u64,
    pub index: u64,
}

impl CacheMeta {
    /// The state vector's bytes; `None` for text that is not hex.
    #[must_use]
    pub fn state_vector(&self) -> Option<Vec<u8>> {
        unhex(&self.state_vector)
    }

    /// How long ago the board was last downloaded whole, at `at`.
    #[must_use]
    pub const fn age_millis(&self, at: u64) -> u64 {
        at.saturating_sub(self.downloaded_at)
    }

    /// Whether the last full download is under `trust_ms` old at `at`.
    #[must_use]
    pub const fn is_trusted(&self, at: u64, trust_ms: u64) -> bool {
        self.age_millis(at) < trust_ms
    }

    /// Whether `last_used` is old enough to be rewritten at `at`.
    #[must_use]
    pub const fn is_touch_due(&self, at: u64) -> bool {
        at.saturating_sub(self.last_used) >= TOUCH_EVERY_MILLIS
    }

    /// The meta after a read at `at`; `None` when nothing is due yet.
    #[must_use]
    pub fn touched(&self, at: u64, seen: ReadCheck) -> Option<Self> {
        let stale = at.saturating_sub(self.verified_at) >= TOUCH_EVERY_MILLIS;
        let (verify, verified_at) = match (seen, stale) {
            (ReadCheck::Verified, true) => (true, at),
            (ReadCheck::Verified, false) | (ReadCheck::Read, _) => {
                (false, self.verified_at)
            }
        };
        (self.is_touch_due(at) || verify).then(|| Self {
            last_used: at,
            verified_at,
            ..self.clone()
        })
    }

    /// The bytes of the entry's files, `meta` its own length.
    #[must_use]
    pub const fn bytes(&self, meta: u64) -> u64 {
        self.sizes
            .document
            .saturating_add(self.sizes.index)
            .saturating_add(meta)
    }
}

/// How a board was read: from the cache alone, or checked with the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadCheck {
    Read,
    Verified,
}

/// The running total of an account's cache, so a store reads no other meta.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheTally {
    pub bytes: u64,
    #[serde(rename = "swept_at")]
    pub evicted_at: u64,
}

impl CacheTally {
    /// The total once an entry of `before` bytes is replaced by `after`.
    #[must_use]
    pub const fn replaced(self, before: u64, after: u64) -> Self {
        Self {
            bytes: self.bytes.saturating_sub(before).saturating_add(after),
            ..self
        }
    }

    /// Whether eviction must read every meta: over `max_bytes`, or a day since.
    #[must_use]
    pub const fn is_eviction_due(&self, max_bytes: u64, at: u64) -> bool {
        self.bytes > max_bytes
            || at.saturating_sub(self.evicted_at) >= SWEEP_EVERY_MILLIS
    }
}

/// The tally file read back; anything else is no tally, so a sweep is due.
#[must_use]
pub fn tally_of(bytes: &[u8]) -> Option<CacheTally> {
    serde_json::from_slice::<CacheTally>(bytes).ok()
}

/// `meta.json` read back; another format or broken JSON is no entry.
#[must_use]
pub fn meta_of(bytes: &[u8]) -> Option<CacheMeta> {
    serde_json::from_slice::<CacheMeta>(bytes)
        .ok()
        .filter(|meta| meta.format == FORMAT)
}

/// Bytes as lowercase hex.
///
/// ```
/// use holst_core::effect::cache::meta::{hex, unhex};
///
/// assert_eq!(hex(&[0, 171, 255]), "00abff");
/// assert_eq!(unhex("00abff"), Some(vec![0, 171, 255]));
/// assert_eq!(unhex("0g"), None);
/// ```
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .format_with("", |byte, write| write(&format_args!("{byte:02x}")))
        .to_string()
}

/// Hex back to bytes; `None` for an odd length or a non-hex digit.
#[must_use]
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if digits.len().is_multiple_of(2) {
        digits
            .chunks_exact(2)
            .map(|pair| {
                let high = char::from(*pair.first()?).to_digit(16)?;
                let low = char::from(*pair.get(1)?).to_digit(16)?;
                u8::try_from(high * 16 + low).ok()
            })
            .collect()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use crate::{
        domain::board::EditRight,
        effect::cache::meta::{
            CacheMeta,
            CacheTally,
            CopySizes,
            FORMAT,
            ReadCheck,
            SWEEP_EVERY_MILLIS,
            TOUCH_EVERY_MILLIS,
            hex,
            meta_of,
            tally_of,
        },
    };

    #[test]
    fn meta_of_another_format_or_broken_json_is_no_entry() -> serde_json::Result<()> {
        let meta = CacheMeta {
            format: FORMAT,
            state_vector: hex(&[1, 2]),
            right: Some(EditRight::Editable),
            downloaded_at: 1,
            verified_at: 2,
            last_used: 3,
            sizes: CopySizes {
                document: 10,
                index: 0,
            },
        };
        let bytes = serde_json::to_vec(&meta)?;
        assert_eq!(meta_of(&bytes), Some(meta.clone()));
        assert_eq!(meta.state_vector(), Some(vec![1, 2]));
        let newer = serde_json::to_vec(&CacheMeta {
            format: FORMAT + 1,
            ..meta
        })?;
        assert_eq!(meta_of(&newer), None);
        assert_eq!(meta_of(b"{\"format\":1"), None);
        Ok(())
    }

    #[test]
    fn touch_is_due_hourly() {
        let meta = CacheMeta {
            format: FORMAT,
            state_vector: String::new(),
            right: None,
            downloaded_at: 0,
            verified_at: 0,
            last_used: 1_000,
            sizes: CopySizes::default(),
        };
        assert!(!meta.is_touch_due(1_000 + TOUCH_EVERY_MILLIS - 1));
        assert!(meta.is_touch_due(1_000 + TOUCH_EVERY_MILLIS));
        assert_eq!(meta.age_millis(500), 500);
    }

    #[test]
    fn a_tally_written_before_the_rename_still_reads() {
        let tally = tally_of(br#"{"bytes":7,"swept_at":9}"#);
        assert_eq!(
            tally,
            Some(CacheTally {
                bytes: 7,
                evicted_at: 9
            })
        );
        let written = serde_json::to_string(&tally).unwrap_or_default();
        assert!(written.contains("swept_at"), "{written}");
    }

    #[test]
    fn the_tally_decides_when_to_sweep() {
        let tally = CacheTally {
            bytes: 100,
            evicted_at: 10,
        };
        assert_eq!(tally.replaced(30, 50).bytes, 120);
        assert_eq!(tally.replaced(500, 0).bytes, 0);
        assert!(!tally.is_eviction_due(100, 10 + SWEEP_EVERY_MILLIS - 1));
        assert!(tally.is_eviction_due(99, 11));
        assert!(tally.is_eviction_due(100, 10 + SWEEP_EVERY_MILLIS));
        assert_eq!(tally_of(b"{\"bytes\":1"), None);
    }

    #[test]
    fn only_a_check_is_a_verification() {
        let meta = CacheMeta {
            format: FORMAT,
            state_vector: String::new(),
            right: None,
            downloaded_at: 0,
            verified_at: 5,
            last_used: 5,
            sizes: CopySizes::default(),
        };
        let later = 5 + TOUCH_EVERY_MILLIS;
        let read = meta.touched(later, ReadCheck::Read);
        assert_eq!(
            read.map(|touched| (touched.last_used, touched.verified_at)),
            Some((later, 5))
        );
        let checked = meta.touched(later, ReadCheck::Verified);
        assert_eq!(checked.map(|touched| touched.verified_at), Some(later));
        assert_eq!(
            meta.touched(6, ReadCheck::Read),
            None,
            "rewritten more than hourly"
        );
    }
}
