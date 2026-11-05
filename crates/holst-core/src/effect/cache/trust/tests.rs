#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

use yrs::{
    Any,
    Doc,
    Map as _,
    ReadTxn as _,
    Transact as _,
    updates::encoder::Encode as _,
};

use crate::{
    domain::{board::EditRight, board_file::document, board_text::index::BoardIndex},
    effect::{
        cache::{
            CachePort,
            CacheRecord,
            meta::ReadCheck,
            trust::{CachedCopy, StaleCopy, Trusted},
        },
        clock::FixedClock,
        settings::DEFAULT_CACHE_TRUST_MILLIS,
    },
    error::{Error, Result, local_data::LocalDataError},
    storage::cache::memory::MemoryCache,
};

pub(crate) const BOARD: &str = "0000000a-1111-4000-8000-000000000000";

pub(crate) fn stickers(client: u64, ids: &[&str]) -> Doc {
    let doc = Doc::with_client_id(client);
    let objects = doc.get_or_insert_map("objects");
    ids.iter().fold((), |(), id| {
        let fields = serde_json::json!({"id": id, "type": "sticker"});
        let sticker = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
        objects.insert(&mut doc.transact_mut(), *id, sticker);
    });
    doc
}

pub(crate) fn keep(cache: &dyn CachePort, doc: &Doc) -> Result<()> {
    let index = BoardIndex::from_document(doc, BOARD).encode()?;
    cache.store(
        BOARD,
        &CacheRecord {
            document: &document::encoded(doc),
            state_vector: &doc.transact().state_vector().encode_v1(),
            index: Some(&index),
            right: EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )
}

fn kept_at(doc: &Doc, later: u64) -> Result<MemoryCache> {
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    keep(&memory, doc)?;
    Ok(memory.with_clock(FixedClock { unix_millis: later }))
}

fn trusted_index(cache: &dyn CachePort) -> Option<CachedCopy<Trusted, BoardIndex>> {
    CachedCopy::read(cache, BOARD)?
        .with_index(cache)?
        .trusted(cache)
        .ok()
}

#[test]
fn a_fresh_copy_the_server_confirms_is_verified() -> Result<()> {
    let doc = stickers(4, &["one"]);
    let cache = kept_at(&doc, DEFAULT_CACHE_TRUST_MILLIS)?;
    let server = doc.transact().state_vector();
    let verified = trusted_index(&cache)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?
        .verify(Some(&server));
    let confirmed = verified.map_err(|_| Error::LocalData(LocalDataError::NoHome))?;
    assert_eq!(confirmed.read_check(), ReadCheck::Verified);
    assert_eq!(confirmed.right(), EditRight::Editable);
    assert_eq!(confirmed.into_body().len(), 1);
    let seed = CachedCopy::read(&cache, BOARD)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?
        .trusted(&cache)
        .map_err(|_| Error::LocalData(LocalDataError::NoHome))?
        .with_document(&cache, document::decoded)
        .map_err(|_| Error::LocalData(LocalDataError::NoHome))?
        .into_seed();
    assert_eq!(seed.transact().state_vector(), server);
    Ok(())
}

#[test]
fn a_copy_past_the_trust_window_is_stale() -> Result<()> {
    let doc = stickers(4, &["one"]);
    let cache = kept_at(&doc, DEFAULT_CACHE_TRUST_MILLIS + 1)?;
    let read = || {
        CachedCopy::read(&cache, BOARD).ok_or(Error::LocalData(LocalDataError::NoHome))
    };
    assert_eq!(read()?.trusted(&cache).err(), Some(StaleCopy::Expired));
    let indexed = read()?
        .with_index(&cache)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?;
    assert_eq!(indexed.trusted(&cache).err(), Some(StaleCopy::Expired));
    Ok(())
}

#[test]
fn a_seed_ahead_of_the_server_is_stale() -> Result<()> {
    let copy = stickers(4, &["mine", "more"]);
    let server = stickers(4, &["restored"]).transact().state_vector();
    let cache = kept_at(&copy, 2)?;
    let checked =
        trusted_index(&cache).map(|cached| cached.verify(Some(&server)).err());
    assert_eq!(checked, Some(Some(StaleCopy::Ahead)));
    Ok(())
}

#[test]
fn a_copy_behind_the_server_is_stale() -> Result<()> {
    let copy = stickers(4, &["one"]);
    let server = stickers(4, &["one", "two"]).transact().state_vector();
    let cache = kept_at(&copy, 2)?;
    let behind = trusted_index(&cache).map(|cached| cached.verify(Some(&server)).err());
    assert_eq!(behind, Some(Some(StaleCopy::Behind)));
    let blind = trusted_index(&cache).map(|cached| cached.verify(None).err());
    assert_eq!(blind, Some(Some(StaleCopy::Behind)));
    Ok(())
}

#[test]
fn offline_serves_a_stale_copy_unverified() -> Result<()> {
    let doc = stickers(4, &["one"]);
    let cache = kept_at(&doc, 10 * DEFAULT_CACHE_TRUST_MILLIS)?;
    let served = CachedCopy::read(&cache, BOARD)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?
        .offline()
        .with_document(&cache, document::decoded)
        .map_err(|_| Error::LocalData(LocalDataError::NoHome))?;
    assert_eq!(served.read_check(), ReadCheck::Read);
    assert_eq!(
        served.into_body().transact().state_vector(),
        doc.transact().state_vector()
    );
    Ok(())
}

#[test]
fn a_corrupt_document_in_memory_is_dropped() -> Result<()> {
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    memory.store(
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
    let read = CachedCopy::read(&memory, BOARD)
        .ok_or(Error::LocalData(LocalDataError::NoHome))?;
    let trusted = read
        .trusted(&memory)
        .map_err(|_| Error::LocalData(LocalDataError::NoHome))?;
    let restored = trusted.with_document(&memory, document::decoded);
    assert_eq!(
        restored.err().map(|_| ()),
        Some(()),
        "a corrupt copy seeded"
    );
    assert!(
        CachedCopy::read(&memory, BOARD).is_none(),
        "the corrupt copy stayed"
    );
    Ok(())
}
