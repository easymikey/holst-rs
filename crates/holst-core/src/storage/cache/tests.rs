#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

use std::{
    fs,
    fs::OpenOptions,
    io,
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use crate::{
    domain::board::EditRight,
    effect::{
        cache::{
            CachePort,
            CacheRecord,
            meta,
            meta::ReadCheck,
            plan,
            plan::CachedBoard,
        },
        clock::FixedClock,
        settings::CacheSettings,
    },
    error::{Error, Result, local_data::LocalDataError},
    storage::{
        cache::{ASSETS_DIR, BOARDS_DIR, Cache, INDEX_FILE, META_FILE, STATE_FILE},
        file::name_of,
        lock::LOCK_FILE,
    },
};

const ALICE: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const BOB: &str = "bbbbbbbb-0000-4000-8000-000000000002";

fn board(number: u32) -> String {
    format!("{number:08x}-1111-4000-8000-000000000000")
}

struct Root(PathBuf);

impl Root {
    fn new(test: &str) -> io::Result<Self> {
        let dir = std::env::temp_dir()
            .join(format!("holst-cache-{test}-{}", std::process::id()));
        fs::remove_dir_all(&dir).or_else(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(err)
            }
        })?;
        Ok(Self(dir))
    }

    fn settings(&self, max_bytes: u64) -> CacheSettings {
        CacheSettings {
            root: Some(self.0.clone()),
            roots: vec![self.0.clone()],
            host: "app.holst.so".into(),
            max_bytes,
            trust_millis: crate::effect::settings::DEFAULT_CACHE_TRUST_MILLIS,
        }
    }

    fn cache(&self, user: &str, max_bytes: u64, at: u64) -> Result<Cache> {
        Cache::from_settings(
            &self.settings(max_bytes),
            user,
            FixedClock { unix_millis: 1 },
        )
        .map(|cache| cache.with_clock(FixedClock { unix_millis: at }))
        .ok_or(Error::LocalData(LocalDataError::NoHome))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0)
            .inspect_err(|err| tracing::debug!(%err, "test root not removed"))
            .unwrap_or_default();
    }
}

fn noise(len: usize, seed: u64) -> Vec<u8> {
    let step = |state: u64| {
        state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407)
    };
    (0..len)
        .scan(step(seed), |state, _| {
            *state = step(*state);
            Some(u8::try_from(*state >> 56).unwrap_or_default())
        })
        .collect()
}

fn record(state: &[u8]) -> CacheRecord<'_> {
    CacheRecord {
        document: state,
        state_vector: &[1, 2, 3],
        index: Some(b"index"),
        right: EditRight::Editable,
        downloaded: true,
        verified: true,
    }
}

#[test]
fn a_stored_board_reads_back() -> Result<()> {
    let root = Root::new("round")?;
    let cache = root.cache(ALICE, 1 << 20, 1_000)?;
    let id = board(1);
    assert_eq!(cache.meta(&id), None);
    cache.store(&id, &record(b"state bytes"))?;
    assert_eq!(
        cache.load_document(&id).as_deref(),
        Some(&b"state bytes"[..])
    );
    assert_eq!(cache.load_index(&id).as_deref(), Some(&b"index"[..]));
    let meta = cache.meta(&id).ok_or(LocalDataError::NoHome)?;
    assert_eq!(meta.state_vector(), Some(vec![1, 2, 3]));
    assert_eq!((meta.downloaded_at, meta.last_used), (1_000, 1_000));
    assert_eq!(meta.right, Some(EditRight::Editable));
    assert_eq!(cache.usage().boards, 1);
    assert!(cache.usage().bytes > 0);
    assert_eq!(
        cache.dir(),
        root.0.join("v2").join("app.holst.so").join(ALICE),
        "the layout is v2/<host>/<user>"
    );
    Ok(())
}

