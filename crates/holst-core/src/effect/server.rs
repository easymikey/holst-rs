//! The board server as the interpreters see it: the calls they make on it.

use serde_json::Value;

use crate::{
    effect::{http::BoxFuture, request::ApiRequest, socket::SocketSession},
    error::Result,
};

/// The server behind a client; an adapter implements it.
pub trait HolstServer: Sync {
    /// Sends `request`; the reply as JSON, empty as `null`.
    ///
    /// # Errors
    /// No or revoked login, a bad address, a failed status, a foreign redirect
    /// or a network failure.
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<Value>>;

    /// Downloads a snapshot `link` from the board server, without the login.
    ///
    /// # Errors
    /// A link off the server's host, a transport failure or a failed status.
    fn download_link<'a>(&'a self, link: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;

    /// The board socket address and the login it sends.
    ///
    /// # Errors
    /// [`crate::error::login::LoginError::NoSavedLogin`] or
    /// [`crate::error::login::LoginError::SessionRevoked`] without a live
    /// login.
    fn socket_session(&self) -> Result<SocketSession>;
}
