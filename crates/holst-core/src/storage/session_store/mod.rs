//! Where a login is kept: the keychain with a 0600 file, or `HOLST_HOME`.

#[cfg(not(target_os = "macos"))] mod keychain;
#[cfg(target_os = "macos")] pub mod macos;
#[cfg(target_os = "macos")] pub mod security;
#[cfg(test)] mod tests;

use std::{
    fs,
    io,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use secrecy::{ExposeSecret, SecretString};

use crate::{
    domain::session::Session,
    effect::settings::SessionLocation,
    error::{Error, Result, local_data::LocalDataError},
    storage::{
        file::write_error,
        local_data::HOME_SESSION_FILE,
        lock::{LOCK_FILE, LOCK_WAIT_MILLIS, lock_waiting},
    },
};

const SERVICE: &str = "holst-rs";
const ACCOUNT: &str = "session-token";

/// Where a login was found or saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStoreKind {
    Keychain,
    File,
}

impl SessionStoreKind {
    /// The name a person knows the store by.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionStoreKind::Keychain if cfg!(target_os = "macos") => "Keychain",
            SessionStoreKind::Keychain if cfg!(windows) => "Credential Manager",
            SessionStoreKind::Keychain => "Secret Service",
            SessionStoreKind::File => "file",
        }
    }
}

/// `<home>/.config/holst/token` on every OS (never change it); none without
/// home.
#[must_use]
pub fn default_file(home: Option<&Path>) -> Option<PathBuf> {
    home.map(|home| home.join(".config/holst/token"))
}

impl SessionLocation {
    /// The session file this location writes, if there is one.
    #[must_use]
    pub fn file(&self) -> Option<PathBuf> {
        match self {
            SessionLocation::System { home } => default_file(home.as_deref()),
            SessionLocation::Dir(dir) => Some(dir.join(HOME_SESSION_FILE)),
        }
    }

    /// Takes the `LOCK` file beside the session file, so a save, a
    /// read-check-write or a logout is one step across processes; `None`
    /// without a file. A busy lock is waited for as long as the cache's.
    ///
    /// # Errors
    /// [`LocalDataError::FileWrite`] on the `LOCK` path when it cannot be
    /// created or locked; [`LocalDataError::LoginBusy`] when it stays busy.
    fn lock(&self) -> Result<Option<fs::File>> {
        self.file()
            .map(|file| file.with_file_name(LOCK_FILE))
            .map(|path| {
                let failed = |source| write_error(&path, source);
                path.parent()
                    .map(fs::create_dir_all)
                    .transpose()
                    .map_err(failed)?;
                let held = lock_waiting(&path, LOCK_WAIT_MILLIS).map_err(failed)?;
                held.ok_or_else(|| Error::LocalData(LocalDataError::LoginBusy { path }))
            })
            .transpose()
    }

    fn stores(&self) -> Vec<Box<dyn SessionStore>> {
        match self {
            SessionLocation::System { home } => {
                vec![
                    Box::new(Keychain),
                    Box::new(SessionFile::in_home(home.as_deref())),
                ]
            }
            SessionLocation::Dir(dir) => {
                vec![Box::new(SessionFile::at(dir.join(HOME_SESSION_FILE)))]
            }
        }
    }

    /// The saved login and the store it came from.
    ///
    /// # Errors
    /// The first store failure when no store has a login.
    pub fn load(&self) -> Result<Option<(Session, SessionStoreKind)>> {
        first_stored(&borrowed(&self.stores()))
    }

    /// Save a new login in the first store that takes it.
    ///
    /// # Errors
    /// The last store's error when none takes it; the lock's error when it
    /// cannot be taken.
    pub fn save(&self, session: &Session) -> Result<LoginSaveReport> {
        let _lock = self.lock()?;
        save_in(&borrowed(&self.stores()), session)
    }

    /// Save a rotated login back where it was loaded from, or a later store.
    ///
    /// # Errors
    /// As [`resave_in`].
    pub fn resave(
        &self,
        session: &Session,
        from: SessionStoreKind,
    ) -> Result<LoginSaveReport> {
        let _lock = self.lock()?;
        resave_in(starting_at(&borrowed(&self.stores()), from), session)
    }

