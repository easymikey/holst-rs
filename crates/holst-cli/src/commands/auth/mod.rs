//! `holst auth`: log in, log out and the status of the saved login.

pub(crate) mod login;
mod logout;
mod status;

use holst_core::command::{auth::AuthVerb, table};
use serde_json::{Map, Value};

use crate::{error::Result, output::OutputFormat, run::RunSettings};

pub(crate) async fn run(
    verb: AuthVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match verb {
        AuthVerb::Login => login::login(table::parse(arguments)?, run, format).await,
        AuthVerb::Logout => logout::logout(table::parse(arguments)?, run, format).await,
        AuthVerb::Status => status::status(table::parse(arguments)?, run, format).await,
    }
}