#[test]
fn the_index_is_not_packed_twice() -> Result<()> {
    let root = Root::new("index-raw")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?;
    let index = crate::domain::board_text::index::BoardIndex::from_objects(
        &[],
        &board(1),
        vec![0],
    );
    let bytes = index.encode()?;
    cache.store(
        &board(1),
        &CacheRecord {
            index: Some(&bytes),
            ..record(b"state")
        },
    )?;
    let on_disk =
        fs::read(cache.dir().join(BOARDS_DIR).join(board(1)).join(INDEX_FILE))?;
    assert_eq!(on_disk, bytes);
    assert!(on_disk.starts_with(&crate::domain::board_text::index::MAGIC));
    Ok(())
}

#[test]
fn a_store_without_a_fetch_keeps_fetched_at() -> Result<()> {
    let root = Root::new("verified")?;
    let id = board(1);
    let first = root.cache(ALICE, 1 << 20, 1_000)?;
    first.store(&id, &record(b"one"))?;
    let later = root.cache(ALICE, 1 << 20, 5_000)?;
    later.store(
        &id,
        &CacheRecord {
            downloaded: false,
            ..record(b"two")
        },
    )?;
    let meta = later.meta(&id).ok_or(LocalDataError::NoHome)?;
    assert_eq!((meta.downloaded_at, meta.verified_at), (1_000, 5_000));
    let offline = root.cache(ALICE, 1 << 20, 9_000)?;
    offline.store(
        &id,
        &CacheRecord {
            downloaded: false,
            verified: false,
            ..record(b"three")
        },
    )?;
    let offline_meta = offline.meta(&id).ok_or(LocalDataError::NoHome)?;
    assert_eq!(
        (offline_meta.downloaded_at, offline_meta.verified_at),
        (1_000, 5_000)
    );
    Ok(())
}

