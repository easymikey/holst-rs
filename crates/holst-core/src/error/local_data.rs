//! Files on disk and the store that keeps the login.

#[cfg(not(target_os = "macos"))] use std::fmt;
use std::{io, path::PathBuf, time::Duration};

use crate::error::{Error, hint::Hint};

/// A file or the login store refused, failed or holds something unexpected.
#[derive(Debug, thiserror::Error)]
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

    #[error(
        "{} ends in .gz, but --uncompressed writes plain bytes: drop .gz or the flag",
        .path.display()
    )]
    PlainGz { path: PathBuf },

    #[error("{} is a symbolic link; refusing to remove through it", .path.display())]
    Symlink { path: PathBuf },

    #[error("{} is not where holst keeps its data; refusing to remove it", .path.display())]
    OutsideLocalData { path: PathBuf },

    #[error("board {board} is not in the cache, and --offline allows no network")]
    NotCached { board: String },

    #[error(
        "--offline reads the board cache, which is off (HOLST_CACHE=off or --no-cache)"
    )]
    CacheOff,

    #[error(
        "--offline does not know whose board cache to read: the saved login names no account yet"
    )]
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
    KeychainLocked,

    #[error("the saved login at {} is busy in another holst process", .path.display())]
    LoginBusy { path: PathBuf },

    #[error(
        "an older login may be left in the {session_store}, which is read before the new one"
    )]
    StaleLogin {
        session_store: &'static str,
        #[source]
        error: Box<Error>,
    },

    #[error("the saved login is corrupted")]
    SavedLoginCorrupt,

    #[error("the saved login changed while this command ran, so it was kept")]
    LoginReplaced,

    #[error("the keychain keeps nothing between runs")]
    KeychainVolatile,

    #[cfg(not(target_os = "macos"))]
    #[error("no Secret Service found (gnome-keyring or KWallet)")]
    NoSecretService(#[source] keyring::Error),

    #[cfg(not(target_os = "macos"))]
    #[error("Secret Service failed: {}", platform_cause(.0))]
    SecretService(keyring::Error),

    #[cfg(not(target_os = "macos"))]
    #[error("the keychain is unavailable")]
    KeychainUnavailable(#[from] keyring::Error),

    #[error(transparent)]
    Keychain(#[from] KeychainError),
}

impl LocalDataError {
    #[must_use]
    pub(super) fn mcp_words(&self) -> Option<String> {
        match self {
            LocalDataError::PlainGz { path } => Some(format!(
                "{} ends in .gz, but uncompressed: true writes plain bytes: drop .gz or \
                 uncompressed",
                path.display()
            )),
            #[cfg(not(target_os = "macos"))]
            LocalDataError::NoSecretService(_)
            | LocalDataError::SecretService(_)
            | LocalDataError::KeychainUnavailable(_) => None,
            // `--offline` is the CLI's only: MCP never reads offline.
            LocalDataError::NotCached { .. }
            | LocalDataError::CacheOff
            | LocalDataError::AccountUnknown
            | LocalDataError::SavedLoginCorrupt
            | LocalDataError::KeychainLocked
            | LocalDataError::Keychain(_)
            | LocalDataError::StaleLogin { .. }
            | LocalDataError::NoFileName { .. }
            | LocalDataError::AlreadyExists { .. }
            | LocalDataError::FileRead { .. }
            | LocalDataError::FileWrite { .. }
            | LocalDataError::FileRemove { .. }
            | LocalDataError::Pack(_)
            | LocalDataError::Unpack(_)
            | LocalDataError::UnpackTooLarge { .. }
            | LocalDataError::Symlink { .. }
            | LocalDataError::OutsideLocalData { .. }
            | LocalDataError::CacheBusy { .. }
            | LocalDataError::LoginBusy { .. }
            | LocalDataError::Io(_)
            | LocalDataError::NotBoardIndex
            | LocalDataError::IndexVersion { .. }
            | LocalDataError::IndexCorrupt { .. }
            | LocalDataError::IndexEncode(_)
            | LocalDataError::NoHome
            | LocalDataError::LoginReplaced
            | LocalDataError::KeychainVolatile => None,
        }
    }

    /// What to do next, decided for every variant.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            LocalDataError::SavedLoginCorrupt => Some(Hint::LoginAgain),
            #[cfg(not(target_os = "macos"))]
            LocalDataError::KeychainLocked
            | LocalDataError::KeychainUnavailable(_)
            | LocalDataError::Keychain(_) => Some(Hint::UnlockKeychain),
            #[cfg(target_os = "macos")]
            LocalDataError::KeychainLocked | LocalDataError::Keychain(_) => {
                Some(Hint::UnlockKeychain)
            }
            #[cfg(not(target_os = "macos"))]
            LocalDataError::NoSecretService(_) | LocalDataError::SecretService(_) => {
                Some(Hint::UseHome)
            }
            LocalDataError::StaleLogin { .. } => Some(Hint::ReplaceStale),
            LocalDataError::NotCached { .. } | LocalDataError::AccountUnknown => {
                Some(Hint::LoadOnline)
            }
            LocalDataError::NoFileName { .. }
            | LocalDataError::AlreadyExists { .. }
            | LocalDataError::FileRead { .. }
            | LocalDataError::FileWrite { .. }
            | LocalDataError::FileRemove { .. }
            | LocalDataError::Pack(_)
            | LocalDataError::Unpack(_)
            | LocalDataError::UnpackTooLarge { .. }
            | LocalDataError::PlainGz { .. }
            | LocalDataError::Symlink { .. }
            | LocalDataError::OutsideLocalData { .. }
            | LocalDataError::CacheOff
            | LocalDataError::CacheBusy { .. }
            | LocalDataError::LoginBusy { .. }
            | LocalDataError::Io(_)
            | LocalDataError::NotBoardIndex
            | LocalDataError::IndexVersion { .. }
            | LocalDataError::IndexCorrupt { .. }
            | LocalDataError::IndexEncode(_)
            | LocalDataError::NoHome
            | LocalDataError::LoginReplaced
            | LocalDataError::KeychainVolatile => None,
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_cause(err: &keyring::Error) -> impl fmt::Display + '_ {
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
    use std::path::PathBuf;

    use crate::error::{
        Error,
        hint::{Hint, decided::assert_every_variant},
        local_data::{KeychainError, LocalDataError, LocalDataErrorDiscriminants, io},
    };

    fn io() -> io::Error {
        io::Error::from(io::ErrorKind::NotFound)
    }

    pub(in crate::error) fn samples() -> Vec<(LocalDataError, Option<Hint>)> {
        file_samples()
            .into_iter()
            .chain(login_samples())
            .chain(platform())
            .collect()
    }

    fn file_samples() -> Vec<(LocalDataError, Option<Hint>)> {
        let path = || PathBuf::from("/x");
        vec![
            (LocalDataError::NoFileName { path: path() }, None),
            (LocalDataError::AlreadyExists { path: path() }, None),
            (
                LocalDataError::FileRead {
                    path: path(),
                    error: io(),
                },
                None,
            ),
            (
                LocalDataError::FileWrite {
                    path: path(),
                    error: io(),
                },
                None,
            ),
            (
                LocalDataError::FileRemove {
                    path: path(),
                    error: io(),
                },
                None,
            ),
            (LocalDataError::Pack(io()), None),
            (LocalDataError::Unpack(io()), None),
            (LocalDataError::UnpackTooLarge { limit_mib: 1 }, None),
            (LocalDataError::PlainGz { path: path() }, None),
            (LocalDataError::Symlink { path: path() }, None),
            (LocalDataError::OutsideLocalData { path: path() }, None),
            (
                LocalDataError::NotCached { board: "b".into() },
                Some(Hint::LoadOnline),
            ),
            (LocalDataError::CacheOff, None),
            (LocalDataError::AccountUnknown, Some(Hint::LoadOnline)),
            (LocalDataError::CacheBusy { path: path() }, None),
            (LocalDataError::LoginBusy { path: path() }, None),
            (LocalDataError::Io(io()), None),
        ]
    }

    fn login_samples() -> Vec<(LocalDataError, Option<Hint>)> {
        vec![
            (LocalDataError::NotBoardIndex, None),
            (
                LocalDataError::IndexVersion {
                    found: 9,
                    supported: 1,
                },
                None,
            ),
            (LocalDataError::IndexCorrupt { error: None }, None),
            (
                LocalDataError::IndexEncode(postcard::Error::SerializeBufferFull),
                None,
            ),
            (LocalDataError::NoHome, None),
            (LocalDataError::KeychainLocked, Some(Hint::UnlockKeychain)),
            (
                LocalDataError::StaleLogin {
                    session_store: "keychain",
                    error: Box::new(Error::from(LocalDataError::KeychainLocked)),
                },
                Some(Hint::ReplaceStale),
            ),
            (LocalDataError::SavedLoginCorrupt, Some(Hint::LoginAgain)),
            (LocalDataError::LoginReplaced, None),
            (LocalDataError::KeychainVolatile, None),
            (
                LocalDataError::Keychain(KeychainError::Cancelled),
                Some(Hint::UnlockKeychain),
            ),
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, LocalDataErrorDiscriminants>(
            samples(),
            LocalDataError::hint,
        );
    }

    #[cfg(not(target_os = "macos"))]
    fn platform() -> Vec<(LocalDataError, Option<Hint>)> {
        vec![
            (
                LocalDataError::NoSecretService(keyring::Error::NoEntry),
                Some(Hint::UseHome),
            ),
            (
                LocalDataError::SecretService(keyring::Error::NoEntry),
                Some(Hint::UseHome),
            ),
            (
                LocalDataError::KeychainUnavailable(keyring::Error::NoEntry),
                Some(Hint::UnlockKeychain),
            ),
        ]
    }

    #[cfg(target_os = "macos")]
    fn platform() -> Vec<(LocalDataError, Option<Hint>)> {
        Vec::new()
    }
}

/// Why the Keychain could not be used through `security`.
#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    #[error("the keychain is locked")]
    Locked,
    #[error("Keychain access was denied")]
    Denied,
    #[error("Keychain access was cancelled")]
    Cancelled,
    #[error(
        "Keychain did not answer within {} s; an access dialog may be waiting",
        .0.as_secs()
    )]
    TimedOut(Duration),
    #[error("the login is too long for the Keychain: {len} bytes, at most {max}")]
    TooLong { len: usize, max: usize },
    #[error("a Keychain name or path contains a line break, a NUL or non-UTF-8 bytes")]
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
