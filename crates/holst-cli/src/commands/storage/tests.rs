#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests build and read real files in temp directories"
)]

use std::path::PathBuf;

use holst_core::{
    command::{
        storage::{CleanArgs, clean},
        table::parse,
    },
    effect::{
        cache::CachePort as _,
        settings::{CacheSettings, SessionLocation},
    },
    storage::{
        cache::Cache,
        local_data::{CONFIG_DIR, DataPlace, DataRoots, FilePlace, PlaceRole},
    },
};
use rstest::rstest;
use serde_json::json;

use crate::{
    commands::storage::render::{cleaned_text, listed_text},
    output::view::size_text,
    run::{RunSettings, local_data::forget_cache},
    terminal::detect::TerminalType,
};

#[rstest]
#[case(0, "0 B")]
#[case(1023, "1023 B")]
#[case(1024, "1.0 KiB")]
#[case(1536, "1.5 KiB")]
#[case(169_869_312, "162.0 MiB")]
#[case(3 << 30, "3.0 GiB")]
fn sizes_read_as_a_person_would(#[case] bytes: u64, #[case] expected: &str) {
    assert_eq!(size_text(bytes), expected);
}

fn file(name: &str, bytes: u64, role: PlaceRole) -> FilePlace {
    FilePlace {
        path: PathBuf::from(name),
        name: name.to_owned(),
        bytes,
        role,
        root: PathBuf::from("/"),
    }
}

#[test]
fn the_listing_shows_each_place_and_what_clean_frees() {
    let places = [
        DataPlace::Keychain {
            name: "Keychain item holst-rs/session-token".to_owned(),
        },
        DataPlace::File(file(
            "/h/.config/holst/browser",
            2048,
            PlaceRole::BrowserProfile,
        )),
        DataPlace::File(file("/c/v1/u", 3 << 20, PlaceRole::Cache)),
    ];
    let text = listed_text(&places, 2048);
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        [
            "         -  saved-login  Keychain item holst-rs/session-token  (the saved login)",
            "   2.0 KiB  browser-profile  /h/.config/holst/browser  (profile of the removed browser \
             login)",
            "   3.0 MiB  cache        /c/v1/u  (cached boards of one account, colleagues' \
             content included)",
            "`holst storage clean` frees 2.0 KiB",
        ]
    );
    assert_eq!(listed_text(&[], 0), "holst keeps nothing on this machine");
}

/// A home with a browser profile of ten bytes, which `clean` counts as old.
fn home(name: &str) -> holst_core::Result<DataRoots> {
    let dir = std::env::temp_dir()
        .join(format!("holst-cli-storage-{name}-{}", std::process::id()));
    let profile = dir.join(CONFIG_DIR).join("browser").join("Default");
    holst_core::storage::file::create_dir_all(&profile)?;
    std::fs::write(profile.join("cookies"), b"0123456789")?;
    Ok(DataRoots {
        home: Some(dir.clone()),
        location: SessionLocation::Dir(dir.join(CONFIG_DIR)),
        session_store: None,
        caches: Vec::new(),
    })
}

fn cleaned_with(
    roots: &DataRoots,
    arguments: &serde_json::Value,
) -> holst_core::Result<String> {
    let args: CleanArgs = parse(arguments.as_object().cloned().unwrap_or_default())?;
    let clock: std::sync::Arc<dyn holst_core::effect::clock::Clock> =
        std::sync::Arc::new(holst_core::effect::clock::FixedClock { unix_millis: 0 });
    let cleaned = clean(roots, (&CacheSettings::default(), &clock), &args);
    Ok(cleaned_text(&cleaned, None))
}

#[test]
fn a_dry_run_says_what_it_would_remove() -> holst_core::Result<()> {
    let text = cleaned_with(&home("dry")?, &json!({ "dry_run": true }))?;
    assert_eq!(
        (
            text.lines()
                .next()
                .map(|line| line.starts_with("would remove ")),
            text.lines().last(),
        ),
        (
            Some(true),
            Some("dry run, nothing removed: 10 B would be freed")
        )
    );
    Ok(())
}

#[test]
fn a_clean_says_what_it_removed_and_then_has_nothing_left() -> holst_core::Result<()> {
    let roots = home("sent")?;
    let first = cleaned_with(&roots, &json!({}))?;
    let second = cleaned_with(&roots, &json!({}))?;
    assert_eq!(
        (first.lines().last(), second.as_str()),
        (Some("removed 10 B in total"), "nothing to clean")
    );
    Ok(())
}

const ALICE: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const BOB: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const BOARD: &str = "cccccccc-0000-4000-8000-000000000003";

fn env_without_cache(dir: &std::path::Path) -> holst_core::Result<RunSettings> {
    let (cache, _) = CacheSettings::from_variables(
        &|name| (name == "HOLST_CACHE").then(|| "off".into()),
        &SessionLocation::Dir(dir.to_path_buf()),
        &holst_core::domain::url::BaseUrl::or_default(None)?,
    );
    Ok(RunSettings::new(
        holst_core::effect::settings::Settings {
            base_url: holst_core::domain::url::BaseUrl::or_default(None)?,
            location: SessionLocation::Dir(dir.to_path_buf()),
            log: holst_core::effect::settings::LogSettings::default(),
            cache,
        },
        None,
        TerminalType::Capable,
        crate::run::Prompting::Off,
    ))
}

fn board_load(
    dir: &std::path::Path,
    (host, user): (&str, &str),
    at: u64,
) -> holst_core::Result<Cache> {
    let settings = CacheSettings {
        root: Some(dir.join("cache")),
        roots: vec![dir.join("cache")],
        host: host.into(),
        max_bytes: 1 << 20,
        trust_millis: holst_core::effect::settings::DEFAULT_CACHE_TRUST_MILLIS,
    };
    let cache = Cache::from_settings(
        &settings,
        user,
        holst_core::effect::clock::FixedClock { unix_millis: at },
    )
    .ok_or(holst_core::error::local_data::LocalDataError::NoHome)?;
    cache.store(
        BOARD,
        &holst_core::effect::cache::CacheRecord {
            document: b"board",
            state_vector: &[],
            index: None,
            right: holst_core::domain::board::EditRight::Editable,
            downloaded: true,
            verified: true,
        },
    )?;
    Ok(cache)
}

#[test]
#[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
fn logout_wipes_the_account_cache() -> holst_core::Result<()> {
    let dir = std::env::temp_dir().join(format!("holst-logout-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).unwrap_or_default();
    let now = holst_core::effect::clock::Clock::unix_millis(
        &holst_core::system::clock::SystemClock,
    );
    let alice = board_load(&dir, ("app.holst.so", ALICE), now)?;
    let staging = board_load(&dir, ("127.0.0.1_9", ALICE), now)?;
    let bob = board_load(&dir, ("app.holst.so", BOB), now)?;
    let run = env_without_cache(&dir)?;
    forget_cache(&run, Some(ALICE));
    let left = [&alice, &staging, &bob].map(|cache| cache.usage().boards);
    forget_cache(&run, None);
    let after = bob.usage().boards;
    std::fs::remove_dir_all(&dir).unwrap_or_default();
    assert_eq!(left, [0, 0, 1], "the account's cache outlived the logout");
    assert_eq!(after, 0, "an unknown account's logout left a cache");
    Ok(())
}
