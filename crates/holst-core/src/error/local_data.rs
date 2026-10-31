//! Files on disk and the store that keeps the login.

#[cfg(not(target_os = "macos"))] use std::fmt;
use std::{io, path::PathBuf, time::Duration};

use holst_macros::Hint;

use crate::{domain::session::SessionStoreKind, error::Error};

/// A file or the login store refused, failed or holds something unexpected.
#[derive(Debug, thiserror::Error, Hint)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum LocalDataError {
    #[error("the path {} has no file name", .path.display())]
    NoFileName { path: PathBuf },

    #[error("{} already exists; refusing to overwrite it", .path.display())]
    AlreadyExists { path: PathBuf },

    #[error("could not read {}", .path.display())]
    FileRead {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("could not write {}", .path.display())]
    FileWrite {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("could not remove {}", .path.display())]
    FileRemove {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("could not compress the board state")]
    Pack(#[source] io::Error),

    #[error("the file is not a valid gzip-compressed board state")]
    Unpack(#[source] io::Error),

    #[error("the file unpacks to more than {limit_mib} MiB; refusing to read it")]
    UnpackTooLarge { limit_mib: u64 },

    #[error("{} ends in .gz, but --no-compress writes plain bytes", .path.display())]
    #[hint(DropGzOrPlain)]
    PlainGz { path: PathBuf },

    #[error("{} is a symbolic link; refusing to remove through it", .path.display())]
    Symlink { path: PathBuf },

    #[error("{} is not where holst keeps its data; refusing to remove it", .path.display())]
    OutsideLocalData { path: PathBuf },

    #[error("board {board} is not in the cache, and --offline allows no network")]
    #[hint(LoadOnline)]
    NotCached { board: String },

    #[error(
        "--offline reads the board cache, which is off (HOLST_CACHE=off or --no-cache)"
    )]
    CacheOff,

    #[error(
        "--offline does not know whose board cache to read: the saved login names no account yet"
    )]
    #[hint(LoadOnline)]
    AccountUnknown,

    #[error("the board cache at {} is busy in another holst process", .path.display())]
    CacheBusy { path: PathBuf },

    #[error("file error")]
    Io(#[from] io::Error),

    #[error("the data is not a holst board index")]
    NotBoardIndex,

    #[error(
        "the board index has format version {found}; this holst reads version {supported}"
    )]
    IndexVersion { found: u16, supported: u16 },

    #[error("the board index is damaged")]
    IndexCorrupt {
        #[source]
        error: Option<postcard::Error>,
    },

    #[error("could not encode the board index")]
    IndexEncode(#[source] postcard::Error),

    #[error("no home directory for the fallback login file")]
    NoHome,

    #[error("the keychain is locked or access was denied")]
    #[hint(UnlockKeychain)]
    KeychainLocked,

    #[error("the saved login at {} is busy in another holst process", .path.display())]
    LoginBusy { path: PathBuf },

    #[error(
        "an older login may be left in the {session_store}, which is read before the new one"
    )]
    #[hint(ReplaceStale)]
    StaleLogin {
        session_store: SessionStoreKind,
        #[source]
        error: Box<Error>,
    },

    #[error("the saved login is corrupted")]
    #[hint(LoginAgain)]
    SavedLoginCorrupt,

    #[error("the saved login changed while this command ran, so it was kept")]
    LoginReplaced,

    #[cfg(not(target_os = "macos"))]
    #[error("no Secret Service found (gnome-keyring or KWallet)")]
    #[hint(UseHome)]
    NoSecretService(#[source] keyring::Error),

    #[cfg(not(target_os = "macos"))]
    #[error("the Secret Service failed")]
    #[hint(UseHome)]
    SecretService(#[source] keyring::Error),

    #[cfg(not(target_os = "macos"))]
    #[error("the keychain is unavailable")]
    #[hint(UnlockKeychain)]
    KeychainUnavailable(#[from] keyring::Error),

    #[error(transparent)]
    #[hint(UnlockKeychain)]
    Keychain(#[from] KeychainError),
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn platform_cause(err: &keyring::Error) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        if let keyring::Error::PlatformFailure(cause) = err {
            fmt::Display::fmt(cause, f)
        } else {
            fmt::Display::fmt(err, f)
        }
    })
}

#[cfg(test)]
pub(super) mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::{
        domain::session::SessionStoreKind,
        error::{
            Error,
            hint::decided::assert_all_listed,
            local_data::{
                KeychainError,
                LocalDataError,
                LocalDataErrorDiscriminants,
                io,
            },
        },
    };

    fn io() -> io::Error {
        io::Error::from(io::ErrorKind::NotFound)
    }

    pub(in crate::error) fn samples() -> Vec<LocalDataError> {
        file_samples()
            .into_iter()
            .chain(login_samples())
            .chain(platform())
            .collect()
    }

    fn file_samples() -> Vec<LocalDataError> {
        let path = || PathBuf::from("/x");
        vec![
            LocalDataError::NoFileName { path: path() },
            LocalDataError::AlreadyExists { path: path() },
            LocalDataError::FileRead {
                path: path(),
                error: io(),
            },
            LocalDataError::FileWrite {
                path: path(),
                error: io(),
            },
            LocalDataError::FileRemove {
                path: path(),
                error: io(),
            },
            LocalDataError::Pack(io()),
            LocalDataError::Unpack(io()),
            LocalDataError::UnpackTooLarge { limit_mib: 1 },
            LocalDataError::PlainGz { path: path() },
            LocalDataError::Symlink { path: path() },
            LocalDataError::OutsideLocalData { path: path() },
            LocalDataError::NotCached { board: "b".into() },
            LocalDataError::CacheOff,
            LocalDataError::AccountUnknown,
            LocalDataError::CacheBusy { path: path() },
            LocalDataError::LoginBusy { path: path() },
            LocalDataError::Io(io()),
        ]
    }

    fn login_samples() -> Vec<LocalDataError> {
        vec![
            LocalDataError::NotBoardIndex,
            LocalDataError::IndexVersion {
                found: 9,
                supported: 1,
            },
            LocalDataError::IndexCorrupt { error: None },
            LocalDataError::IndexEncode(postcard::Error::SerializeBufferFull),
            LocalDataError::NoHome,
            LocalDataError::KeychainLocked,
            LocalDataError::StaleLogin {
                session_store: SessionStoreKind::Keychain,
                error: Box::new(Error::from(LocalDataError::KeychainLocked)),
            },
            LocalDataError::SavedLoginCorrupt,
            LocalDataError::LoginReplaced,
            LocalDataError::Keychain(KeychainError::Cancelled),
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_all_listed::<_, LocalDataErrorDiscriminants>(&samples());
    }

    #[test]
    fn a_keychain_timeout_names_its_seconds() {
        assert_eq!(
            KeychainError::TimedOut(Duration::from_secs(30)).to_string(),
            "the keychain did not answer within 30 s; an access dialog may be waiting"
        );
    }

    #[test]
    fn stale_login_names_its_store() {
        let error = LocalDataError::StaleLogin {
            session_store: SessionStoreKind::File,
            error: Box::new(Error::from(LocalDataError::KeychainLocked)),
        };
        assert_eq!(
            error.to_string(),
            "an older login may be left in the file, which is read before the new one"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn secret_service_keeps_its_cause_as_the_source() {
        let error = LocalDataError::SecretService(keyring::Error::NoEntry);
        assert_eq!(
            (
                error.to_string(),
                std::error::Error::source(&error).map(ToString::to_string)
            ),
            (
                "the Secret Service failed".to_owned(),
                Some(keyring::Error::NoEntry.to_string())
            )
        );
    }

    #[cfg(not(target_os = "macos"))]
    fn platform() -> Vec<LocalDataError> {
        vec![
            LocalDataError::NoSecretService(keyring::Error::NoEntry),
            LocalDataError::SecretService(keyring::Error::NoEntry),
            LocalDataError::KeychainUnavailable(keyring::Error::NoEntry),
        ]
    }

    #[cfg(target_os = "macos")]
    fn platform() -> Vec<LocalDataError> {
        Vec::new()
    }
}

/// Why the Keychain could not be used through `security`.
#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    #[error("the keychain is locked")]
    Locked,
    #[error("access to the keychain was denied")]
    Denied,
    #[error("access to the keychain was cancelled")]
    Cancelled,
    #[error(
        "the keychain did not answer within {} s; an access dialog may be waiting",
        .0.as_secs()
    )]
    TimedOut(Duration),
    #[error("the login is too long for the Keychain: {len} bytes, at most {max}")]
    TooLong { len: usize, max: usize },
    #[error("a keychain name or path contains a line break, a NUL or non-UTF-8 bytes")]
    BadName,
    #[error("could not run {}", .program.display())]
    Spawn {
        program: PathBuf,
        #[source]
        error: io::Error,
    },
    #[error("`security` failed with status {status}")]
    Exited { status: i32 },
    #[error("`security` was stopped by a signal")]
    Killed,
}
