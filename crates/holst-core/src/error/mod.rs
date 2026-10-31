//! Core errors by domain, their causes (`source()`) and advice ([`Hint`]).

pub mod board_change;
pub mod board_socket;
pub mod check_all;
pub mod confirmation;
pub mod hint;
pub mod http;
pub mod input;
pub mod local_data;
pub mod login;
pub mod lookup;
pub mod picture;
pub mod stored_file;

use std::{error::Error as StdError, fmt::Display};

use holst_macros::Hint;

use self::{
    board_change::BoardChangeError,
    board_socket::{BoardSocketError, SocketErrorCode},
    http::HttpError,
    input::InputError,
    local_data::LocalDataError,
    login::LoginError,
    lookup::LookupError,
    picture::PictureError,
    stored_file::StoredFileError,
};

/// An error from the Holst API or from the local side of a call, by domain.
///
/// `hint()` is what the person can do about it, when the core knows; each
/// domain decides.
///
/// ```
/// use holst_core::{
///     Error,
///     error::{hint::Hint, input::InputError, login::LoginError},
/// };
///
/// assert_eq!(
///     Error::from(LoginError::SessionRevoked).hint(),
///     Some(Hint::LoginAgain)
/// );
/// assert_eq!(Error::from(InputError::FullUrl).hint(), None);
/// ```
#[derive(Debug, thiserror::Error, Hint)]
pub enum Error {
    #[error(transparent)]
    #[hint(forward)]
    Login(#[from] LoginError),

    #[error(transparent)]
    #[hint(forward)]
    Http(#[from] HttpError),

    #[error(transparent)]
    #[hint(forward)]
    Input(#[from] InputError),

    #[error(transparent)]
    #[hint(forward)]
    LocalData(#[from] LocalDataError),

    #[error(transparent)]
    #[hint(forward)]
    Lookup(#[from] LookupError),

    #[error(transparent)]
    #[hint(forward)]
    BoardSocket(#[from] BoardSocketError),

    #[error(transparent)]
    #[hint(forward)]
    BoardChange(#[from] BoardChangeError),

    #[error(transparent)]
    #[hint(forward)]
    Picture(#[from] PictureError),

    #[error(transparent)]
    #[hint(forward)]
    StoredFile(#[from] StoredFileError),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::LocalData(error.into())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Http(error.into())
    }
}

impl From<yrs::encoding::read::Error> for Error {
    fn from(error: yrs::encoding::read::Error) -> Self {
        Self::BoardSocket(error.into())
    }
}

impl From<yrs::error::UpdateError> for Error {
    fn from(error: yrs::error::UpdateError) -> Self {
        Self::BoardSocket(error.into())
    }
}

impl From<local_data::KeychainError> for Error {
    fn from(error: local_data::KeychainError) -> Self {
        Self::LocalData(error.into())
    }
}

impl Error {
    /// 401 or 403: the session is not valid or there is no access.
    ///
    /// ```
    /// use holst_core::{Error, error::login::LoginError};
    ///
    /// assert!(Error::from(LoginError::SessionRevoked).is_unauthorized());
    /// assert!(!Error::from(LoginError::NoSavedLogin).is_unauthorized());
    /// ```
    #[must_use]
    pub fn is_unauthorized(&self) -> bool {
        matches!(
            self,
            Error::Login(LoginError::SessionRevoked | LoginError::BareToken { .. })
                | Error::Http(HttpError::Status {
                    status: 401 | 403,
                    ..
                })
        )
    }

    /// The board is closed to the user now: 403, 404, forbidden, not found.
    ///
    /// ```
    /// use holst_core::{
    ///     Error,
    ///     error::board_socket::{BoardSocketError, SocketErrorCode},
    /// };
    ///
    /// let refused = |code| Error::from(BoardSocketError::NotSubscribed { code });
    /// assert!(refused(SocketErrorCode::Forbidden).is_board_closed());
    /// assert!(refused(SocketErrorCode::NotFound).is_board_closed());
    /// assert!(!refused(SocketErrorCode::InternalError).is_board_closed());
    /// assert!(!Error::from(BoardSocketError::Closed).is_board_closed());
    /// ```
    #[must_use]
    pub fn is_board_closed(&self) -> bool {
        match self {
            Error::BoardSocket(BoardSocketError::NotSubscribed { code }) => {
                matches!(code, SocketErrorCode::Forbidden | SocketErrorCode::NotFound)
            }
            Error::Http(HttpError::Status {
                status: 403 | 404, ..
            }) => true,
            Error::Login(_)
            | Error::Http(_)
            | Error::Input(_)
            | Error::LocalData(_)
            | Error::Lookup(_)
            | Error::BoardSocket(_)
            | Error::BoardChange(_)
            | Error::Picture(_)
            | Error::StoredFile(_) => false,
        }
    }

    /// No login or a revoked one: going on makes no sense.
    ///
    /// ```
    /// use holst_core::{
    ///     Error,
    ///     error::{input::InputError, login::LoginError},
    /// };
    ///
    /// assert!(Error::from(LoginError::NoSavedLogin).is_fatal());
    /// assert!(!Error::from(InputError::FullUrl).is_fatal());
    /// ```
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Error::Login(
                LoginError::NoSavedLogin
                    | LoginError::SessionRevoked
                    | LoginError::BareToken { .. }
            ) | Error::Http(HttpError::Status { status: 401, .. })
        )
    }

    /// The HTTP status of an [`HttpError::Status`].
    ///
    /// ```
    /// use holst_core::{Error, error::input::InputError};
    ///
    /// assert_eq!(Error::from(InputError::FullUrl).status(), None);
    /// ```
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Http(HttpError::Status { status, .. }) => Some(*status),
            Error::Login(_)
            | Error::Http(_)
            | Error::Input(_)
            | Error::LocalData(_)
            | Error::Lookup(_)
            | Error::BoardSocket(_)
            | Error::BoardChange(_)
            | Error::Picture(_)
            | Error::StoredFile(_) => None,
        }
    }
}

/// Causes along `source()`, nearest first, skipping ones already printed.
///
/// ```
/// use std::io;
///
/// use holst_core::error::chain;
///
/// let err = io::Error::from(io::ErrorKind::NotFound);
/// assert_eq!(chain(&err).count(), 0);
/// ```
pub fn chain<'a>(
    err: &'a (dyn StdError + 'static),
) -> impl Iterator<Item = String> + 'a {
    chain_unless(err, |_| false)
}

/// The error and its causes in one line, `a: b: c`, skipping what [`chain`]
/// skips.
///
/// ```
/// use std::io;
///
/// use holst_core::error::chained;
///
/// let err = io::Error::from(io::ErrorKind::NotFound);
/// assert_eq!(chained(&err).to_string(), err.to_string());
/// ```
pub fn chained<'a>(error: &'a (dyn StdError + 'static)) -> impl Display + 'a {
    std::fmt::from_fn(move |f| {
        write!(f, "{error}")?;
        chain(error).try_for_each(|cause| write!(f, ": {cause}"))
    })
}

/// `serialize_with` for a report field that keeps its [`Error`] and writes
/// the [`chained`] text.
///
/// # Errors
/// Those of the serializer.
pub fn serialize_chained<S: serde::Serializer>(
    error: &Error,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(&chained(error))
}

/// [`chain`] without the causes `hidden` says are not for the reader.
pub fn chain_unless<'a>(
    err: &'a (dyn StdError + 'static),
    hidden: impl Fn(&(dyn StdError + 'static)) -> bool + 'a,
) -> impl Iterator<Item = String> + 'a {
    std::iter::successors(err.source(), |&e| e.source())
        .scan(err.to_string(), move |previous, cause| {
            let text = cause.to_string();
            let repeat = text.is_empty() || previous.contains(&text) || hidden(cause);
            previous.clone_from(&text);
            Some((!repeat).then_some(text))
        })
        .flatten()
}

/// Result with the core [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use std::io;

