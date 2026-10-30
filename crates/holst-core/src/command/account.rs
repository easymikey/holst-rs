//! The signed-in user: who they are, and whether the server accepts the login.

use crate::{
    domain::account::Account,
    error::{Error, Result},
    remote::{api, client::Client},
};

/// What the server says about the client's login.
#[derive(Debug)]
pub enum SessionCheck {
    /// The server accepts it.
    Active(Account),
    /// The server refuses it (revoked, expired, or no access).
    Revoked(Error),
    /// The server could not be asked or answered something else.
    Unknown(Error),
}

/// The user the client's login belongs to; the login keeps what was read.
///
/// # Errors
/// [`Client::send_json`] errors or
/// [`crate::error::http::HttpError::MissingField`] (no user id).
pub async fn account(client: &Client) -> Result<Account> {
    let account = Account::of(&api::user::current(client).await?)?;
    client.learn_account(&account);
    Ok(account)
}

/// Asks the server whether the login is accepted.
pub async fn check(client: &Client) -> SessionCheck {
    match account(client).await {
        Ok(user) => SessionCheck::Active(user),
        Err(err) if err.is_unauthorized() => SessionCheck::Revoked(err),
        Err(err) => SessionCheck::Unknown(err),
    }
}

/// The account whose cache the client's login reads: saved with the login, or
/// asked once and saved with it at the end of the command; `None` on failure.
pub async fn known_account_id(client: &Client) -> Option<String> {
    account_id(client)
        .await
        .inspect_err(|err| tracing::debug!(%err, "board cache: the account is unknown"))
        .ok()
}

/// The id of the signed-in user, who authors what they create: saved with the
/// login, or asked once and saved with it at the end of the command.
///
/// # Errors
/// As [`account`].
pub async fn account_id(client: &Client) -> Result<String> {
    match client.account_id() {
        Some(user) => Ok(user),
        None => Ok(account(client).await?.id),
    }
}
