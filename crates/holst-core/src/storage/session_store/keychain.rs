//! The system store off macOS: Credential Manager or Secret Service.

use secrecy::{ExposeSecret, SecretString};

use crate::{
    error::{
        Error,
        Result,
        local_data::{LocalDataError, platform_cause},
    },
    storage::session_store::{
        ACCOUNT,
        Keychain,
        LoginRemoval,
        SERVICE,
        SessionStore,
        SessionStoreKind,
    },
};

impl Keychain {
    fn entry() -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|err| {
            // The store is opened once; `Entry::new` then only says there is none.
            match keyring::Entry::store_status() {
                Err(init) if matches!(err, keyring::Error::NoDefaultStore) => {
                    init_error(init)
                }
                _ => keychain_error(err),
            }
        })
    }
}

const HAS_SECRET_SERVICE: bool = cfg!(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd"
));

pub(super) fn keychain_error(err: keyring::Error) -> Error {
    let missing = is_missing(&err);
    classify(err, missing)
}

/// The cached opening failure cannot be moved out, so its message is carried
/// over.
pub(super) fn init_error(init: &keyring::Error) -> Error {
    let cause = platform_cause(init).to_string();
    classify(
        keyring::Error::PlatformFailure(cause.into()),
        is_missing(init),
    )
}

fn is_missing(err: &keyring::Error) -> bool {
    matches!(err, keyring::Error::PlatformFailure(cause) if is_unavailable(&**cause))
}

fn classify(err: keyring::Error, missing: bool) -> Error {
    // `keyring::Error` is non-exhaustive: guards instead of a wildcard arm.
    Error::LocalData(match () {
        () if matches!(err, keyring::Error::NoStorageAccess(_)) => {
            LocalDataError::KeychainLocked
        }
        () if matches!(err, keyring::Error::BadEncoding(_)) => {
            LocalDataError::SavedLoginCorrupt
        }
        () if HAS_SECRET_SERVICE && missing => LocalDataError::NoSecretService(err),
        () if HAS_SECRET_SERVICE
            && matches!(err, keyring::Error::PlatformFailure(_)) =>
        {
            LocalDataError::SecretService(err)
        }
        () => LocalDataError::KeychainUnavailable(err),
    })
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
fn is_unavailable(cause: &(dyn std::error::Error + 'static)) -> bool {
    match cause.downcast_ref::<secret_service::Error>() {
        Some(secret_service::Error::Unavailable) => true,
        Some(secret_service::Error::Zbus(zbus::Error::MethodError(name, ..))) => {
            name.as_str() == SERVICE_UNKNOWN
        }
        Some(secret_service::Error::Zbus(zbus::Error::FDO(fdo))) => {
            matches!(**fdo, zbus::fdo::Error::ServiceUnknown(_))
        }
        Some(secret_service::Error::ZbusFdo(fdo)) => {
            matches!(fdo, zbus::fdo::Error::ServiceUnknown(_))
        }
        Some(_) | None => false,
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";

#[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd")))]
const fn is_unavailable(_: &(dyn std::error::Error + 'static)) -> bool {
    false
}

impl SessionStore for Keychain {
    fn kind(&self) -> SessionStoreKind {
        SessionStoreKind::Keychain
    }

    fn read(&self) -> Result<Option<SecretString>> {
        match Self::entry()?.get_password() {
            Ok(stored) => Ok(Some(SecretString::from(stored))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(keychain_error(err)),
        }
    }

    fn write(&self, stored: &SecretString) -> Result<()> {
        Self::entry()?
            .set_password(stored.expose_secret())
            .map_err(keychain_error)
    }

    fn remove(&self) -> Result<LoginRemoval> {
        match Self::entry()?.delete_credential() {
            Ok(()) => Ok(LoginRemoval::Removed),
            Err(keyring::Error::NoEntry) => Ok(LoginRemoval::Absent),
            Err(err) => {
                let err = keychain_error(err);
                if matches!(err, Error::LocalData(LocalDataError::NoSecretService(_))) {
                    Ok(LoginRemoval::Absent)
                } else {
                    Err(err)
                }
            }
        }
    }
}