    /// Remove the login from every store; what each one found. A lock that
    /// cannot be created does not stop the removal; one that stays busy does.
    #[must_use]
    pub fn clear(&self) -> Vec<(SessionStoreKind, Result<LoginRemoval>)> {
        let stores = self.stores();
        let lock = self.lock();
        match &lock {
            Err(Error::LocalData(LocalDataError::LoginBusy { path })) => stores
                .iter()
                .map(|store| {
                    (
                        store.kind(),
                        Err(Error::LocalData(LocalDataError::LoginBusy {
                            path: path.clone(),
                        })),
                    )
                })
                .collect(),
            Err(err) => {
                tracing::debug!(%err, "login store: logging out unlocked");
                clear_from(&borrowed(&stores))
            }
            Ok(_) => clear_from(&borrowed(&stores)),
        }
    }
}

/// The store port: where a session lives between runs.
pub trait SessionStore {
    /// Which store it is.
    fn kind(&self) -> SessionStoreKind;

    /// The stored value; `Ok(None)` when nothing is stored.
    ///
    /// # Errors
    /// [`crate::error::local_data::LocalDataError::KeychainLocked`],
    /// [`crate::error::local_data::LocalDataError::SavedLoginCorrupt`] or
    /// another store error.
    fn read(&self) -> Result<Option<SecretString>>;

    /// Store a value, replacing the old one.
    ///
    /// # Errors
    /// The store's error: unavailable, not written, not read back.
    fn write(&self, stored: &SecretString) -> Result<()>;

    /// Remove the value: gone, or there was none.
    ///
    /// # Errors
    /// The store's error: the value may still be there.
    fn remove(&self) -> Result<LoginRemoval>;
}

/// What removing a login found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRemoval {
    Removed,
    Absent,
}

/// Where a login was saved, and an older copy that is still read first.
#[derive(Debug)]
pub struct LoginSaveReport {
    pub store: SessionStoreKind,
    pub stale: Option<Error>,
}

/// The system store: Keychain, Credential Manager or Secret Service.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keychain;

/// The fallback file with the session JSON, mode 0600.
#[derive(Debug, Clone, Default)]
pub struct SessionFile {
    path: Option<PathBuf>,
}

impl SessionFile {
    /// [`default_file`] of `home`; no file without a home directory.
    #[must_use]
    pub fn in_home(home: Option<&Path>) -> Self {
        Self {
            path: default_file(home),
        }
    }

    /// The file at a given path.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }
}

impl SessionStore for SessionFile {
    fn kind(&self) -> SessionStoreKind {
        SessionStoreKind::File
    }

    fn read(&self) -> Result<Option<SecretString>> {
        self.path
            .as_deref()
            .map_or(Ok(None), |path| match fs::read_to_string(path) {
                Ok(stored) => Ok(Some(SecretString::from(stored))),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(Error::LocalData(LocalDataError::FileRead {
                    path: path.to_path_buf(),
                    error: source,
                })),
            })
    }

    fn write(&self, stored: &SecretString) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or(Error::LocalData(LocalDataError::NoHome))?;
        path.parent()
            .map(fs::create_dir_all)
            .transpose()
            .map_err(|source| {
                Error::LocalData(LocalDataError::FileWrite {
                    path: path.clone(),
                    error: source,
                })
            })?;
        crate::storage::file::replace_private(
            path,
            &[stored.expose_secret().as_bytes(), b"\n"],
        )
    }

    fn remove(&self) -> Result<LoginRemoval> {
        self.path
            .as_deref()
            .map_or(Ok(LoginRemoval::Absent), |path| {
                match fs::remove_file(path) {
                    Ok(()) => Ok(LoginRemoval::Removed),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        Ok(LoginRemoval::Absent)
                    }
                    Err(source) => Err(Error::LocalData(LocalDataError::FileRemove {
                        path: path.to_path_buf(),
                        error: source,
                    })),
                }
            })
    }
}

fn borrowed(stores: &[Box<dyn SessionStore>]) -> Vec<&dyn SessionStore> {
    stores.iter().map(AsRef::as_ref).collect()
}

fn starting_at<'a, 'b>(
    stores: &'a [&'b dyn SessionStore],
    from: SessionStoreKind,
) -> &'a [&'b dyn SessionStore] {
    stores
        .iter()
        .position(|store| store.kind() == from)
        .and_then(|at| stores.get(at..))
        .unwrap_or(stores)
}

