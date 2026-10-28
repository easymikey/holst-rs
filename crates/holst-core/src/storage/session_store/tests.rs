#![expect(
    clippy::panic_in_result_fn,
    reason = "asserting is the point of a test"
)]

use std::{cell::RefCell, fs, io, path::PathBuf, sync::Arc, thread};

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
use crate::storage::session_store::keychain::keychain_error;
use crate::{
    domain::session::Session,
    effect::settings::SessionLocation,
    error::{Error, Result, local_data::LocalDataError},
    storage::session_store::{
        LoginRemoval,
        SessionFile,
        SessionStore,
        SessionStoreKind,
        clear_from,
        first_stored,
        resave_in,
        save_in,
        starting_at,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Health {
    Works,
    Broken,
    Locked,
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
    Absent,
}

#[derive(Debug)]
struct Memory {
    kind: SessionStoreKind,
    health: Health,
    value: RefCell<Option<String>>,
}

impl Memory {
    fn new(kind: SessionStoreKind, value: Option<&str>) -> Self {
        Self {
            kind,
            health: Health::Works,
            value: RefCell::new(value.map(str::to_owned)),
        }
    }

    fn with(kind: SessionStoreKind, health: Health) -> Self {
        Self {
            health,
            ..Self::new(kind, None)
        }
    }

    fn value(&self) -> Option<String> {
        self.value.borrow().clone()
    }
}

impl SessionStore for Memory {
    fn kind(&self) -> SessionStoreKind {
        self.kind
    }

    fn read(&self) -> Result<Option<SecretString>> {
        match self.health {
            Health::Locked => Err(Error::LocalData(LocalDataError::KeychainLocked)),
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                target_os = "openbsd"
            ))]
            Health::Absent => Err(absent()),
            Health::Works | Health::Broken => Ok(self.value().map(SecretString::from)),
        }
    }

    fn write(&self, stored: &SecretString) -> Result<()> {
        match self.health {
            Health::Broken => Err(Error::LocalData(LocalDataError::KeychainVolatile)),
            Health::Locked => Err(Error::LocalData(LocalDataError::KeychainLocked)),
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                target_os = "openbsd"
            ))]
            Health::Absent => Err(absent()),
            Health::Works => {
                self.value.replace(Some(stored.expose_secret().to_owned()));
                Ok(())
            }
        }
    }

    fn remove(&self) -> Result<LoginRemoval> {
        match (self.health, self.value.take()) {
            (Health::Locked, value) => {
                self.value.replace(value);
                Err(Error::LocalData(LocalDataError::KeychainLocked))
            }
            (_, Some(_)) => Ok(LoginRemoval::Removed),
            (_, None) => Ok(LoginRemoval::Absent),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
fn absent() -> Error {
    keychain_error(keyring::Error::PlatformFailure(Box::new(
        secret_service::Error::Unavailable,
    )))
}

fn holding(kind: SessionStoreKind, health: Health, value: &str) -> Memory {
    Memory {
        health,
        ..Memory::new(kind, Some(value))
    }
}

fn token_in(store: &Memory) -> Option<String> {
    store
        .value()
        .and_then(|raw| raw.parse::<Session>().ok())
        .map(|session| session.token().expose_secret().to_owned())
}

#[derive(Debug, PartialEq, Eq)]
enum ReadError {
    Locked,
    Corrupt,
    Other,
}

fn found(
    stores: &[&dyn SessionStore],
) -> std::result::Result<Option<(String, SessionStoreKind)>, ReadError> {
    first_stored(stores)
        .map(|found| {
            found.map(|(s, kind)| (s.token().expose_secret().to_owned(), kind))
        })
        .map_err(|err| match err {
            Error::LocalData(LocalDataError::KeychainLocked) => ReadError::Locked,
            Error::LocalData(LocalDataError::SavedLoginCorrupt) => ReadError::Corrupt,
            Error::LocalData(_)
            | Error::Login(_)
            | Error::Http(_)
            | Error::Input(_)
            | Error::Lookup(_)
            | Error::BoardSocket(_)
            | Error::BoardChange(_)
            | Error::Picture(_)
            | Error::StoredFile(_) => ReadError::Other,
        })
}

#[test]
fn first_readable_store_wins() {
    let keychain = Memory::new(SessionStoreKind::Keychain, None);
    let file = Memory::new(SessionStoreKind::File, Some("from-file"));
    assert_eq!(
        found(&[&keychain, &file]),
        Ok(Some(("from-file".into(), SessionStoreKind::File)))
    );
}

#[rstest]
#[case::unset(None, SessionLocation::System { home: None })]
#[case::empty(Some(""), SessionLocation::System { home: None })]
#[case::dir(Some("/tmp/h"), SessionLocation::Dir("/tmp/h".into()))]
fn location_comes_from_the_directory(
    #[case] dir: Option<&str>,
    #[case] expected: SessionLocation,
) {
    assert_eq!(SessionLocation::new(dir.map(PathBuf::from), None), expected);
}

#[test]
fn directory_location_never_touches_the_keychain() -> Result<()> {
    let dir = scratch("location")?;
    let location = SessionLocation::Dir(dir.clone());
    let kinds: Vec<_> = location.stores().iter().map(|s| s.kind()).collect();
    assert_eq!(kinds, [SessionStoreKind::File]);
    assert_eq!(location.file(), Some(dir.join("session")));
    assert!(location.load()?.is_none());
    let session = Session::with_cookies("t".into(), "s=1".parse().unwrap_or_default());
    assert_eq!(location.save(&session)?.store, SessionStoreKind::File);
    assert_eq!(location.load()?, Some((session, SessionStoreKind::File)));
    assert!(matches!(
        location.clear().as_slice(),
        [(SessionStoreKind::File, Ok(LoginRemoval::Removed))]
    ));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn locked_keychain_falls_back_to_the_file() {
    let locked = Memory::with(SessionStoreKind::Keychain, Health::Locked);
    let file = Memory::new(SessionStoreKind::File, Some("from-file"));
    let empty = Memory::new(SessionStoreKind::File, None);
    assert_eq!(
        found(&[&locked, &file]),
        Ok(Some(("from-file".into(), SessionStoreKind::File)))
    );
    assert_eq!(found(&[&locked, &empty]), Err(ReadError::Locked));
}

#[test]
fn corrupt_value_is_reported_only_when_nothing_else_is_found() {
    let corrupt = Memory::new(SessionStoreKind::Keychain, Some(r#"{"cookies":"x"}"#));
    let file = Memory::new(SessionStoreKind::File, Some("from-file"));
    let empty = Memory::new(SessionStoreKind::File, None);
    assert_eq!(
        found(&[&corrupt, &file]),
        Ok(Some(("from-file".into(), SessionStoreKind::File)))
    );
    assert_eq!(found(&[&corrupt, &empty]), Err(ReadError::Corrupt));
    assert_eq!(found(&[&empty]), Ok(None));
}

#[test]
fn save_prefers_primary_and_drops_stale_copy() -> Result<()> {
    let keychain = Memory::new(SessionStoreKind::Keychain, None);
    let file = Memory::new(SessionStoreKind::File, Some("old"));
    let saved = save_in(&[&keychain, &file], &Session::new("new".into()))?;
    assert_eq!(saved.store, SessionStoreKind::Keychain);
    assert!(saved.stale.is_none(), "{saved:?}");
    assert_eq!(
        (token_in(&keychain), file.value()),
        (Some("new".into()), None)
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Staleness {
    Reported,
    Absent,
}

#[rstest]
#[case::broken(Health::Broken, Staleness::Absent)]
#[case::locked(Health::Locked, Staleness::Reported)]
fn save_falls_back_when_primary_fails(
    #[case] health: Health,
    #[case] staleness: Staleness,
) -> Result<()> {
    let keychain = Memory::with(SessionStoreKind::Keychain, health);
    let file = Memory::new(SessionStoreKind::File, None);
    let session = Session::with_cookies("t".into(), "s=1".parse().unwrap_or_default());
    let saved = save_in(&[&keychain, &file], &session)?;
    assert_eq!(
        (
            saved.store,
            saved
                .stale
                .map_or(Staleness::Absent, |_| Staleness::Reported)
        ),
        (SessionStoreKind::File, staleness)
    );
    assert_eq!(
        file.value().and_then(|raw| raw.parse::<Session>().ok()),
        Some(session)
    );
    Ok(())
}

#[test]
fn stale_copy_read_first_is_reported() -> Result<()> {
    let keychain = holding(SessionStoreKind::Keychain, Health::Locked, "old");
    let file = Memory::new(SessionStoreKind::File, None);
    let saved = save_in(&[&keychain, &file], &Session::new("new".into()))?;
    assert_eq!(saved.store, SessionStoreKind::File);
    assert_eq!(token_in(&file).as_deref(), Some("new"));
    let stale = saved.stale.map(|err| (err.to_string(), err.hint()));
    let expected = format!(
        "an older login may be left in the {}, which is read before the new one",
        SessionStoreKind::Keychain.as_str()
    );
    assert_eq!(
        stale,
        Some((expected, Some(crate::error::hint::Hint::ReplaceStale)))
    );
    Ok(())
}

#[test]
fn stale_copy_read_later_is_not_reported() -> Result<()> {
    let keychain = Memory::new(SessionStoreKind::Keychain, None);
    let file = holding(SessionStoreKind::File, Health::Locked, "old");
    let saved = save_in(&[&keychain, &file], &Session::new("new".into()))?;
    assert_eq!(
        (saved.store, saved.stale.is_none()),
        (SessionStoreKind::Keychain, true)
    );
    Ok(())
}

#[test]
fn rotation_stays_in_the_store_it_came_from() -> Result<()> {
    let keychain = Memory::new(SessionStoreKind::Keychain, None);
    let file = Memory::new(SessionStoreKind::File, Some("old"));
    let stores: [&dyn SessionStore; 2] = [&keychain, &file];
    let rotated = Session::new("rotated".into());
    let saved = save_in(starting_at(&stores, SessionStoreKind::File), &rotated)?;
    assert_eq!(saved.store, SessionStoreKind::File);
    assert_eq!(
        (keychain.value(), token_in(&file)),
        (None, Some("rotated".into()))
    );
    let moved_back =
        save_in(starting_at(&stores, SessionStoreKind::Keychain), &rotated)?;
    assert_eq!(
        (moved_back.store, file.value()),
        (SessionStoreKind::Keychain, None)
    );
    Ok(())
}

#[rstest]
#[case::same_login(Some("t"), Some("s=rotated"))]
#[case::logged_out(None, None)]
#[case::other_login(Some("other"), None)]
fn rotation_of_a_replaced_login_is_not_saved(
    #[case] held: Option<&str>,
    #[case] saved_cookies: Option<&str>,
) {
    let file = Memory::new(SessionStoreKind::File, held);
    let rotated =
        Session::with_cookies("t".into(), "s=rotated".parse().unwrap_or_default());
    let saved = resave_in(&[&file], &rotated);
    assert!(
        saved.is_ok()
            || matches!(saved, Err(Error::LocalData(LocalDataError::LoginReplaced))),
        "{saved:?}"
    );
    let cookies = file
        .value()
        .and_then(|raw| raw.parse::<Session>().ok())
        .map(|kept| kept.cookies().header().expose_secret().to_owned())
        .filter(|_| saved.is_ok());
    assert_eq!(cookies.as_deref(), saved_cookies);
    assert_eq!(token_in(&file).as_deref(), held);
}

#[test]
fn rotation_into_an_unreadable_store_falls_back_as_before() -> Result<()> {
    let keychain = Memory::with(SessionStoreKind::Keychain, Health::Locked);
    let file = Memory::new(SessionStoreKind::File, None);
    let saved = resave_in(&[&keychain, &file], &Session::new("t".into()))?;
    assert_eq!(
        (saved.store, token_in(&file).as_deref()),
        (SessionStoreKind::File, Some("t"))
    );
    Ok(())
}

#[test]
fn rotation_in_a_directory_location_stays_in_its_file() -> Result<()> {
    let dir = scratch("resave")?;
    let location = SessionLocation::Dir(dir.clone());
    let session = Session::new("t".into());
    location.save(&session)?;
    let kinds = [SessionStoreKind::Keychain, SessionStoreKind::File]
        .map(|from| location.resave(&session, from).map(|saved| saved.store));
    assert!(
        kinds
            .iter()
            .all(|kind| matches!(kind, Ok(SessionStoreKind::File))),
        "{kinds:?}"
    );
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
#[test]
fn missing_secret_service_is_reported_not_fallen_back() {
    let keychain = Memory::with(SessionStoreKind::Keychain, Health::Absent);
    let file = Memory::new(SessionStoreKind::File, None);
    let saved = save_in(&[&keychain, &file], &Session::new("t".into()));
    assert!(
        matches!(
            saved,
            Err(Error::LocalData(LocalDataError::NoSecretService(_)))
        ),
        "{saved:?}"
    );
    assert!(file.value().is_none());
    assert!(found(&[&keychain, &file]).is_err());
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
#[test]
fn missing_secret_service_keeps_using_the_file() -> Result<()> {
    let keychain = Memory::with(SessionStoreKind::Keychain, Health::Absent);
    let file = Memory::new(SessionStoreKind::File, Some("from-file"));
    let stores: [&dyn SessionStore; 2] = [&keychain, &file];
    assert_eq!(
        found(&stores),
        Ok(Some(("from-file".into(), SessionStoreKind::File)))
    );
    let rotated = Session::new("new".into());
    let saved = save_in(starting_at(&stores, SessionStoreKind::File), &rotated)?;
    assert_eq!(
        (saved.store, saved.stale.is_none()),
        (SessionStoreKind::File, true)
    );
    assert_eq!(token_in(&file).as_deref(), Some("new"));
    let cleared: Vec<_> = clear_from(&stores)
        .into_iter()
        .map(|(kind, removed)| (kind, removed.ok()))
        .collect();
    assert_eq!(
        cleared,
        [
            (SessionStoreKind::Keychain, Some(LoginRemoval::Absent)),
            (SessionStoreKind::File, Some(LoginRemoval::Removed))
        ]
    );
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
#[test]
fn secret_service_failures_are_told_apart() {
    let failure = |cause: secret_service::Error| {
        keychain_error(keyring::Error::PlatformFailure(Box::new(cause)))
    };
    let missing = failure(secret_service::Error::Unavailable);
    assert!(
        matches!(
            missing,
            Error::LocalData(LocalDataError::NoSecretService(_))
        ),
        "{missing:?}"
    );
    let other = failure(secret_service::Error::Crypto("bad key"));
    assert!(
        matches!(other, Error::LocalData(LocalDataError::SecretService(_))),
        "{other:?}"
    );
    assert_eq!(
        other.to_string(),
        "Secret Service failed: Crypto error: bad key"
    );
    assert_eq!(other.hint(), Some(crate::error::hint::Hint::UseHome));
}

#[test]
fn clear_reports_what_each_store_found() {
    let keychain = Memory::new(SessionStoreKind::Keychain, Some("t"));
    let file = Memory::new(SessionStoreKind::File, None);
    let locked = holding(SessionStoreKind::Keychain, Health::Locked, "t");
    let outcome = |stores: &[&dyn SessionStore]| {
        clear_from(stores)
            .into_iter()
            .map(|(kind, removed)| (kind, removed.ok()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        outcome(&[&keychain, &file]),
        [
            (SessionStoreKind::Keychain, Some(LoginRemoval::Removed)),
            (SessionStoreKind::File, Some(LoginRemoval::Absent))
        ]
    );
    assert!(matches!(
        clear_from(&[&locked]).as_slice(),
        [(
            SessionStoreKind::Keychain,
            Err(Error::LocalData(LocalDataError::KeychainLocked))
        )]
    ));
    assert_eq!(locked.value().as_deref(), Some("t"));
}

fn scratch(test: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join(format!("holst-core-store-{test}-{}", std::process::id()));
    fs::remove_dir_all(&dir).or_else(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(err)
        }
    })?;
    Ok(dir)
}

#[test]
fn session_file_round_trips_privately() -> Result<()> {
    let dir = scratch("round-trip")?;
    let store = SessionFile::at(dir.join("holst/token"));
    assert!(store.read()?.is_none());
    store.write(&SecretString::from("abc"))?;
    let back = store.read()?.map(|s| s.expose_secret().trim().to_owned());
    assert_eq!(back.as_deref(), Some("abc"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(dir.join("holst/token"))?.permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert_eq!(store.remove()?, LoginRemoval::Removed);
    assert_eq!(store.remove()?, LoginRemoval::Absent);
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn racing_writes_never_tear_the_file() -> Result<()> {
    let dir = scratch("race")?;
    let path = dir.join("token");
    let store = Arc::new(SessionFile::at(path.clone()));
    let tokens: Vec<String> = (0..8)
        .map(|n| format!("token-{n}-{}", "x".repeat(4096)))
        .collect();
    let writers: Vec<_> = tokens
        .iter()
        .cloned()
        .map(|token| {
            let store = Arc::clone(&store);
            thread::spawn(move || store.write(&SecretString::from(token)))
        })
        .collect();
    writers.into_iter().try_for_each(|writer| {
        writer
            .join()
            .unwrap_or(Err(Error::LocalData(LocalDataError::KeychainLocked)))
    })?;
    let left = fs::read_to_string(&path)?;
    assert!(
        tokens.contains(&left.trim().to_owned()),
        "torn file: {} bytes",
        left.len()
    );
    let names: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .collect();
    assert_eq!(names, ["token"]);
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn no_home_means_no_file() {
    let store = SessionFile::default();
    assert!(matches!(store.read(), Ok(None)));
    assert!(matches!(
        store.write(&SecretString::from("t")),
        Err(Error::LocalData(LocalDataError::NoHome))
    ));
}

#[test]
fn a_login_file_that_cannot_be_read_names_its_path() -> Result<()> {
    let dir = scratch("read-fails")?;
    let path = dir.join("token");
    fs::create_dir_all(&path)?;
    let failed = SessionFile::at(path.clone()).read();
    let failure = read_failure(&failed);
    assert_eq!(failure.map(|(named, _)| named), Some(&path), "{failed:?}");
    assert_ne!(failure.map(|(_, kind)| kind), Some(io::ErrorKind::NotFound));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_login_file_whose_folder_cannot_be_made_names_the_file() -> Result<()> {
    let dir = scratch("write-fails")?;
    fs::create_dir_all(&dir)?;
    let blocker = dir.join("holst");
    fs::write(&blocker, b"a file where the folder should go")?;
    let path = blocker.join("token");
    let failed = SessionFile::at(path.clone()).write(&SecretString::from("t"));
    assert_eq!(write_failure(&failed), Some(&path), "{failed:?}");
    assert!(!path.exists());
    fs::remove_dir_all(&dir)?;
    Ok(())
}

fn location_with(test: &str, session: &Session) -> Result<(PathBuf, SessionLocation)> {
    let dir = scratch(test)?;
    let location = SessionLocation::Dir(dir.clone());
    location.save(session)?;
    Ok((dir, location))
}

fn stored_token(location: &SessionLocation) -> Result<Option<String>> {
    Ok(location
        .load()?
        .map(|(session, _)| session.token().expose_secret().to_owned()))
}

#[test]
fn a_held_lock_blocks_a_fresh_save() -> Result<()> {
    let dir = scratch("save-lock")?;
    let location = SessionLocation::Dir(dir.clone());
    let holder = location.lock()?;
    let saved = location.save(&Session::new("t".into()));
    drop(holder);
    assert!(
        matches!(
            saved,
            Err(Error::LocalData(LocalDataError::LoginBusy { .. }))
        ),
        "{saved:?}"
    );
    assert!(!dir.join("session").exists(), "the save ignored the lock");
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_held_lock_blocks_a_resave() -> Result<()> {
    let session = Session::new("t".into());
    let (dir, location) = location_with("resave-lock", &session)?;
    let holder = location.lock()?;
    let resaved = location.resave(&Session::new("u".into()), SessionStoreKind::File);
    drop(holder);
    assert!(
        matches!(
            resaved,
            Err(Error::LocalData(LocalDataError::LoginBusy { .. }))
        ),
        "{resaved:?}"
    );
    assert_eq!(stored_token(&location)?.as_deref(), Some("t"));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_held_lock_blocks_a_logout() -> Result<()> {
    let (dir, location) = location_with("clear-lock", &Session::new("t".into()))?;
    let holder = location.lock()?;
    let cleared = location.clear();
    drop(holder);
    assert!(
        matches!(
            cleared.as_slice(),
            [(
                SessionStoreKind::File,
                Err(Error::LocalData(LocalDataError::LoginBusy { .. }))
            )]
        ),
        "{cleared:?}"
    );
    assert!(
        cleared
            .iter()
            .all(|(_, found)| found.as_ref().is_err_and(|err| err.hint().is_none())),
        "a busy lock is not a keychain problem: {cleared:?}"
    );
    assert_eq!(stored_token(&location)?.as_deref(), Some("t"));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_stale_resave_after_a_logout_leaves_no_session() -> Result<()> {
    let session = Session::new("t".into());
    let (dir, logout) = location_with("logout-then-resave", &session)?;
    let rotation = SessionLocation::Dir(dir.clone());
    assert_eq!(logout.clear().len(), 1);
    let resaved = rotation.resave(&session, SessionStoreKind::File);
    assert!(
        matches!(
            resaved,
            Err(Error::LocalData(LocalDataError::LoginReplaced))
        ),
        "{resaved:?}"
    );
    assert_eq!(stored_token(&logout)?, None);
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_fresh_save_racing_a_stale_resave_keeps_the_fresh_login() -> Result<()> {
    let stale = Session::new("t".into());
    let fresh = Session::new("u".into());
    let (dir, location) = location_with("save-vs-resave", &stale)?;
    let rotation = SessionLocation::Dir(dir.clone());
    let holder = location.lock()?;
    let (resaved, saved) = thread::scope(|scope| {
        let resaving = scope.spawn(|| rotation.resave(&stale, SessionStoreKind::File));
        let saving = scope.spawn(|| location.save(&fresh));
        drop(holder);
        (resaving.join().ok(), saving.join().ok())
    });
    assert!(matches!(saved, Some(Ok(_))), "{saved:?}");
    assert!(
        matches!(
            resaved,
            Some(Ok(_) | Err(Error::LocalData(LocalDataError::LoginReplaced)))
        ),
        "{resaved:?}"
    );
    assert_eq!(stored_token(&location)?.as_deref(), Some("u"));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

fn with_unusable_lock(test: &str) -> Result<(PathBuf, SessionLocation, Session)> {
    let session = Session::new("t".into());
    let (dir, location) = location_with(test, &session)?;
    // A directory cannot be opened as a lock file, even by root.
    fs::remove_file(dir.join("LOCK"))?;
    fs::create_dir(dir.join("LOCK"))?;
    Ok((dir, location, session))
}

#[test]
fn a_lock_that_cannot_be_created_still_lets_a_person_log_out() -> Result<()> {
    let (dir, location, _) = with_unusable_lock("clear-no-lock")?;
    let cleared = location.clear();
    assert!(
        matches!(
            cleared.as_slice(),
            [(SessionStoreKind::File, Ok(LoginRemoval::Removed))]
        ),
        "{cleared:?}"
    );
    assert_eq!(stored_token(&location)?, None);
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_lock_that_cannot_be_created_fails_a_resave_with_its_cause() -> Result<()> {
    let (dir, location, session) = with_unusable_lock("resave-no-lock")?;
    let resaved = location.resave(&session, SessionStoreKind::File);
    assert!(
        matches!(
            resaved,
            Err(Error::LocalData(LocalDataError::FileWrite { .. }))
        ),
        "{resaved:?}"
    );
    assert!(
        resaved
            .err()
            .is_some_and(|err| err.to_string().contains("LOCK")),
        "the error does not name the lock file"
    );
    fs::remove_dir_all(&dir)?;
    Ok(())
}

/// Returns the file a failed read names and how it failed, when it failed so.
fn read_failure<T>(result: &Result<T>) -> Option<(&PathBuf, io::ErrorKind)> {
    if let Err(Error::LocalData(LocalDataError::FileRead { path, error })) = result {
        Some((path, error.kind()))
    } else {
        None
    }
}

/// Returns the file a failed write names, when it failed so.
fn write_failure<T>(result: &Result<T>) -> Option<&PathBuf> {
    if let Err(Error::LocalData(LocalDataError::FileWrite { path, .. })) = result {
        Some(path)
    } else {
        None
    }
}
