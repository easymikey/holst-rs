//! Two handles on one cache directory, as two processes would hold it.
use std::{fs, thread};

use holst_core::{
    command::board_load::account_cache,
    domain::board::EditRight,
    effect::{
        cache::{CachePort, CacheRecord},
        settings::CacheSettings,
    },
    error::local_data::LocalDataError,
    system::clock::SystemClock,
};

const USER: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const BOARD: &str = "11111111-1111-4000-8000-000000000000";

#[test]
fn two_handles_storing_the_same_board_both_succeed_and_leave_one_entry()
-> holst_core::Result<()> {
    let root =
        std::env::temp_dir().join(format!("holst-shared-cache-{}", std::process::id()));
    let settings = CacheSettings {
        root: Some(root.clone()),
        roots: vec![root.clone()],
        host: "app.holst.so".into(),
        max_bytes: 1 << 24,
        trust_millis: 1_000,
    };
    let (first, second) = (
        account_cache(&settings, USER, SystemClock),
        account_cache(&settings, USER, SystemClock),
    );
    let missing = || holst_core::Error::LocalData(LocalDataError::NoHome);
    let (first, second) = (first.ok_or_else(missing)?, second.ok_or_else(missing)?);
    let state = vec![7u8; 200_000];
    let record = CacheRecord {
        document: &state,
        state_vector: &[1, 2, 3],
        index: Some(b"index"),
        right: EditRight::Editable,
        downloaded: true,
        verified: true,
    };
    let (stored_first, stored_second) = thread::scope(|scope| {
        let first = scope.spawn(|| first.store(BOARD, &record));
        let second = scope.spawn(|| second.store(BOARD, &record));
        (first.join(), second.join())
    });
    assert!(matches!(stored_first, Ok(Ok(()))), "{stored_first:?}");
    assert!(matches!(stored_second, Ok(Ok(()))), "{stored_second:?}");
    assert_eq!(first.usage().boards, 1);
    assert_eq!(second.load_document(BOARD), Some(state));
    fs::remove_dir_all(&root)?;
    Ok(())
}
