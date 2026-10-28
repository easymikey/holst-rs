//! Errors of the MCP server: its own refusals on top of the core errors.

use std::path::PathBuf;

use holst_core::{
    effect::http::HttpMethod,
    error::{
        confirmation::Unconfirmed,
        hint::{Hint, Surface},
    },
};

use crate::memory::alias::UnknownAlias;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Holst(#[from] holst_core::Error),

    #[error(transparent)]
    Alias(#[from] UnknownAlias),

    #[error("the search query must not be empty")]
    EmptyQuery,

    #[error("missing target: {what}")]
    NoArgument { what: &'static str },

    #[error(
        "`{command}` takes at most {max} items per call, not {given}: split them over several calls"
    )]
    TooManyItems {
        command: String,
        max: usize,
        given: usize,
    },

    #[error("the original goes into a new file: there is no standard output here")]
    NoStdout,

    #[error("{command} is for the command line: it has no tool")]
    CliOnly { command: String },

    #[error("{}", .0.on(Surface::Mcp))]
    Unconfirmed(Unconfirmed),

    #[error(
        "{method} through api is disabled because it bypasses confirmations; only \
         a human can enable it with HOLST_MCP_RAW_WRITE=1 in the server settings"
    )]
    RawWriteDisabled { method: HttpMethod },

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
            | Error::NoArgument { .. }
            | Error::TooManyItems { .. }
            | Error::NoStdout
            | Error::CliOnly { .. }
            | Error::RawWriteDisabled { .. }
            | Error::Unreadable { .. }
            | Error::NotAFile
            | Error::Secret { .. }
            | Error::SecretPlace { .. }
            | Error::JsonTooLong { .. }
            | Error::StoreAborted => None,
        }
    }

    #[must_use]
    pub(crate) fn advice(&self) -> Option<holst_core::error::hint::Advice> {
        self.hint().map(|hint| hint.on(Surface::Mcp))
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use holst_core::error::{
        confirmation::{ChangeRisk, Unconfirmed},
        hint::Hint,
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
        let unconfirmed = Error::Unconfirmed(Unconfirmed::new(
            "granting a right",
            ChangeRisk::Sharing,
        ));
        assert_eq!(unconfirmed.hint(), Some(Hint::ConfirmFirst));
        assert_eq!(
            unconfirmed.to_string(),
            "granting a right changes who can open or edit it; confirm with confirm: true"
        );
    }
}
