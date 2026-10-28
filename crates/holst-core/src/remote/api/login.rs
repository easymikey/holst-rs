use holst_macros::post;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use crate::{domain::login::Email, error::Result, remote::client::Client};

/// The ways to log in for an address, as the server words them.
#[post(
    "login/get_public_options",
    anonymous,
    body = json!({ "email": email.as_str() })
)]
pub async fn public_options(client: &Client, email: &Email) -> Result<Value>;

/// Ask the server to email a one-time code.
#[post(
    "send_temporary_password",
    anonymous,
    body = json!({ "email": email.as_str() })
)]
pub async fn send_temporary_password(client: &Client, email: &Email) -> Result<Value>;

/// Log in with a code or password, as typed; the reply holds the token.
#[post(
    "login/with_email",
    anonymous,
    body = json!({ "email": email.as_str(), "password": password.expose_secret() })
)]
pub async fn with_email(
    client: &Client,
    email: &Email,
    password: &SecretString,
) -> Result<Value>;

/// End the session on the server; the client still holds it.
#[post("logout")]
pub async fn end_server_session(client: &Client) -> Result<Value>;
