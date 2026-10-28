//! Everything holst keeps on this machine: where, how much, and what may go.

use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

use crate::{
    effect::settings::SessionLocation,
    error::{Error, Result, local_data::LocalDataError},
    storage::{
        file::{subdirs, tree_size},
        session_store::SessionStoreKind,
    },
};

pub const CONFIG_DIR: &str = ".config/holst";

const BROWSER_PROFILE: &str = "browser";

const CONFIG_SESSION_FILE: &str = "token";

pub(crate) const HOME_SESSION_FILE: &str = "session";

/// What a place holds, which decides whether `clean` removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlaceRole {
    SavedLogin,
    StaleLogin,
    OtherLogin,
    BrowserProfile,
    Partial,
    Cache,
    OldCache,
}

impl PlaceRole {
    /// What the role means, for a person.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::SavedLogin => "the saved login",
            Self::StaleLogin => "an old copy of the login; the keychain one is read",
            Self::OtherLogin => "the login file used without HOLST_HOME",
            Self::BrowserProfile => "profile of the removed browser login",
            Self::Partial => "a temporary file left by an interrupted login write",
            Self::Cache => "cached boards of one account, colleagues' content included",
            Self::OldCache => "the board cache of another holst version",
        }
    }
}

/// How much `clean` removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanMode {
    Leftovers,
    Cache,
    All,
}

/// Whether `clean` in `scope` removes a place of `role`.
#[must_use]
pub const fn is_removable(role: PlaceRole, scope: CleanMode) -> bool {
    match (role, scope) {
        (
            PlaceRole::StaleLogin
            | PlaceRole::BrowserProfile
            | PlaceRole::Partial
            | PlaceRole::OldCache,
            _,
        )
        | (PlaceRole::SavedLogin, CleanMode::All)
        | (PlaceRole::Cache, CleanMode::Cache | CleanMode::All) => true,
        (PlaceRole::SavedLogin, CleanMode::Leftovers | CleanMode::Cache)
        | (PlaceRole::Cache, CleanMode::Leftovers)
        | (PlaceRole::OtherLogin, _) => false,
    }
}

/// One place where holst keeps data: the keychain item or a file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataPlace {
    Keychain { name: String },
    File(FilePlace),
}

/// A file or directory of holst's, right inside its root.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FilePlace {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub role: PlaceRole,
    #[serde(skip)]
    pub root: PathBuf,
}

impl DataPlace {
    /// What the place is called for a person.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Keychain { name } => name,
            Self::File(file) => &file.name,
        }
    }

    #[must_use]
    pub const fn role(&self) -> PlaceRole {
        match self {
            Self::Keychain { .. } => PlaceRole::SavedLogin,
            Self::File(file) => file.role,
        }
    }

    /// The size on disk; the keychain item has none.
    #[must_use]
    pub const fn bytes(&self) -> Option<u64> {
        match self {
            Self::Keychain { .. } => None,
            Self::File(file) => Some(file.bytes),
        }
    }

    /// The file, unless this is the keychain item (which `logout` removes).
    #[must_use]
    pub const fn file(&self) -> Option<&FilePlace> {
        match self {
            Self::Keychain { .. } => None,
            Self::File(file) => Some(file),
        }
    }
}

/// Keeps the JSON keys: the keychain item has null `path` and `bytes`.
impl serde::Serialize for DataPlace {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;

        match self {
            Self::File(file) => file.serialize(serializer),
            Self::Keychain { name } => {
                let mut item = serializer.serialize_struct("DataPlace", 4)?;
                item.serialize_field("path", &None::<PathBuf>)?;
                item.serialize_field("name", name)?;
                item.serialize_field("bytes", &None::<u64>)?;
                item.serialize_field("role", &PlaceRole::SavedLogin)?;
                item.end()
            }
        }
    }
}

/// Where to look: home, login location, and the store the login came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoots {
    pub home: Option<PathBuf>,
    pub location: SessionLocation,
    pub session_store: Option<SessionStoreKind>,
    pub caches: Vec<PathBuf>,
}

impl DataRoots {
    /// The directory of holst's own files.
    #[must_use]
    pub fn config_dir(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|home| home.join(CONFIG_DIR))
    }
}

/// Every place holst keeps data in, with sizes; reads only metadata.
#[must_use]
pub fn places(roots: &DataRoots) -> Vec<DataPlace> {
    gather(roots, true)
}