    use crate::error::{
        Error,
        board_change,
        chain,
        chained,
        http,
        input,
        local_data,
        local_data::LocalDataError,
        login,
        login::LoginError,
        stored_file,
    };

    fn hinted<E, H>(samples: Vec<(E, H)>) -> impl Iterator<Item = Error>
    where
        Error: From<E>,
    {
        samples.into_iter().map(|(err, _)| Error::from(err))
    }

    fn all_samples() -> Vec<Error> {
        login::tests::samples()
            .into_iter()
            .map(Error::from)
            .chain(hinted(http::tests::samples()))
            .chain(input::tests::samples().into_iter().map(Error::from))
            .chain(local_data::tests::samples().into_iter().map(Error::from))
            .chain(hinted(crate::error::board_socket::tests::samples()))
            .chain(board_change::tests::samples().into_iter().map(Error::from))
            .chain(
                crate::error::picture::tests::samples()
                    .into_iter()
                    .map(Error::from),
            )
            .chain(stored_file::tests::samples().into_iter().map(Error::from))
            .collect()
    }

    #[test]
    fn messages_are_not_empty_and_end_without_a_period() {
        let bad: Vec<String> = all_samples()
            .iter()
            .map(ToString::to_string)
            .filter(|text| text.is_empty() || text.ends_with('.'))
            .collect();
        assert_eq!(bad, Vec::<String>::new());
    }