#[test]
fn only_a_server_check_moves_verified_at() -> Result<()> {
    let root = Root::new("touch")?;
    let id = board(1);
    root.cache(ALICE, 1 << 20, 1)?.store(&id, &record(b"x"))?;
    let later = 1 + meta::TOUCH_EVERY_MILLIS;
    let cache = root.cache(ALICE, 1 << 20, later)?;
    cache.touch(&id, ReadCheck::Read)?;
    let meta = cache.meta(&id).ok_or(LocalDataError::NoHome)?;
    assert_eq!((meta.last_used, meta.verified_at), (later, 1));
    let checked = root.cache(ALICE, 1 << 20, later + 5)?;
    checked.touch(&id, ReadCheck::Verified)?;
    let checked_meta = checked.meta(&id).ok_or(LocalDataError::NoHome)?;
    assert_eq!(
        (checked_meta.last_used, checked_meta.verified_at),
        (later + 5, later + 5)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn everything_is_private() -> Result<()> {
    use std::{os::unix::fs::PermissionsExt as _, path::Path};

    use crate::storage::cache::{TALLY_FILE, files::layout};

    let root = Root::new("private")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?;
    let id = board(1);
    fs::create_dir_all(cache.dir().join(BOARDS_DIR))?;
    [
        layout(&root.0),
        cache.dir().to_path_buf(),
        cache.dir().join(BOARDS_DIR),
    ]
    .iter()
    .try_for_each(|dir| fs::set_permissions(dir, fs::Permissions::from_mode(0o755)))?;
    cache.store(&id, &record(b"x"))?;
    let mode =
        |path: &Path| fs::metadata(path).map(|meta| meta.permissions().mode() & 0o777);
    let dir = cache.dir().join(BOARDS_DIR).join(&id);
    [
        layout(&root.0),
        layout(&root.0).join("app.holst.so"),
        cache.dir().to_path_buf(),
        cache.dir().join(BOARDS_DIR),
        dir.clone(),
    ]
    .iter()
    .try_for_each(|path| {
        assert_eq!(mode(path)?, 0o700, "{}", path.display());
        Ok::<_, Error>(())
    })?;
    [
        dir.join(STATE_FILE),
        dir.join(INDEX_FILE),
        dir.join(META_FILE),
        cache.dir().join(LOCK_FILE),
        cache.dir().join(TALLY_FILE),
    ]
    .iter()
    .try_for_each(|path| {
        assert_eq!(mode(path)?, 0o600, "{}", path.display());
        Ok::<_, Error>(())
    })
}

#[test]
fn leftovers_of_a_crash_are_ignored_and_cleaned() -> Result<()> {
    let root = Root::new("crash")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?;
    let id = board(1);
    cache.store(&id, &record(b"whole"))?;
    let dir = cache.dir().join(BOARDS_DIR).join(&id);
    let partial = dir.join(".holst-4242-7.partial");
    fs::write(&partial, b"torn")?;
    let unfinished = cache.dir().join(BOARDS_DIR).join(board(2));
    fs::create_dir_all(&unfinished)?;
    fs::write(unfinished.join(STATE_FILE), b"half")?;
    assert_eq!(cache.load_document(&id).as_deref(), Some(&b"whole"[..]));
    assert_eq!(cache.load_document(&board(2)), None);
    cache.evict()?;
    assert!(!partial.exists() && !unfinished.exists());
    assert_eq!(cache.load_document(&id).as_deref(), Some(&b"whole"[..]));
    Ok(())
}

#[test]
fn stray_files_never_stop_eviction() -> Result<()> {
    let root = Root::new("strays")?;
    let old = root.cache(ALICE, 1 << 20, 1)?;
    old.store(&board(1), &record(&[7u8; 4_000]))?;
    let boards = old.dir().join(BOARDS_DIR);
    fs::write(boards.join(".DS_Store"), b"finder")?;
    fs::write(boards.join("Thumbs.db"), b"explorer")?;
    fs::create_dir_all(boards.join("not-a-board"))?;
    let month = plan::UNUSED_MILLIS + 10;
    let later = root.cache(ALICE, 1 << 20, month)?;
    later.store(&board(2), &record(b"new"))?;
    assert_eq!(later.meta(&board(1)), None, "a month unused stayed");
    assert!(later.meta(&board(2)).is_some());
    assert!(!boards.join(".DS_Store").exists() && !boards.join("not-a-board").exists());
    fs::write(boards.join(".DS_Store"), b"again")?;
    let tight = root.cache(ALICE, 10, month)?;
    assert!(tight.evict()?.boards > 0, "the cap was not applied");
    Ok(())
}

#[cfg(unix)]
#[test]
fn one_failed_removal_does_not_stop_the_rest() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let root = Root::new("stuck")?;
    let old = root.cache(ALICE, 1 << 20, 1)?;
    [board(1), board(2)]
        .iter()
        .try_for_each(|id| old.store(id, &record(b"old")))?;
    let stuck = old.dir().join(BOARDS_DIR).join(board(1));
    fs::set_permissions(&stuck, fs::Permissions::from_mode(0o500))?;
    let later = root.cache(ALICE, 1 << 20, plan::UNUSED_MILLIS + 10);
    let evicted = later.map(|cache| cache.evict());
    fs::set_permissions(&stuck, fs::Permissions::from_mode(0o700))?;
    let evicted = evicted?;
    assert!(evicted.is_ok(), "{evicted:?}");
    assert!(!old.dir().join(BOARDS_DIR).join(board(2)).exists());
    assert!(stuck.exists(), "the stuck board was removed after all");
    Ok(())
}

/// Waits for `done`: the bad-copy drop of `load_document` runs on its own
/// thread.
fn eventually(done: impl Fn() -> bool) -> bool {
    (0..1000).any(|_| {
        done() || {
            std::thread::sleep(Duration::from_millis(10));
            false
        }
    }) || done()
}

#[test]
fn another_format_is_left_alone_and_broken_files_are_dropped() -> Result<()> {
    let root = Root::new("format")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?;
    let (newer, broken) = (board(1), board(2));
    [&newer, &broken]
        .iter()
        .try_for_each(|id| cache.store(id, &record(b"state")))?;
    let meta_path = cache.dir().join(BOARDS_DIR).join(&newer).join(META_FILE);
    let text = fs::read_to_string(&meta_path)?;
    fs::write(&meta_path, text.replace("\"format\":2", "\"format\":3"))?;
    assert_eq!(cache.meta(&newer), None);
    assert_eq!(cache.load_document(&newer), None);
    assert_eq!(cache.load_index(&newer), None);
    assert!(
        meta_path.exists(),
        "a reader deleted an entry of another format"
    );
    let state_path = cache.dir().join(BOARDS_DIR).join(&broken).join(STATE_FILE);
    fs::write(&state_path, [0x1f, 0x8b, 0, 1, 2])?;
    assert_eq!(cache.load_document(&broken), None);
    assert!(
        eventually(|| cache.meta(&broken).is_none()),
        "a bad copy was not dropped"
    );
    cache.evict()?;
    assert!(
        meta_path.exists(),
        "the sweep deleted a copy of another format"
    );
    Ok(())
}

#[test]
fn the_sweep_drops_a_folder_without_a_readable_meta() -> Result<()> {
    let root = Root::new("sweep-meta")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?;
    let (none, broken, kept) = (board(1), board(2), board(3));
    [&none, &broken, &kept]
        .iter()
        .try_for_each(|id| cache.store(id, &record(b"state")))?;
    let boards = cache.dir().join(BOARDS_DIR);
    fs::remove_file(boards.join(&none).join(META_FILE))?;
    fs::write(boards.join(&broken).join(META_FILE), b"{ not json")?;
    cache.evict()?;
    assert!(!boards.join(&none).exists());
    assert!(!boards.join(&broken).exists());
    assert!(boards.join(&kept).exists());
    Ok(())
}

#[test]
fn discard_and_wipe_wait_for_the_lock() -> Result<()> {
    let root = Root::new("discard-lock")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?.with_lock_wait(100);
    let id = board(1);
    cache.store(&id, &record(b"kept"))?;
    let folder = cache.dir().join(BOARDS_DIR).join(&id);
    let other = OpenOptions::new()
        .write(true)
        .open(cache.dir().join(LOCK_FILE))?;
    other.lock()?;
    let busy = cache.discard(&id);
    assert!(
        matches!(
            busy,
            Err(Error::LocalData(LocalDataError::CacheBusy { .. }))
        ),
        "{busy:?}"
    );
    let wiping = cache.wipe();
    assert!(
        matches!(
            wiping,
            Err(Error::LocalData(LocalDataError::CacheBusy { .. }))
        ),
        "{wiping:?}"
    );
    assert!(folder.exists(), "removed without the lock");
    other.unlock()?;
    cache.discard(&id)?;
    assert!(!folder.exists());
    Ok(())
}

#[test]
fn a_store_leaves_the_entry_of_another_format_to_the_writer() -> Result<()> {
    let root = Root::new("format-writer")?;
    let (old, new) = (
        root.cache(ALICE, 1 << 20, 1)?,
        root.cache(ALICE, 1 << 20, 2)?,
    );
    let id = board(1);
    old.store(&id, &record(b"state"))?;
    let meta_path = old.dir().join(BOARDS_DIR).join(&id).join(META_FILE);
    let text = fs::read_to_string(&meta_path)?;
    fs::write(&meta_path, text.replace("\"format\":2", "\"format\":3"))?;
    assert_eq!(old.load_document(&id), None);
    assert!(fs::read_to_string(&meta_path)?.contains("\"format\":3"));
    new.store(&id, &record(b"fresh"))?;
    assert_eq!(new.load_document(&id).as_deref(), Some(&b"fresh"[..]));
    Ok(())
}

#[test]
fn parallel_stores_do_not_wait_for_each_others_gzip() -> Result<()> {
    let root = Root::new("parallel")?;
    let cache = root.cache(ALICE, 1 << 30, 1)?;
    let states: Vec<Vec<u8>> = (1..=4).map(|seed| noise(3 << 20, seed)).collect();
    let stored: Vec<Result<()>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (1..=4u32)
            .zip(&states)
            .map(|(number, state)| {
                let handle = cache.clone();
                scope.spawn(move || handle.store(&board(number), &record(state)))
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err(Error::LocalData(LocalDataError::NoHome)))
            })
            .collect()
    });
    assert!(stored.iter().all(Result::is_ok), "{stored:?}");
    assert_eq!(cache.usage().boards, 4);
    Ok(())
}

