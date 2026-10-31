//! What can go wrong in `holst-mcp setup`.

use holst_form::error::FormError;

use crate::setup::{ATTEMPTS, Secret};

#[derive(Debug, thiserror::Error)]
pub(crate) enum SetupError {
    #[error("no Holst account uses that address")]
    NoAccount,

    #[error(
        "that account signs in through single sign-on, which `holst-mcp setup` does not do"
    )]
    SsoOnly,

    #[error(
        "the {} was not accepted {ATTEMPTS} times: run `holst-mcp setup` to try again",
        .0.noun()
    )]
    Refused(Secret),

    #[error("there is no saved login: run `holst-mcp setup` in a terminal")]
    NoTerminal,

    #[error(
        "unknown client {0}: `--clients` takes `claude` or `codex`, comma-separated"
    )]
    UnknownClient(String),

    #[error(transparent)]
    Form(#[from] FormError),

    #[error("could not log in")]
    Holst(#[from] holst_core::Error),
}