/// The places `clean` in `scope` removes by path; the accounts' caches are not
/// walked when `scope` keeps them.
#[must_use]
pub fn removable_places(roots: &DataRoots, scope: CleanMode) -> Vec<FilePlace> {
    gather(roots, is_removable(PlaceRole::Cache, scope))
        .into_iter()
        .filter_map(|place| match place {
            DataPlace::File(file) if is_removable(file.role, scope) => Some(file),
            DataPlace::File(_) | DataPlace::Keychain { .. } => None,
        })
        .collect()
}

fn gather(roots: &DataRoots, accounts: bool) -> Vec<DataPlace> {
    let config = roots.config_dir();
    let keychain = matches!(
        (&roots.location, roots.session_store),
        (
            SessionLocation::System { .. },
            Some(SessionStoreKind::Keychain)
        )
    )
    .then(|| DataPlace::Keychain {
        name: format!(
            "{} item holst-rs/session-token",
            SessionStoreKind::Keychain.as_str()
        ),
    });
    let token_role = match (&roots.location, roots.session_store) {
        (SessionLocation::Dir(_), _) => PlaceRole::OtherLogin,
        (SessionLocation::System { .. }, Some(SessionStoreKind::Keychain)) => {
            PlaceRole::StaleLogin
        }
        (SessionLocation::System { .. }, _) => PlaceRole::SavedLogin,
    };
    let in_config = config.iter().flat_map(|config| {
        [
            found(config, CONFIG_SESSION_FILE, token_role),
            found(config, BROWSER_PROFILE, PlaceRole::BrowserProfile),
        ]
        .into_iter()
        .flatten()
        .chain(partials(config, CONFIG_SESSION_FILE))
    });
    let in_home = match &roots.location {
        SessionLocation::Dir(dir) => {
            found(dir, HOME_SESSION_FILE, PlaceRole::SavedLogin)
                .into_iter()
                .chain(partials(dir, HOME_SESSION_FILE))
                .collect()
        }
        SessionLocation::System { .. } => Vec::new(),
    };
    keychain
        .into_iter()
        .chain(in_config)
        .chain(in_home)
        .chain(roots.caches.iter().flat_map(|root| cached(root, accounts)))
        .collect()
}

fn cached(root: &Path, accounts: bool) -> Vec<DataPlace> {
    let layout = root.join(crate::storage::cache::files::LAYOUT);
    let accounts = subdirs(&layout)
        .into_iter()
        .filter(|_| accounts)
        .flat_map(|host| {
            subdirs(&host)
                .into_iter()
                .map(move |account| place(&host, account, PlaceRole::Cache))
        });
    let old = crate::storage::cache::Cache::other_layouts(root)
        .into_iter()
        .map(|path| place(root, path, PlaceRole::OldCache));
    accounts.chain(old).collect()
}

fn found(dir: &Path, name: &str, role: PlaceRole) -> Option<DataPlace> {
    let path = dir.join(name);
    fs::symlink_metadata(&path)
        .ok()
        .map(|_| place(dir, path, role))
}

fn place(dir: &Path, path: PathBuf, role: PlaceRole) -> DataPlace {
    DataPlace::File(FilePlace {
        name: path.display().to_string(),
        bytes: tree_size(&path),
        path,
        role,
        root: dir.to_path_buf(),
    })
}

fn partials<'a>(dir: &'a Path, name: &str) -> impl Iterator<Item = DataPlace> + 'a {
    let prefix = format!(".{name}.");
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(move |entry| {
            entry.file_name().to_str().is_some_and(|file| {
                crate::storage::file::is_partial(file)
                    || crate::storage::file::is_pid_stamped(file, &prefix, ".tmp")
            })
        })
        .map(move |entry| place(dir, entry.path(), PlaceRole::Partial))
}

/// The places `clean` in `scope` removes by path (a keychain item goes by
/// `logout`).
#[must_use]
pub fn to_remove(places: &[DataPlace], scope: CleanMode) -> Vec<&FilePlace> {
    places
        .iter()
        .filter_map(DataPlace::file)
        .filter(|file| is_removable(file.role, scope))
        .collect()
}