fn asset_batch(count: u32) -> Vec<(String, Vec<u8>)> {
    (0..count)
        .map(|number| (format!("a{number}.png"), vec![1u8; 100]))
        .collect()
}

#[test]
fn ten_assets_take_one_scan() -> Result<()> {
    let root = Root::new("assets-scan")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?;
    let batch = asset_batch(10);
    let borrowed: Vec<(&str, &[u8])> = batch
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    cache.store_assets(&borrowed)?;
    assert_eq!(cache.scans.load(Ordering::Relaxed), 1);
    assert!(batch.iter().all(|(name, bytes)| {
        cache.load_asset(name).as_deref() == Some(bytes.as_slice())
    }));
    Ok(())
}

#[test]
fn assets_are_written_under_the_lock() -> Result<()> {
    let root = Root::new("assets-lock")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?.with_lock_wait(100);
    cache.store(&board(1), &record(b"kept"))?;
    let other = OpenOptions::new()
        .write(true)
        .open(cache.dir().join(LOCK_FILE))?;
    other.lock()?;
    let batch = asset_batch(3);
    let borrowed: Vec<(&str, &[u8])> = batch
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let busy = cache.store_assets(&borrowed);
    assert!(
        matches!(
            busy,
            Err(Error::LocalData(LocalDataError::CacheBusy { .. }))
        ),
        "{busy:?}"
    );
    assert!(
        batch
            .iter()
            .all(|(name, _)| cache.load_asset(name).is_none())
    );
    other.unlock()?;
    cache.store_assets(&borrowed)?;
    assert!(
        batch
            .iter()
            .all(|(name, _)| cache.load_asset(name).is_some())
    );
    Ok(())
}

