use reqwest::Url;
use secrecy::SecretString;

use crate::{effect::http::BoxFuture, error::Result};

/// What the board socket needs: its address and the login, both secret.
#[derive(Debug)]
pub struct SocketSession {
    pub(crate) url: Url,
    pub(crate) token: SecretString,
    pub(crate) cookies: SecretString,
    pub(crate) user_agent: String,
}

/// One open board socket: binary messages in and out.
pub trait BoardSocket: Send {
    /// Send one binary message.
    ///
    /// # Errors
    /// [`crate::error::board_socket::BoardSocketError::Connection`] when the
    /// connection fails.
    fn send(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<()>>;
    /// The next binary message; `None` once the server closed.
    ///
    /// # Errors
    /// [`crate::error::board_socket::BoardSocketError::Connection`] when the
    /// connection fails.
    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>>>;
}

/// Opening a board socket with the login headers.
pub trait SocketConnector: Send + Sync {
    /// Connect to `login.url` with the session cookies and `User-Agent`.
    ///
    /// # Errors
    /// [`crate::error::board_socket::BoardSocketError::Connection`] when the
    /// handshake fails.
    fn open<'a>(
        &'a self,
        login: &'a SocketSession,
    ) -> BoxFuture<'a, Result<Box<dyn BoardSocket>>>;
}
