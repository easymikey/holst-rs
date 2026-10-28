//! The board server (hud) and the WebSocket to it.

use std::io;

use crate::error::hint::Hint;

/// The board could not be loaded: the server refused, hung up or sent nonsense.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum BoardSocketError {
    #[error("the board server sent a malformed frame: {what}")]
    MalformedFrame { what: &'static str },

    #[error("the board server refused the board: {code}")]
    NotSubscribed { code: SocketErrorCode },

    #[error("the board connection closed before the board loaded")]
    Closed,

    #[error("the board server sent nothing for {seconds} s while the board loaded")]
    Timeout { seconds: u64 },

    #[error(
        "the board was still loading after {seconds} s, the longest a load may take"
    )]
    TotalTimeout { seconds: u64 },

    #[error("the board server sent an update link to another host: {host}")]
    ForeignLink { host: String },

    #[error("the board content could not be decoded")]
    Decode(#[from] yrs::encoding::read::Error),

    #[error("the board content could not be applied")]
    UpdateNotMerged(#[from] yrs::error::UpdateError),

    #[error("the board connection failed")]
    Connection(#[from] Box<tungstenite::Error>),

    #[error("the board connection could not go through the proxy {proxy}")]
    Proxy {
        proxy: String,
        #[source]
        error: io::Error,
    },

    #[error("the proxy {proxy} refused the board connection: {detail}")]
    ProxyDenied { proxy: String, detail: String },

    #[error(
        "the board connection goes only through an http:// proxy, not {scheme}://{host}"
    )]
    ProxyScheme { scheme: String, host: String },

    #[error("the cached copy of the board is ahead of the server; it was dropped")]
    CopyAhead,

    #[error(
        "the board did not load completely: its document waits for updates the server did \
         not send; nothing was read or changed"
    )]
    Incomplete,

    #[error(
        "this board is stored in format v{schema} on {provider}, which this version \
         cannot read or write; open it in the web client"
    )]
    UnsupportedFormat { schema: u32, provider: String },
}

/// Why the board server refused a board, parsed once from the code it sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SocketErrorCode {
    #[error("forbidden")]
    Forbidden,

    #[error("not found")]
    NotFound,

    /// A board that does not exist is an `internal-error` there.
    #[error("internal-error")]
    InternalError,

    /// A code this program does not know, as the server sent it.
    #[error("{0}")]
    Other(String),
}

impl SocketErrorCode {
    /// Reads the code of an `error` field of the answer frame.
    #[must_use]
    pub fn from_code(code: String) -> Self {
        match code.as_str() {
            "forbidden" => SocketErrorCode::Forbidden,
            "not-found" | "not_found" | "notFound" | "not found" => {
                SocketErrorCode::NotFound
            }
            "internal-error" => SocketErrorCode::InternalError,
            _ => SocketErrorCode::Other(code),
        }
    }
}

impl BoardSocketError {
    /// What to do next, decided for every variant.
    #[must_use]
    pub fn hint(&self) -> Option<Hint> {
        match self {
            BoardSocketError::Connection(_)
            | BoardSocketError::Proxy { .. }
            | BoardSocketError::ProxyDenied { .. }
            | BoardSocketError::ProxyScheme { .. } => Some(Hint::CheckNetwork),
            BoardSocketError::Incomplete
            | BoardSocketError::Closed
            | BoardSocketError::Timeout { .. }
            | BoardSocketError::TotalTimeout { .. } => Some(Hint::RetryLater),
            BoardSocketError::NotSubscribed {
                code: SocketErrorCode::Forbidden,
            } => Some(Hint::AskOwner),
            BoardSocketError::NotSubscribed { .. } => Some(Hint::CheckId),
            BoardSocketError::MalformedFrame { .. }
            | BoardSocketError::ForeignLink { .. }
            | BoardSocketError::Decode(_)
            | BoardSocketError::UpdateNotMerged(_)
            | BoardSocketError::UnsupportedFormat { .. }
            | BoardSocketError::CopyAhead => None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::io;

    use crate::error::{
        board_socket::{
            BoardSocketError,
            BoardSocketErrorDiscriminants,
            SocketErrorCode,
        },
        hint::{Hint, decided::assert_every_variant},
    };

    pub(in crate::error) fn samples() -> Vec<(BoardSocketError, Option<Hint>)> {
        let proxy = || String::from("proxy:3128");
        let parent = yrs::error::UpdateError::InvalidParent(
            yrs::ID::new(yrs::ClientID::new(1), 0),
            0,
        );
        vec![
            (BoardSocketError::MalformedFrame { what: "x" }, None),
            (
                BoardSocketError::NotSubscribed {
                    code: SocketErrorCode::Forbidden,
                },
                Some(Hint::AskOwner),
            ),
            (
                BoardSocketError::NotSubscribed {
                    code: SocketErrorCode::InternalError,
                },
                Some(Hint::CheckId),
            ),
            (BoardSocketError::Closed, Some(Hint::RetryLater)),
            (
                BoardSocketError::Timeout { seconds: 30 },
                Some(Hint::RetryLater),
            ),
            (
                BoardSocketError::TotalTimeout { seconds: 600 },
                Some(Hint::RetryLater),
            ),
            (BoardSocketError::ForeignLink { host: "x".into() }, None),
            (
                BoardSocketError::Decode(yrs::encoding::read::Error::InvalidVarInt),
                None,
            ),
            (BoardSocketError::UpdateNotMerged(parent), None),
            (
                BoardSocketError::Connection(Box::new(
                    tungstenite::Error::ConnectionClosed,
                )),
                Some(Hint::CheckNetwork),
            ),
            (
                BoardSocketError::Proxy {
                    proxy: proxy(),
                    error: io::Error::from(io::ErrorKind::ConnectionRefused),
                },
                Some(Hint::CheckNetwork),
            ),
            (
                BoardSocketError::ProxyDenied {
                    proxy: proxy(),
                    detail: "HTTP/1.1 403".into(),
                },
                Some(Hint::CheckNetwork),
            ),
            (
                BoardSocketError::ProxyScheme {
                    scheme: "https".into(),
                    host: proxy(),
                },
                Some(Hint::CheckNetwork),
            ),
            (BoardSocketError::Incomplete, Some(Hint::RetryLater)),
            (
                BoardSocketError::UnsupportedFormat {
                    schema: 3,
                    provider: "hud".into(),
                },
                None,
            ),
            (BoardSocketError::CopyAhead, None),
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, BoardSocketErrorDiscriminants>(
            samples(),
            BoardSocketError::hint,
        );
    }
}