fn first_stored(
    stores: &[&dyn SessionStore],
) -> Result<Option<(Session, SessionStoreKind)>> {
    let found =
        stores.iter().try_fold(None, |first_failure, store| {
            match read_session(*store) {
                Ok(Some(session)) => ControlFlow::Break((session, store.kind())),
                Ok(None) => ControlFlow::Continue(first_failure),
                Err(err) => ControlFlow::Continue(first_failure.or(Some(err))),
            }
        });
    match found {
        ControlFlow::Break(found) => Ok(Some(found)),
        ControlFlow::Continue(Some(failure)) => Err(failure),
        ControlFlow::Continue(None) => Ok(None),
    }
}

fn read_session(store: &dyn SessionStore) -> Result<Option<Session>> {
    store
        .read()?
        .map(|raw| {
            raw.expose_secret()
                .parse::<Session>()
                .map_err(|_| Error::LocalData(LocalDataError::SavedLoginCorrupt))
        })
        .transpose()
}

/// A rotation of the login the first store still holds, saved as [`save_in`].
///
/// # Errors
/// [`crate::error::local_data::LocalDataError::LoginReplaced`] when that store
/// now holds another login or none; a logout or a new login meanwhile wins.
/// Else as [`save_in`].
pub fn resave_in(
    stores: &[&dyn SessionStore],
    session: &Session,
) -> Result<LoginSaveReport> {
    let held = stores.first().map(|store| read_session(*store));
    let token = |kept: &Session| {
        kept.token().expose_secret() == session.token().expose_secret()
    };
    match held {
        Some(Ok(None)) => Err(Error::LocalData(LocalDataError::LoginReplaced)),
        Some(Ok(Some(kept))) if !token(&kept) => {
            Err(Error::LocalData(LocalDataError::LoginReplaced))
        }
        // An unreadable store cannot tell: the write decides, as before.
        Some(Ok(Some(_)) | Err(_)) | None => save_in(stores, session),
    }
}

/// Write to the first store that takes it and remove the copies elsewhere.
///
/// # Errors
/// The last store's error when none took it;
/// [`crate::error::local_data::LocalDataError::NoHome`] without stores; a
/// Secret Service error (Linux) at once: a new login never goes quietly to the
/// file.
pub fn save_in(
    stores: &[&dyn SessionStore],
    session: &Session,
) -> Result<LoginSaveReport> {
    let value = session.to_saved_login();
    let written = stores.iter().enumerate().try_fold(
        Error::LocalData(LocalDataError::NoHome),
        |_, (at, store)| match store.write(&value) {
            Ok(()) => ControlFlow::Break(Ok((at, store.kind()))),
            #[cfg(not(target_os = "macos"))]
            Err(
                err @ Error::LocalData(
                    LocalDataError::NoSecretService(_)
                    | LocalDataError::SecretService(_),
                ),
            ) => ControlFlow::Break(Err(err)),
            Err(err) => {
                tracing::debug!("{} is unavailable ({err})", store.kind().as_str());
                ControlFlow::Continue(err)
            }
        },
    );
    match written {
        ControlFlow::Break(Err(err)) | ControlFlow::Continue(err) => Err(err),
        ControlFlow::Break(Ok((at, kind))) => Ok(LoginSaveReport {
            store: kind,
            stale: remove_others(stores, at),
        }),
    }
}

fn remove_others(stores: &[&dyn SessionStore], at: usize) -> Option<Error> {
    stores
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != at)
        .fold(None, |stale, (other, store)| match store.remove() {
            Ok(_) => stale,
            Err(err) if other < at => stale.or(Some(Error::LocalData(LocalDataError::StaleLogin {
                session_store: store.kind().as_str(),
                error: Box::new(err),
            }))),
            Err(err) => {
                tracing::warn!(%err, "an old copy of the login is left in the {}", store.kind().as_str());
                stale
            }
        })
}

/// Remove the login from every store; what each one found.
#[must_use]
pub fn clear_from(
    stores: &[&dyn SessionStore],
) -> Vec<(SessionStoreKind, Result<LoginRemoval>)> {
    stores
        .iter()
        .map(|store| (store.kind(), store.remove()))
        .collect()
}
