use std::path::{Path, PathBuf};

use crate::effect::cache::{
    CacheUsage,
    plan::{CachedBoard, Eviction},
};

pub(crate) const LAYOUT: &str = "v2";

pub(crate) fn freed(copies: &[CachedBoard], evictions: &[Eviction]) -> CacheUsage {
    let gone = |entry: &CachedBoard| {
        let whole = evictions.contains(&Eviction::Board(entry.board.clone()));
        let state = evictions.contains(&Eviction::Document(entry.board.clone()));
        match (whole, state) {
            (true, _) => Some(entry.bytes()),
            (false, true) => Some(entry.state),
            (false, false) => None,
        }
    };
    copies
        .iter()
        .filter_map(gone)
        .map(|bytes| CacheUsage { boards: 1, bytes })
        .sum::<CacheUsage>()
}

pub(crate) fn layout(root: &Path) -> PathBuf {
    root.join(LAYOUT)
}

pub(crate) fn is_layout(name: &str) -> bool {
    name.strip_prefix('v').is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

pub(crate) fn is_host_key(name: &str) -> bool {
    let safe = name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_".contains(&byte)
    });
    safe && !name.is_empty() && name != "." && name != ".."
}
