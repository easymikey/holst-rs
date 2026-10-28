//! The board cache with a synthetic 4000-object board, never real content.

use std::{fs, io, path::PathBuf};

use divan::{Bencher, black_box};
use holst_core::{
    domain::{board::EditRight, board_file::document},
    effect::{
        cache::{CachePort as _, CacheRecord},
        settings::CacheSettings,
    },
    storage::cache::Cache,
};
use yrs::{
    Any,
    Doc,
    Map as _,
    ReadTxn as _,
    Transact as _,
    updates::encoder::Encode as _,
};

fn main() -> holst_core::Result<()> {
    let doc = board();
    let full = document::encoded(&doc);
    let state_vector = doc.transact().state_vector().encode_v1();
    let scratch = TempCache::new("check");
    let cache = scratch
        .cache()
        .ok_or(holst_core::error::local_data::LocalDataError::NoHome)?;
    cache.store(BOARD, &record(&full, &state_vector))?;
    cache
        .load_document(BOARD)
        .filter(|state| *state == full)
        .ok_or(holst_core::error::local_data::LocalDataError::NoHome)?;
    drop(scratch);
    divan::main();
    Ok(())
}

const OBJECTS: usize = 4_000;

const BOARD: &str = "0000000b-1111-4000-8000-000000000000";

const USER: &str = "0000000a-2222-4000-8000-000000000000";

fn board() -> Doc {
    let doc = Doc::with_client_id(1);
    let objects = doc.get_or_insert_map("objects");
    (0..OBJECTS).for_each(|n| {
        let id = format!("{n:08x}-0000-4000-8000-000000000000");
        let fields = format!(
            r#"{{"id":"{id}","type":"sticker","position":{{"x":{},"y":{}}},"width":192,"height":192,"zIndex":{n},"fillColor":{{"color":"yellow4","opacity":1}},"text":"Идея {n}: проверить гипотезу и записать вывод"}}"#,
            n % 80 * 210,
            n / 80 * 210
        );
        let value = Any::from_json(&fields).unwrap_or(Any::Null);
        objects.insert(&mut doc.transact_mut(), id, value);
    });
    doc
}

struct TempCache(PathBuf);

impl TempCache {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("holst-bench-cache-{name}-{}", std::process::id()));
        let _cleared: io::Result<()> = fs::remove_dir_all(&dir);
        Self(dir)
    }

    fn cache(&self) -> Option<Cache> {
        let settings = CacheSettings {
            root: Some(self.0.clone()),
            roots: vec![self.0.clone()],
            host: "app.holst.so".into(),
            max_bytes: 1 << 30,
            trust_millis: holst_core::effect::settings::DEFAULT_CACHE_TRUST_MILLIS,
        };
        Cache::from_settings(&settings, USER, holst_core::system::clock::SystemClock)
    }
}

impl Drop for TempCache {
    fn drop(&mut self) {
        let _removed: io::Result<()> = fs::remove_dir_all(&self.0);
    }
}

fn record<'a>(state: &'a [u8], state_vector: &'a [u8]) -> CacheRecord<'a> {
    CacheRecord {
        document: state,
        state_vector,
        index: None,
        right: EditRight::Editable,
        downloaded: true,
        verified: true,
    }
}

#[divan::bench]
fn cache_store(bencher: Bencher<'_, '_>) {
    let doc = board();
    let full = document::encoded(&doc);
    let state_vector = doc.transact().state_vector().encode_v1();
    let scratch = TempCache::new("store");
    let cache = scratch.cache();
    bencher
        .counter(divan::counter::BytesCount::new(full.len()))
        .bench(|| {
            cache.as_ref().map(|cache| {
                cache.store(BOARD, &record(black_box(&full), &state_vector))
            })
        });
}

#[divan::bench]
fn cache_load_state(bencher: Bencher<'_, '_>) {
    let doc = board();
    let full = document::encoded(&doc);
    let state_vector = doc.transact().state_vector().encode_v1();
    let scratch = TempCache::new("load");
    let cache = scratch.cache();
    let stored = cache
        .as_ref()
        .map(|cache| cache.store(BOARD, &record(&full, &state_vector)));
    drop(stored);
    bencher
        .counter(divan::counter::BytesCount::new(full.len()))
        .bench(|| {
            cache
                .as_ref()
                .and_then(|cache| cache.load_document(black_box(BOARD)))
                .map(|state| document::decoded(&state).map(drop))
        });
}