fn modified_millis(path: &std::path::Path) -> io::Result<u64> {
    let age = fs::metadata(path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?;
    u64::try_from(age.as_millis()).map_err(io::Error::other)
}

#[test]
fn a_stale_asset_read_re_dates_the_image_and_a_fresh_one_does_not() -> Result<()> {
    let root = Root::new("assets-redate")?;
    root.cache(ALICE, 1 << 20, 1)?
        .store_assets(&[("a.png", &[1u8; 100][..])])?;
    let stored = fs::read_dir(root.cache(ALICE, 1 << 20, 1)?.dir().join(ASSETS_DIR))?
        .next()
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))??
        .path();
    let written = modified_millis(&stored)?;
    let fresh =
        root.cache(ALICE, 1 << 20, written + meta::TOUCH_EVERY_MILLIS - 1_000)?;
    assert_eq!(fresh.load_asset("a.png"), Some(vec![1u8; 100]));
    assert_eq!(modified_millis(&stored)?, written, "a recent read rewrote");
    let used = written + meta::TOUCH_EVERY_MILLIS + 1_000;
    let stale = root.cache(ALICE, 1 << 20, used)?;
    assert_eq!(stale.load_asset("a.png"), Some(vec![1u8; 100]));
    assert_eq!(modified_millis(&stored)?, used);
    Ok(())
}

