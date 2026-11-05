//! Errors of the MCP server: its own refusals on top of the core errors.

use std::path::PathBuf;

use holst_core::{
    command::table::Command,
    error::{
        confirmation::Unconfirmed,
        hint::{Hint, Surface},
    },
};

mod call;
mod http;
mod setup;
mod start;

pub(crate) use self::{
    call::{CallError, UnknownAlias, Verbs},
    http::HttpError,
    setup::SetupError,
    start::StartError,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Holst(#[from] holst_core::Error),

    #[error(transparent)]
    Alias(#[from] UnknownAlias),

    #[error("the search query must not be empty")]
    EmptyQuery,

    #[error("`-` is for the command line: give a path")]
    DashPath,

    #[error(
        "`{}` takes at most {max} items per call, not {given}: split them over several calls",
        .command.path()
    )]
    TooManyItems {
        command: &'static Command,
        max: usize,
        given: usize,
    },

    #[error("the original goes into a new file: there is no standard output here")]
    NoStdout,

    #[error("{} is for the command line: it has no tool", .command.path())]
    CliOnly { command: &'static Command },

    #[error(transparent)]
    Unconfirmed(Unconfirmed),

    #[error("the file cannot be read")]
    Unreadable {
        #[source]
        source: std::io::Error,
    },

    #[error("only a regular file can be uploaded")]
    NotAFile,

    #[error("{} is a secret; uploading it is not allowed", .path.display())]
    Secret { path: PathBuf },

    #[error("{} is a secret place; saving files there is not allowed", .path.display())]
    SecretPlace { path: PathBuf },

    #[error(
        "the JSON reply is {bytes} bytes, over the {max} byte limit: ask for a smaller page: pass a lower `limit`, or use output text"
    )]
    JsonTooLong { bytes: usize, max: usize },

    #[error("the login store could not be reached: its background task stopped")]
    StoreAborted,
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::Io(err),
        ))
    }
}

impl Error {
    #[must_use]
    fn hint(&self) -> Option<Hint> {
        match self {
            Error::Holst(err) => err.hint(),
            Error::Unconfirmed(err) => err.hint(),
            Error::Alias(_)
            | Error::EmptyQuery
            | Error::DashPath
            | Error::TooManyItems { .. }
            | Error::NoStdout
            | Error::CliOnly { .. }
            | Error::Unreadable { .. }
            | Error::NotAFile
            | Error::Secret { .. }
            | Error::SecretPlace { .. }
            | Error::JsonTooLong { .. }
            | Error::StoreAborted => None,
        }
    }

    #[must_use]
    pub(crate) fn advice(&self) -> Option<&'static str> {
        self.hint().map(|hint| hint.text(Surface::Mcp))
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use holst_core::{
        command::storage::StorageVerb,
        error::{
            confirmation::{RiskyChange, Unconfirmed},
            hint::Hint,
        },
    };

    use crate::error::Error;

    #[test]
    fn hint_comes_from_the_core_and_confirmations() {
        assert_eq!(
            Error::Holst(holst_core::Error::Login(
                holst_core::error::login::LoginError::SessionRevoked
            ))
            .hint(),
            Some(Hint::LoginAgain)
        );
        assert_eq!(Error::NoStdout.hint(), None);
        let unconfirmed =
            Error::Unconfirmed(Unconfirmed::from(RiskyChange::ChangeSharing {
                board: "@2".to_owned(),
            }));
        assert_eq!(unconfirmed.hint(), Some(Hint::ConfirmFirst));
        assert_eq!(
            unconfirmed.to_string(),
            "changing the sharing settings of @2 changes who can open or edit it"
        );
    }

    #[test]
    fn refusals_name_the_command_they_were_given() {
        let command = StorageVerb::Ls.command();
        assert_eq!(
            Error::CliOnly { command }.to_string(),
            "storage ls is for the command line: it has no tool"
        );
        assert_eq!(
            Error::TooManyItems {
                command,
                max: 2,
                given: 3
            }
            .to_string(),
            "`storage ls` takes at most 2 items per call, not 3: split them over several calls"
        );
        assert_eq!(
            Error::DashPath.to_string(),
            "`-` is for the command line: give a path"
        );
    }
}
