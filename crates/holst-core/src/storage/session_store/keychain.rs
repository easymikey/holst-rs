//! The system store off macOS: Credential Manager or Secret Service.

use secrecy::{ExposeSecret, SecretString};

use crate::{
    error::{Error, Result, local_data::LocalDataError},
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
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(keychain_error)
    }
}

fn check_persistent() -> Result<()> {
    use keyring::credential::CredentialPersistence;
    match keyring::default::default_credential_builder().persistence() {
        CredentialPersistence::EntryOnly | CredentialPersistence::ProcessOnly => {
            Err(Error::LocalData(LocalDataError::KeychainVolatile))
        }
        _ => Ok(()),
    }
}

const HAS_SECRET_SERVICE: bool = cfg!(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd"
));

fn keychain_error(err: keyring::Error) -> Error {
    match err {
        keyring::Error::NoStorageAccess(_) => {
            Error::LocalData(LocalDataError::KeychainLocked)
        }
        keyring::Error::BadEncoding(_) => {
            Error::LocalData(LocalDataError::SavedLoginCorrupt)
        }
        keyring::Error::PlatformFailure(cause)
            if HAS_SECRET_SERVICE && is_unavailable(&*cause) =>
        {
            Error::LocalData(LocalDataError::NoSecretService(
                keyring::Error::PlatformFailure(cause),
            ))
        }
        keyring::Error::PlatformFailure(cause) if HAS_SECRET_SERVICE => {
            Error::LocalData(LocalDataError::SecretService(
                keyring::Error::PlatformFailure(cause),
            ))
        }
        other => Error::LocalData(LocalDataError::KeychainUnavailable(other)),
    }
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
        check_persistent()?;
        Self::entry()?
            .set_password(stored.expose_secret())
            .map_err(keychain_error)
    }

    fn remove(&self) -> Result<LoginRemoval> {
        match Self::entry()?.delete_credential() {
            Ok(()) => Ok(LoginRemoval::Removed),
            Err(keyring::Error::NoEntry) => Ok(LoginRemoval::Absent),
            Err(err) => match keychain_error(err) {
                Error::LocalData(LocalDataError::NoSecretService(_)) => {
                    Ok(LoginRemoval::Absent)
                }
                other => Err(other),
            },
        }
    }
}