#[test]
fn accounts_and_servers_are_isolated() -> Result<()> {
    let root = Root::new("accounts")?;
    let alice = root.cache(ALICE, 1 << 20, 1)?;
    let bob = root.cache(BOB, 1 << 20, 1)?;
    alice.store(&board(1), &record(b"alice's"))?;
    assert_eq!(bob.meta(&board(1)), None);
    assert_eq!(bob.load_document(&board(1)), None);
    let staging = CacheSettings {
        host: "127.0.0.1_9".into(),
        ..root.settings(1 << 20)
    };
    let elsewhere =
        Cache::from_settings(&staging, ALICE, FixedClock { unix_millis: 1 })
            .ok_or(LocalDataError::NoHome)?;
    assert_eq!(
        elsewhere.meta(&board(1)),
        None,
        "another server's copy was read"
    );
    elsewhere.store(&board(1), &record(b"staging"))?;
    let clock: std::sync::Arc<dyn crate::effect::clock::Clock> =
        std::sync::Arc::new(FixedClock { unix_millis: 1 });
    let found: Vec<(String, String)> = itertools::Itertools::sorted(
        Cache::accounts(&root.settings(1 << 20), &clock)
            .iter()
            .map(|cache| {
                let host = cache.dir().parent().and_then(name_of).unwrap_or_default();
                (host.to_owned(), cache.account_id().to_owned())
            }),
    )
    .collect();
    assert_eq!(
        found,
        [
            ("127.0.0.1_9".to_owned(), ALICE.to_owned()),
            ("app.holst.so".to_owned(), ALICE.to_owned())
        ]
    );
    assert!(
        root.cache("../escape", 1, 1).is_err(),
        "a user must be a UUID"
    );
    let bad_host = CacheSettings {
        host: "..".into(),
        ..root.settings(1)
    };
    assert!(
        Cache::from_settings(&bad_host, ALICE, FixedClock { unix_millis: 1 }).is_none()
    );
    bob.store("../../x", &record(b"no"))?;
    assert!(!root.0.join("x").exists());
    Ok(())
}

#[test]
fn eviction_keeps_the_total_under_the_cap() -> Result<()> {
    let root = Root::new("evict")?;
    let state = vec![7u8; 4_000];
    (1..=4).try_for_each(|n| {
        let cache = root.cache(ALICE, 1 << 20, u64::from(n))?;
        cache.store(&board(n), &record(&state))
    })?;
    let tight = root.cache(ALICE, 450, 10)?;
    assert!(tight.evictable().boards > 0);
    tight.evict()?;
    let held: u64 = tight.copies().iter().map(CachedBoard::bytes).sum();
    assert!(held <= 450, "{held}");
    assert_eq!(tight.load_document(&board(1)), None);
    Ok(())
}

#[test]
fn a_store_reads_no_other_meta_until_eviction_is_due() -> Result<()> {
    let root = Root::new("tally")?;
    let first = root.cache(ALICE, 1 << 20, 1)?;
    first.store(&board(1), &record(&noise(3_000, 1)))?;
    let broken = first.dir().join(BOARDS_DIR).join(board(2));
    fs::create_dir_all(&broken)?;
    fs::write(broken.join(META_FILE), b"{\"format\":1")?;
    first.store(&board(3), &record(b"small"))?;
    assert!(broken.exists(), "a store read every meta");
    let tally = first.tally().unwrap_or_default();
    assert!(tally.bytes > 3_000 && tally.evicted_at == 1, "{tally:?}");
    let tight = root.cache(ALICE, 3_000, 2)?;
    tight.store(&board(4), &record(&noise(2_000, 4)))?;
    assert!(!broken.exists(), "eviction past the cap did not sweep");
    let total: u64 = tight.copies().iter().map(CachedBoard::bytes).sum();
    assert!(total <= 3_000, "{total}");
    let counted = tight.tally().map(|kept| kept.bytes.abs_diff(total));
    assert!(counted.is_some_and(|off| off < 64), "{counted:?}");
    Ok(())
}

#[test]
fn a_held_lock_skips_the_write_in_time() -> Result<()> {
    let root = Root::new("busy")?;
    let cache = root.cache(ALICE, 1 << 20, 1)?.with_lock_wait(100);
    cache.store(&board(1), &record(b"kept"))?;
    let other = OpenOptions::new()
        .write(true)
        .open(cache.dir().join(LOCK_FILE))?;
    other.lock()?;
    let start = Instant::now();
    let busy = cache.store(&board(2), &record(b"skipped"));
    assert!(
        matches!(
            busy,
            Err(Error::LocalData(LocalDataError::CacheBusy { .. }))
        ),
        "{busy:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    assert!(matches!(
        cache.touch(&board(1), ReadCheck::Read),
        Ok(()) | Err(Error::LocalData(LocalDataError::CacheBusy { .. }))
    ));
    assert_eq!(
        cache.load_document(&board(1)).as_deref(),
        Some(&b"kept"[..])
    );
    other.unlock()?;
    cache.store(&board(2), &record(b"now"))?;
    assert!(cache.meta(&board(2)).is_some());
    Ok(())
}

