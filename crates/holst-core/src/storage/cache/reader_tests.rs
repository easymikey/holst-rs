#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "the test holds the lock file of a real cache and times the wait"
)]

use std::{
    fs::{self, OpenOptions},
    time::{Duration, Instant},
};

use crate::{
    domain::{board::EditRight, board_file::document},
    effect::{
        cache::{
            CachePort,
            CacheRecord,
            trust::{CachedCopy, StaleCopy},
        },
        clock::FixedClock,
        settings::{CacheSettings, DEFAULT_CACHE_TRUST_MILLIS},
    },
    error::{Error, Result, local_data::LocalDataError},
    storage::{cache::Cache, lock::LOCK_FILE},
};

const BOARD: &str = "0000000a-1111-4000-8000-000000000000";

#[test]
fn a_corrupt_document_on_disk_is_dropped_without_waiting_for_the_lock() -> Result<()> {
    let root = std::env::temp_dir()
        .join(format!("holst-trust-discard-{}", std::process::id()));
    let settings = CacheSettings {
        root: Some(root.clone()),
        roots: vec![root.clone()],
        host: "app.holst.so".into(),
        max_bytes: 1 << 20,
        trust_millis: DEFAULT_CACHE_TRUST_MILLIS,
    };
    let wait = Duration::from_secs(3);
    let cache = Cache::from_settings(&settings, BOARD, FixedClock { unix_millis: 1 })
        .ok_or(Error::LocalData(LocalDataError::NoHome))?
        .with_lock_wait(3_000);
    cache.store(
        BOARD,
        &CacheRecord {
            document: b"not a yjs update",
            state_vector: &[0],
            index: None,
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    let other = OpenOptions::new()
        .write(true)
        .open(cache.dir().join(LOCK_FILE))?;
    other.lock()?;
    let copy = CachedCopy::read(&cache, BOARD)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?;
    let started = Instant::now();
    let restored = copy.offline().with_document(&cache, document::decoded);
    let took = started.elapsed();
    other.unlock()?;
    fs::remove_dir_all(&root).or_else(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(err)
        }
    })?;
    assert_eq!(restored.err(), Some(StaleCopy::Corrupt));
    assert!(took < wait, "the reader waited {took:?} for the lock");
    Ok(())
}