/// Remove a place that sits right in its root; neither may be a symbolic link.
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::Symlink`],
/// [`crate::error::local_data::LocalDataError::OutsideLocalData`] or
/// [`crate::error::local_data::LocalDataError::FileRemove`].
pub fn remove(place: &FilePlace) -> Result<()> {
    let (path, root) = (place.path.as_path(), place.root.as_path());
    let failed = |source| {
        Error::LocalData(LocalDataError::FileRemove {
            path: path.to_path_buf(),
            error: source,
        })
    };
    [root, path].into_iter().try_for_each(|checked| {
        let link = fs::symlink_metadata(checked).map_err(failed)?.is_symlink();
        if link {
            Err(Error::LocalData(LocalDataError::Symlink {
                path: checked.to_path_buf(),
            }))
        } else {
            Ok(())
        }
    })?;
    let inside = path
        .parent()
        .map(fs::canonicalize)
        .transpose()
        .map_err(failed)?
        .zip(fs::canonicalize(root).ok())
        .is_some_and(|(parent, root)| parent == root);
    if inside {
        let removed = if fs::symlink_metadata(path).map_err(failed)?.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        removed.or_else(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(failed(err))
            }
        })
    } else {
        Err(Error::LocalData(LocalDataError::OutsideLocalData {
            path: path.to_path_buf(),
        }))
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::{fs, io, path::PathBuf};

    use itertools::Itertools as _;
    use rstest::rstest;

    use crate::{
        effect::settings::SessionLocation,
        error::{Error, Result, local_data::LocalDataError},
        storage::{
            local_data::{
                BROWSER_PROFILE,
                CONFIG_DIR,
                CONFIG_SESSION_FILE,
                CleanMode,
                DataPlace,
                DataRoots,
                FilePlace,
                HOME_SESSION_FILE,
                PlaceRole,
                is_removable,
                places,
                remove,
                to_remove,
            },
            session_store::SessionStoreKind,
        },
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Expectation {
        Holds,
        Fails,
    }

    impl Expectation {
        fn holds(self) -> bool {
            matches!(self, Self::Holds)
        }
    }

    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> io::Result<Self> {
            let dir = std::env::temp_dir()
                .join(format!("holst-storage-{name}-{}", std::process::id()));
            fs::remove_dir_all(&dir).or_else(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(err)
                }
            })?;
            fs::create_dir_all(
                dir.join(CONFIG_DIR).join(BROWSER_PROFILE).join("Default"),
            )?;
            Ok(Self(dir))
        }

        fn config(&self) -> PathBuf {
            self.0.join(CONFIG_DIR)
        }

        fn roots(
            &self,
            location: SessionLocation,
            login: Option<SessionStoreKind>,
        ) -> DataRoots {
            DataRoots {
                home: Some(self.0.clone()),
                location,
                session_store: login,
                caches: vec![self.0.join("cache")],
            }
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            if let Err(err) = fs::remove_dir_all(&self.0) {
                tracing::debug!(%err, "test home not removed");
            }
        }
    }

    fn roles(places: &[DataPlace]) -> Vec<(String, PlaceRole)> {
        places
            .iter()
            .map(|place| {
                let name = place
                    .file()
                    .and_then(|file| file.path.file_name())
                    .map_or_else(
                        || "keychain".to_owned(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                (name, place.role())
            })
            .collect()
    }

    #[rstest]
    #[case::keychain_login(
        Some(SessionStoreKind::Keychain),
        PlaceRole::StaleLogin,
        Expectation::Holds
    )]
    #[case::file_login(
        Some(SessionStoreKind::File),
        PlaceRole::SavedLogin,
        Expectation::Fails
    )]
    #[case::unreadable(None, PlaceRole::SavedLogin, Expectation::Fails)]
    fn the_token_file_is_stale_only_beside_a_keychain_login(
        #[case] login: Option<SessionStoreKind>,
        #[case] role: PlaceRole,
        #[case] keychain: Expectation,
    ) -> io::Result<()> {
        let home = Home::new(&format!("token-{login:?}"))?;
        fs::write(home.config().join(CONFIG_SESSION_FILE), "{\"token\":\"t\"}")?;
        let found = roles(&places(
            &home.roots(SessionLocation::System { home: None }, login),
        ));
        assert!(found.contains(&("token".to_owned(), role)), "{found:?}");
        assert_eq!(
            found.iter().any(|(name, _)| name == "keychain"),
            keychain.holds()
        );
        Ok(())
    }

    #[test]
    fn everything_is_listed_with_sizes() -> io::Result<()> {
        let home = Home::new("list")?;
        let profile = home.config().join(BROWSER_PROFILE);
        fs::write(profile.join("Default").join("Cookies"), [0u8; 1000])?;
        fs::write(profile.join("Local State"), [0u8; 24])?;
        fs::write(home.config().join(".token.12-3.tmp"), "x")?;
        fs::write(home.config().join(".token.x-3.tmp"), "not ours")?;
        let dir = home.0.join("hh");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(HOME_SESSION_FILE), "{}")?;
        fs::write(dir.join(".session.9-0.tmp"), "x")?;
        fs::write(dir.join(".holst-9-1.partial"), "x")?;
        fs::write(dir.join("notes.txt"), "the user's")?;
        let found = places(&home.roots(SessionLocation::Dir(dir), None));
        let named: Vec<_> = roles(&found)
            .into_iter()
            .sorted_by(|a, b| a.0.cmp(&b.0))
            .collect();
        assert_eq!(
            named,
            [
                (".holst-9-1.partial".to_owned(), PlaceRole::Partial),
                (".session.9-0.tmp".to_owned(), PlaceRole::Partial),
                (".token.12-3.tmp".to_owned(), PlaceRole::Partial),
                ("browser".to_owned(), PlaceRole::BrowserProfile),
                ("session".to_owned(), PlaceRole::SavedLogin),
            ]
        );
        let browser = found
            .iter()
            .find(|place| place.role() == PlaceRole::BrowserProfile);
        assert_eq!(browser.and_then(DataPlace::bytes), Some(1024));
        Ok(())
    }

    #[rstest]
    #[case(PlaceRole::SavedLogin, CleanMode::Leftovers, Expectation::Fails)]
    #[case(PlaceRole::SavedLogin, CleanMode::All, Expectation::Holds)]
    #[case(PlaceRole::OtherLogin, CleanMode::All, Expectation::Fails)]
    #[case(PlaceRole::StaleLogin, CleanMode::Leftovers, Expectation::Holds)]
    #[case(PlaceRole::BrowserProfile, CleanMode::Leftovers, Expectation::Holds)]
    #[case(PlaceRole::Partial, CleanMode::Leftovers, Expectation::Holds)]
    #[case(PlaceRole::Cache, CleanMode::Leftovers, Expectation::Fails)]
    #[case(PlaceRole::Cache, CleanMode::Cache, Expectation::Holds)]
    #[case(PlaceRole::Cache, CleanMode::All, Expectation::Holds)]
    #[case(PlaceRole::SavedLogin, CleanMode::Cache, Expectation::Fails)]
    #[case(PlaceRole::OldCache, CleanMode::Leftovers, Expectation::Holds)]
    fn clean_keeps_the_login_unless_all(
        #[case] role: PlaceRole,
        #[case] scope: CleanMode,
        #[case] expected: Expectation,
    ) {
        assert_eq!(is_removable(role, scope), expected.holds());
    }

    #[test]
    fn clean_removes_leftovers_and_keeps_the_login() -> Result<()> {
        let home = Home::new("clean")?;
        fs::write(home.config().join(CONFIG_SESSION_FILE), "{}")?;
        fs::write(home.config().join(".token.1-1.tmp"), "x")?;
        let found = places(&home.roots(
            SessionLocation::System { home: None },
            Some(SessionStoreKind::File),
        ));
        to_remove(&found, CleanMode::Leftovers)
            .into_iter()
            .try_for_each(remove)?;
        assert!(!home.config().join(BROWSER_PROFILE).exists());
        assert!(!home.config().join(".token.1-1.tmp").exists());
        assert!(
            home.config().join(CONFIG_SESSION_FILE).exists(),
            "the login was removed"
        );
        Ok(())
    }

    #[test]
    fn the_cache_is_listed_per_account() -> Result<()> {
        let home = Home::new("cache")?;
        let cache = home.0.join("cache");
        let account = cache
            .join("v2")
            .join("app.holst.so")
            .join("eeeeeeee-0000-4000-8000-000000000002");
        fs::create_dir_all(account.join("boards"))?;
        fs::write(account.join("boards").join("x"), [0u8; 100])?;
        let staging = cache
            .join("v2")
            .join("127.0.0.1_9")
            .join("aaaaaaaa-0000-4000-8000-000000000001");
        fs::create_dir_all(&staging)?;
        fs::write(cache.join("v2").join(".DS_Store"), "finder")?;
        fs::create_dir_all(cache.join("v1").join("old"))?;
        let found = places(&home.roots(SessionLocation::System { home: None }, None));
        let cached: Vec<(String, PlaceRole, Option<u64>)> = found
            .iter()
            .filter(|place| {
                matches!(place.role(), PlaceRole::Cache | PlaceRole::OldCache)
            })
            .map(|place| {
                let name = place.file().and_then(|file| file.path.file_name());
                let name = name.map(|n| n.to_string_lossy().into_owned());
                (name.unwrap_or_default(), place.role(), place.bytes())
            })
            .sorted_by(|a, b| a.0.cmp(&b.0))
            .collect();
        assert_eq!(
            cached,
            [
                (
                    "aaaaaaaa-0000-4000-8000-000000000001".to_owned(),
                    PlaceRole::Cache,
                    Some(0)
                ),
                (
                    "eeeeeeee-0000-4000-8000-000000000002".to_owned(),
                    PlaceRole::Cache,
                    Some(100)
                ),
                ("v1".to_owned(), PlaceRole::OldCache, Some(0)),
            ]
        );
        let kept: Vec<PlaceRole> = to_remove(&found, CleanMode::Leftovers)
            .iter()
            .map(|file| file.role)
            .collect();
        assert!(!kept.contains(&PlaceRole::Cache), "{kept:?}");
        to_remove(&found, CleanMode::Cache)
            .into_iter()
            .try_for_each(remove)?;
        assert!(!account.exists() && !staging.exists() && !cache.join("v1").exists());
        Ok(())
    }

    #[test]
    fn a_crashed_write_is_cleaned() -> Result<()> {
        let home = Home::new("crashed")?;
        let dir = home.0.join("hh");
        fs::create_dir_all(&dir)?;
        crate::storage::file::replace_private(&dir.join(HOME_SESSION_FILE), &[b"{}"])?;
        let crashed = dir.join(".holst-4242-7.partial");
        fs::write(&crashed, "torn")?;
        let found = places(&home.roots(SessionLocation::Dir(dir.clone()), None));
        to_remove(&found, CleanMode::Leftovers)
            .into_iter()
            .try_for_each(remove)?;
        assert!(!crashed.exists());
        assert_eq!(fs::read(dir.join(HOME_SESSION_FILE))?, b"{}");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_never_followed() -> Result<()> {
        let home = Home::new("symlink")?;
        let outside = home.0.join("outside");
        fs::create_dir_all(&outside)?;
        fs::write(outside.join("precious"), "keep")?;
        fs::remove_dir_all(home.config().join(BROWSER_PROFILE))?;
        std::os::unix::fs::symlink(&outside, home.config().join(BROWSER_PROFILE))?;
        let found = places(&home.roots(SessionLocation::System { home: None }, None));
        let removed: Vec<_> = to_remove(&found, CleanMode::Leftovers)
            .into_iter()
            .map(remove)
            .collect();
        assert!(
            matches!(
                removed.as_slice(),
                [Err(Error::LocalData(LocalDataError::Symlink { .. }))]
            ),
            "{removed:?}"
        );
        assert!(outside.join("precious").exists());

        let moved = home.0.join("real-config");
        fs::rename(home.config(), &moved)?;
        std::os::unix::fs::symlink(&moved, home.config())?;
        fs::create_dir_all(moved.join("x"))?;
        fs::write(moved.join(".token.1-1.tmp"), "x")?;
        let through = places(&home.roots(SessionLocation::System { home: None }, None));
        let partial = through
            .iter()
            .find(|place| place.role() == PlaceRole::Partial);
        assert!(matches!(
            partial.and_then(DataPlace::file).map(remove),
            Some(Err(Error::LocalData(LocalDataError::Symlink { .. })))
        ));
        assert!(moved.join(".token.1-1.tmp").exists());
        Ok(())
    }

    #[test]
    fn a_place_outside_its_root_is_refused() -> Result<()> {
        let home = Home::new("outside")?;
        let stray = home.0.join("stray");
        fs::write(&stray, "keep")?;
        let forged = FilePlace {
            path: stray.clone(),
            name: stray.display().to_string(),
            bytes: 4,
            role: PlaceRole::Partial,
            root: home.config(),
        };
        assert!(matches!(
            remove(&forged),
            Err(Error::LocalData(LocalDataError::OutsideLocalData { .. }))
        ));
        assert!(stray.exists());
        Ok(())
    }

    #[rstest]
    #[case(".token.123-4.tmp", Expectation::Holds)]
    #[case(".token.-4.tmp", Expectation::Fails)]
    #[case(".token.12-.tmp", Expectation::Fails)]
    #[case(".token.1a-4.tmp", Expectation::Fails)]
    #[case("token", Expectation::Fails)]
    #[case(".token.1-2.tmp.bak", Expectation::Fails)]
    fn partial_names_are_recognised(#[case] name: &str, #[case] expected: Expectation) {
        assert_eq!(
            crate::storage::file::is_pid_stamped(name, ".token.", ".tmp"),
            expected.holds()
        );
    }
}
