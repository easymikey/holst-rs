use std::{
    borrow::Cow,
    fs,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use crate::{
    domain::{
        board_file::{compression, compression::Compression},
        id::is_uuid,
    },
    effect::cache::{
        CachePort,
        CacheRecord,
        CacheUsage,
        meta::{CacheMeta, CacheTally, CopySizes, ReadCheck},
        plan::{CachedAsset, CachedBoard, Eviction},
    },
    error::{Error, Result, local_data::LocalDataError},
    storage::{
        cache::{
            ASSETS_DIR,
            BOARDS_DIR,
            Cache,
            INDEX_FILE,
            META_FILE,
            STATE_FILE,
            TALLY_FILE,
            TREES_DIR,
            WORKSPACES_FILE,
            files::{freed, layout},
        },
        file::{
            byte_len,
            entries,
            name_of,
            private_dir,
            remove_any,
            remove_dir,
            remove_file,
            subdirs,
            tighten,
            tree_size,
            write_error,
        },
        lock::{LOCK_FILE, lock_waiting},
    },
};

impl Cache {
    fn locked(&self) -> Result<File> {
        self.private_dir(&self.dir)?;
        let path = self.dir.join(LOCK_FILE);
        lock_waiting(&path, self.lock_wait_millis)
            .map_err(|err| write_error(&path, err))?
            .ok_or_else(|| {
                Error::LocalData(LocalDataError::CacheBusy {
                    path: self.dir.clone(),
                })
            })
    }
    // Readers never delete: the sweep under the lock does.
    fn meta_in(dir: &Path) -> Option<CacheMeta> {
        Self::read_meta(dir).map(|(meta, _)| meta)
    }
    fn read_meta(dir: &Path) -> Option<(CacheMeta, u64)> {
        let bytes = fs::read(dir.join(META_FILE)).ok()?;
        crate::effect::cache::meta::meta_of(&bytes).map(|meta| (meta, byte_len(&bytes)))
    }
    // Another holst version's copy: valid JSON whose `format` is not ours.
    fn is_other_format(dir: &Path) -> bool {
        fs::read(dir.join(META_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|meta| meta.get("format")?.as_u64())
            .is_some_and(|format| {
                format != u64::from(crate::effect::cache::meta::FORMAT)
            })
    }
    fn write_meta(dir: &Path, meta: &CacheMeta) -> Result<u64> {
        let bytes = serde_json::to_vec(meta)?;
        crate::storage::file::replace_private(&dir.join(META_FILE), &[&bytes])?;
        Ok(byte_len(&bytes))
    }
    pub(crate) fn tally(&self) -> Option<CacheTally> {
        fs::read(self.dir.join(TALLY_FILE))
            .ok()
            .and_then(|bytes| crate::effect::cache::meta::tally_of(&bytes))
    }
    fn write_tally(&self, tally: &CacheTally) -> Result<()> {
        let bytes = serde_json::to_vec(tally)?;
        crate::storage::file::replace_private(&self.dir.join(TALLY_FILE), &[&bytes])
    }
    fn store_locked(
        &self,
        dir: &Path,
        record: &CacheRecord<'_>,
        state: &[u8],
    ) -> Result<()> {
        let at = self.clock.unix_millis();
        self.private_dir(dir)?;
        let previous = Self::read_meta(dir);
        crate::storage::file::replace_private(&dir.join(STATE_FILE), &[state])?;
        let index_path = dir.join(INDEX_FILE);
        record.index.map_or_else(
            || remove_file(&index_path),
            |bytes| crate::storage::file::replace_private(&index_path, &[bytes]),
        )?;
        let earlier = previous.as_ref().map(|(meta, _)| meta);
        let meta = CacheMeta {
            format: crate::effect::cache::meta::FORMAT,
            state_vector: crate::effect::cache::meta::hex(record.state_vector),
            right: Some(record.right),
            downloaded_at: match (record.downloaded, earlier) {
                (false, Some(earlier)) => earlier.downloaded_at,
                (true, _) | (false, None) => at,
            },
            verified_at: match (record.verified, earlier) {
                (true, _) => at,
                (false, earlier) => earlier.map_or(0, |earlier| earlier.verified_at),
            },
            last_used: at,
            sizes: CopySizes {
                document: byte_len(state),
                index: record.index.map_or(0, byte_len),
            },
        };
        let written = Self::write_meta(dir, &meta)?;
        let before = previous.map_or(0, |(old, length)| old.bytes(length));
        let tally = self
            .tally()
            .map(|tally| tally.replaced(before, meta.bytes(written)))
            .filter(|tally| !tally.is_eviction_due(self.max_bytes, at));
        tally.map_or_else(
            || {
                self.evict_locked(at);
                Ok(())
            },
            |tally| self.write_tally(&tally),
        )
    }
    pub(crate) fn copies(&self) -> Vec<CachedBoard> {
        self.board_dirs()
            .into_iter()
            .filter_map(|dir| {
                let board = name_of(&dir)?.to_owned();
                let meta = Self::meta_in(&dir)?;
                let size = |name: &str| {
                    fs::metadata(dir.join(name)).map_or(0, |file| file.len())
                };
                Some(CachedBoard {
                    board,
                    last_used: meta.last_used,
                    state: size(STATE_FILE),
                    rest: size(INDEX_FILE).saturating_add(size(META_FILE)),
                })
            })
            .collect()
    }
    fn board_dirs(&self) -> Vec<PathBuf> {
        subdirs(&self.dir.join(BOARDS_DIR))
            .into_iter()
            .filter(|dir| name_of(dir).is_some_and(is_uuid))
            .collect()
    }
    fn evict_files(&self) -> Vec<Error> {
        let boards = self.board_dirs();
        let strays = entries(&self.dir.join(BOARDS_DIR))
            .into_iter()
            .filter(|path| !boards.contains(path))
            .map(|path| remove_any(&path));
        let unfinished = boards
            .iter()
            .filter(|dir| Self::read_meta(dir).is_none() && !Self::is_other_format(dir))
            .map(|dir| remove_dir(dir));
        let assets = self.dir.join(ASSETS_DIR);
        let partials = boards
            .iter()
            .chain([&self.dir, &assets])
            .flat_map(|dir| entries(dir))
            .filter(|path| name_of(path).is_some_and(crate::storage::file::is_partial))
            .map(|path| remove_file(&path));
        strays
            .chain(unfinished)
            .chain(partials)
            .filter_map(Result::err)
            .collect()
    }
    fn evict_locked(&self, at: u64) -> CacheUsage {
        let swept = self.evict_files();
        let copies = self.copies();
        let evictions =
            crate::effect::cache::plan::evictions(&copies, self.max_bytes, at);
        let (done, failed): (Vec<_>, Vec<_>) = evictions
            .into_iter()
            .map(|eviction| {
                let outcome = self.remove_evicted(&eviction);
                (eviction, outcome)
            })
            .partition(|(_, outcome)| outcome.is_ok());
        let done: Vec<Eviction> =
            done.into_iter().map(|(eviction, _)| eviction).collect();
        let freed = freed(&copies, &done).combine(self.evict_assets(at));
        let total: u64 = copies.iter().map(CachedBoard::bytes).sum();
        let tally = CacheTally {
            bytes: total.saturating_sub(freed.bytes),
            evicted_at: at,
        };
        let tallied = self.write_tally(&tally).err();
        swept
            .into_iter()
            .chain(failed.into_iter().filter_map(|(_, outcome)| outcome.err()))
            .chain(tallied)
            .for_each(|err| tracing::debug!(%err, "board cache: not cleaned"));
        freed
    }
    fn assets(&self) -> Vec<CachedAsset> {
        #[cfg(test)]
        self.scans
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        entries(&self.dir.join(ASSETS_DIR))
            .into_iter()
            .filter_map(|path| {
                let key = name_of(&path)?.to_owned();
                let meta = fs::symlink_metadata(&path).ok()?;
                let last_used =
                    meta.modified()
                        .ok()?
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .and_then(|since| u64::try_from(since.as_millis()).ok())?;
                (meta.is_file() && !crate::storage::file::is_partial(&key)).then_some(
                    CachedAsset {
                        key,
                        last_used,
                        bytes: meta.len(),
                    },
                )
            })
            .collect()
    }
    const fn asset_max_bytes(&self) -> u64 {
        self.max_bytes / crate::effect::cache::plan::ASSET_SHARE
    }
    fn evict_assets(&self, at: u64) -> CacheUsage {
        let copies = self.assets();
        let gone = crate::effect::cache::plan::asset_evictions(
            &copies,
            self.asset_max_bytes(),
            at,
        );
        let dir = self.dir.join(ASSETS_DIR);
        let bytes = copies
            .iter()
            .filter(|asset| gone.contains(&asset.key))
            .filter(|asset| {
                remove_file(&dir.join(&asset.key))
                    .inspect_err(
                        |err| tracing::debug!(%err, "board cache: not cleaned"),
                    )
                    .is_ok()
            })
            .map(|asset| asset.bytes)
            .sum();
        CacheUsage { boards: 0, bytes }
    }
    fn asset_path(&self, name: &str) -> Option<PathBuf> {
        crate::effect::cache::plan::asset_key(name)
            .map(|key| self.dir.join(ASSETS_DIR).join(key))
    }
    fn remove_evicted(&self, eviction: &Eviction) -> Result<()> {
        match eviction {
            Eviction::Document(board) => self.board_dir(board).map_or(Ok(()), |dir| {
                remove_file(&dir.join(STATE_FILE))?;
                Self::read_meta(&dir).map_or(Ok(()), |(meta, _)| {
                    let sizes = CopySizes {
                        document: 0,
                        ..meta.sizes
                    };
                    Self::write_meta(&dir, &CacheMeta { sizes, ..meta }).map(drop)
                })
            }),
            Eviction::Board(board) => {
                self.board_dir(board).map_or(Ok(()), |dir| remove_dir(&dir))
            }
        }
    }
    fn private_dir(&self, path: &Path) -> Result<()> {
        private_dir(path).map_err(|err| write_error(path, err))?;
        let layout = layout(&self.root);
        path.ancestors()
            .take_while(|dir| dir.starts_with(&layout))
            .try_for_each(|dir| tighten(dir).map_err(|err| write_error(dir, err)))
    }
}

impl CachePort for Cache {
    fn meta(&self, board: &str) -> Option<CacheMeta> {
        Self::meta_in(&self.board_dir(board)?)
    }

    fn trusts(&self, meta: &CacheMeta) -> bool {
        meta.is_trusted(self.clock.unix_millis(), self.trust_millis)
    }

    fn load_document(&self, board: &str) -> Option<Vec<u8>> {
        let dir = self.board_dir(board)?;
        Self::meta_in(&dir)?;
        let packed = fs::read(dir.join(STATE_FILE)).ok()?;
        let state = compression::uncompressed(&packed).map(Cow::into_owned).ok();
        if state.is_none() {
            tracing::debug!(board, "dropped a cached board that does not unpack");
            // The drop may wait for the lock: not on the caller's thread.
            let (cache, board) = (self.clone(), board.to_owned());
            std::thread::Builder::new()
                .spawn(move || {
                    cache
                        .discard(&board)
                        .inspect_err(
                            |err| tracing::debug!(%err, "board cache: a bad board not dropped"),
                        )
                        .unwrap_or_default();
                })
                .inspect_err(
                    |err| tracing::debug!(%err, "board cache: a bad board not dropped"),
                )
                .map(drop)
                .unwrap_or_default();
        }
        state
    }

    fn load_index(&self, board: &str) -> Option<Vec<u8>> {
        let dir = self.board_dir(board)?;
        Self::meta_in(&dir)?;
        fs::read(dir.join(INDEX_FILE)).ok()
    }

    fn store(&self, board: &str, record: &CacheRecord<'_>) -> Result<()> {
        self.board_dir(board).map_or(Ok(()), |dir| {
            // Gzip runs before the lock: it is the slow step and needs no file.
            let state = compression::compressed(record.document, Compression::Gzip)?;
            let _lock = self.locked()?;
            self.store_locked(&dir, record, &state)
        })
    }

    fn touch(&self, board: &str, seen: ReadCheck) -> Result<()> {
        let at = self.clock.unix_millis();
        let due = self
            .meta(board)
            .and_then(|meta| meta.touched(at, seen))
            .zip(self.board_dir(board));
        due.map_or(Ok(()), |(_, dir)| {
            let _lock = self.locked()?;
            // Read again under the lock: a store meanwhile may have changed it.
            let touched =
                Self::read_meta(&dir).and_then(|(meta, _)| meta.touched(at, seen));
            touched.map_or(Ok(()), |meta| Self::write_meta(&dir, &meta).map(drop))
        })
    }

    fn discard(&self, board: &str) -> Result<()> {
        self.board_dir(board).map_or(Ok(()), |dir| {
            let _lock = self.locked()?;
            remove_dir(&dir)
        })
    }

    fn evict(&self) -> Result<CacheUsage> {
        let _lock = self.locked()?;
        Ok(self.evict_locked(self.clock.unix_millis()))
    }

    fn evictable(&self) -> CacheUsage {
        let copies: Vec<CachedBoard> = self.copies();
        let at = self.clock.unix_millis();
        let evictions =
            crate::effect::cache::plan::evictions(&copies, self.max_bytes, at);
        let assets = self.assets();
        let gone = crate::effect::cache::plan::asset_evictions(
            &assets,
            self.asset_max_bytes(),
            at,
        );
        let asset_bytes = assets
            .iter()
            .filter(|asset| gone.contains(&asset.key))
            .map(|asset| asset.bytes)
            .sum();
        freed(&copies, &evictions).combine(CacheUsage {
            boards: 0,
            bytes: asset_bytes,
        })
    }

    fn wipe(&self) -> Result<()> {
        let _lock = self.locked()?;
        let partials = entries(&self.dir)
            .into_iter()
            .filter(|path| name_of(path).is_some_and(crate::storage::file::is_partial));
        [
            self.dir.join(BOARDS_DIR),
            self.dir.join(ASSETS_DIR),
            self.dir.join(TALLY_FILE),
            self.dir.join(WORKSPACES_FILE),
            self.dir.join(TREES_DIR),
        ]
        .into_iter()
        .chain(partials)
        .map(|path| remove_any(&path))
        .filter_map(Result::err)
        .reduce(|first, _| first)
        .map_or(Ok(()), Err)
    }

    fn usage(&self) -> CacheUsage {
        CacheUsage {
            boards: self.board_dirs().len(),
            bytes: tree_size(&self.dir),
        }
    }

    fn shared(&self) -> Arc<dyn CachePort> {
        Arc::new(self.clone())
    }

    fn load_asset(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.asset_path(name)?;
        let bytes = fs::read(&path).ok()?;
        let used = UNIX_EPOCH + Duration::from_millis(self.clock.unix_millis());
        // A read keeps the file in use, rewritten at most hourly.
        let stale = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| used.duration_since(modified).ok())
            .is_some_and(|age| {
                age.as_millis()
                    >= u128::from(crate::effect::cache::meta::TOUCH_EVERY_MILLIS)
            });
        if stale {
            OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(used))
                .inspect_err(
                    |err| tracing::debug!(%err, "board cache: a stored file not touched"),
                )
                .unwrap_or_default();
        }
        Some(bytes)
    }

    fn store_assets(&self, assets: &[(&str, &[u8])]) -> Result<()> {
        let named: Vec<(PathBuf, &[u8])> = assets
            .iter()
            .filter_map(|(name, bytes)| Some((self.asset_path(name)?, *bytes)))
            .collect();
        let _lock = self.locked()?;
        self.private_dir(&self.dir.join(ASSETS_DIR))?;
        let failed = named
            .iter()
            .map(|(path, bytes)| crate::storage::file::replace_private(path, &[bytes]))
            .filter_map(Result::err)
            .reduce(|first, _| first);
        self.evict_assets(self.clock.unix_millis());
        failed.map_or(Ok(()), Err)
    }
}