    #[test]
    fn mcp_messages_name_no_flags() {
        let all = all_samples();
        let flagged: Vec<String> = all
            .iter()
            .filter(|err| err.to_string().contains("--"))
            .map(|err| format!("{err:?}"))
            .collect();
        // `--offline` exists on the command line only; `PlainGz` names its flag
        // on both.
        let cli_only = [
            "NotCached",
            "CacheOff",
            "AccountUnknown",
            "OfflineByName",
            "PlainGz",
        ];
        let reachable: Vec<&String> = flagged
            .iter()
            .filter(|debug| !cli_only.iter().any(|name| debug.contains(name)))
            .collect();
        assert!(all.len() > 80, "{}", all.len());
        assert_eq!(reachable, Vec::<&String>::new());
        assert_eq!(flagged.len(), cli_only.len(), "{flagged:?}");
    }

    #[derive(Debug, thiserror::Error)]
    enum Outer {
        #[error("wrapper: {0}")]
        Repeats(#[source] Inner),
        #[error("outside")]
        Plain(#[source] Inner),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("inside")]
    struct Inner(#[source] io::Error);

    fn inner() -> Inner {
        Inner(io::Error::from(io::ErrorKind::NotFound))
    }

    #[test]
    fn chain_lists_causes_without_repeats() {
        let not_found = io::Error::from(io::ErrorKind::NotFound).to_string();
        assert_eq!(
            chain(&Outer::Plain(inner())).collect::<Vec<_>>(),
            ["inside".to_string(), not_found.clone()]
        );
        assert_eq!(
            chain(&Outer::Repeats(inner())).collect::<Vec<_>>(),
            [not_found]
        );
    }

    #[test]
    fn chained_is_one_line_without_repeats() {
        assert_eq!(
            chained(&Outer::Plain(inner())).to_string(),
            "outside: inside: entity not found"
        );
        assert_eq!(
            chained(&Outer::Repeats(inner())).to_string(),
            "wrapper: inside: entity not found"
        );
    }

    #[test]
    fn domains_show_through_the_top_level() {
        let err = Error::from(LocalDataError::FileRead {
            path: "/x".into(),
            error: io::Error::from(io::ErrorKind::NotFound),
        });
        assert_eq!(err.to_string(), "could not read /x");
        assert_eq!(chain(&err).count(), 1);
        assert!(matches!(
            Error::from(io::Error::from(io::ErrorKind::NotFound)),
            Error::LocalData(LocalDataError::Io(_))
        ));
    }

    #[test]
    fn revoked_session_is_fatal_and_auth() {
        let revoked = Error::from(LoginError::SessionRevoked);
        let forbidden = Error::from(http::tests::status(403, ""));
        assert!(revoked.is_fatal() && revoked.is_unauthorized());
        assert!(forbidden.is_unauthorized() && !forbidden.is_fatal());
        assert_eq!(
            Error::from(http::tests::status(500, "")).status(),
            Some(500)
        );
    }
}