#[test]
fn wipe_removes_the_boards_and_keeps_the_lock() -> Result<()> {
    let root = Root::new("wipe")?;
    let (alice, bob) = (root.cache(ALICE, 1 << 20, 1)?, root.cache(BOB, 1 << 20, 1)?);
    bob.store(&board(2), &record(b"bob's"))?;
    alice.store(&board(1), &record(b"alice's"))?;
    alice.wipe()?;
    assert_eq!(alice.usage().boards, 0);
    assert!(!alice.dir().join(BOARDS_DIR).exists());
    assert!(alice.dir().join(LOCK_FILE).exists(), "the lock was removed");
    assert!(bob.meta(&board(2)).is_some(), "another account stays");
    Ok(())
}

#[test]
fn forget_reaches_every_server_and_root() -> Result<()> {
    let (one, two) = (Root::new("forget-one")?, Root::new("forget-two")?);
    let settings = CacheSettings {
        root: None,
        roots: vec![one.0.clone(), two.0.clone()],
        ..one.settings(1 << 20)
    };
    let at = |root: &Root, host: &str, user: &str| {
        let on = CacheSettings {
            host: host.into(),
            ..root.settings(1 << 20)
        };
        Cache::from_settings(&on, user, FixedClock { unix_millis: 1 })
            .ok_or(Error::LocalData(LocalDataError::NoHome))
    };
    let caches = [
        at(&one, "app.holst.so", ALICE)?,
        at(&two, "127.0.0.1_9", ALICE)?,
        at(&one, "app.holst.so", BOB)?,
    ];
    caches
        .iter()
        .try_for_each(|cache| cache.store(&board(1), &record(b"content")))?;
    let old = one.0.join("v1").join(ALICE).join("boards");
    fs::create_dir_all(&old)?;
    let wiped = Cache::forget(&settings, Some(&ALICE.to_ascii_uppercase()))?;
    assert_eq!(wiped.boards, 2);
    let left: Vec<usize> = caches.iter().map(|cache| cache.usage().boards).collect();
    assert_eq!(left, [0, 0, 1], "Bob's cache is not Alice's to wipe");
    assert!(
        !one.0.join("v1").exists(),
        "an old layout outlived the logout"
    );
    Cache::forget(&settings, None)?;
    assert_eq!(
        caches
            .iter()
            .map(|cache| cache.usage().boards)
            .sum::<usize>(),
        0
    );
    Ok(())
}

#[test]
fn other_layouts_are_found() -> Result<()> {
    let root = Root::new("layouts")?;
    ["v0", "v1", "v2", "v12", "va", "notes"]
        .iter()
        .try_for_each(|name| fs::create_dir_all(root.0.join(name)))?;
    fs::write(root.0.join("v3"), b"a file")?;
    let found: Vec<String> = itertools::Itertools::sorted(
        Cache::other_layouts(&root.0)
            .iter()
            .filter_map(|path| Some(name_of(path)?.to_owned())),
    )
    .collect();
    assert_eq!(found, ["v0", "v1", "v12"]);
    Ok(())
}

#[test]
fn logout_drops_the_workspace_list_and_trees() -> Result<()> {
    use crate::{
        domain::account::Workspace,
        effect::cache::AccountCache,
        storage::cache::account::AccountDisk,
    };
    let root = Root::new("logout-lists")?;
    let settings = root.settings(1 << 20);
    let store = AccountDisk::from_settings(&settings)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?;
    let space = Workspace {
        id: board(1),
        name: "One".into(),
    };
    store.keep_workspaces(ALICE, &[space])?;
    store.keep_tree(ALICE, &board(1), "t", 1)?;
    let alice = root.cache(ALICE, 1 << 20, 1)?;
    alice.store(&board(1), &record(b"x"))?;
    assert!(alice.dir().join("workspaces.json").exists());
    Cache::forget(&settings, Some(ALICE))?;
    assert!(!alice.dir().join("workspaces.json").exists());
    assert!(!alice.dir().join("trees").exists());
    Ok(())
}
