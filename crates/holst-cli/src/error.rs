//! Errors of the `holst` command and the hints shown under them; pure.

use std::{io, path::PathBuf};

use holst_core::{
    command::storage::CleanPlace,
    domain::{login::Email, session::SessionStoreKind},
    error::{
        confirmation::Unconfirmed,
        hint::{LOGIN, Surface},
        input::JsonSpot,
    },
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Holst(#[from] holst_core::Error),

    #[error("could not read the input")]
    ReadInput(#[source] io::Error),

    #[error("no input: stdin ended before {question} was given")]
    NoInput { question: String },

    #[error("stdout was closed by the reader")]
    StdoutClosed,

    #[error("could not write to stdout")]
    WriteStdout(#[source] io::Error),

    #[error("the terminal stopped responding")]
    Terminal(#[source] io::Error),

    #[error(
        "could not hide the input on this terminal, so the secret would be visible"
    )]
    EchoNotHidden(#[source] io::Error),

    #[error("could not render the server's reply as JSON")]
    Render(#[source] serde_json::Error),

    #[error("the picture is binary and stdout is a terminal")]
    BinaryToTerminal,

    #[error("the SVG picture is not printed on a terminal")]
    SvgToTerminal,

    #[error(
        "there is no home directory to find ~/Downloads in: give a file or --to <dir>"
    )]
    NoDownloads,

    #[error(
        "\"{reference}\" is a reference from the MCP server; give an id, a link or a name instead"
    )]
    McpAlias { reference: String },

    #[error("interrupted")]
    Interrupted,
    #[error("terminated")]
    Terminated,
    #[error("the terminal hung up")]
    HungUp,

    #[error("the saved login could not be reached")]
    SessionStoreLost,

    #[error("login cancelled")]
    Cancelled,

    // The form showed the inner error under its `└`; its exit code is left.
    #[error(transparent)]
    Shown(Box<Error>),

    #[error("{question}: {reason}")]
    InvalidAnswer { question: String, reason: String },

    #[error("there is no account for {email}")]
    NoAccount { email: Email },

    #[error("{email} can only log in through SSO")]
    SsoOnly { email: Email },

    #[error("the {secret} did not match")]
    WrongSecret {
        secret: SecretKind,
        #[source]
        error: holst_core::Error,
    },

    #[error("could not remove the saved login from the {store}")]
    NotForgotten {
        store: SessionStoreKind,
        #[source]
        error: holst_core::Error,
    },

    #[error("could not remove {place}")]
    NotCleaned {
        place: CleanPlace,
        #[source]
        error: holst_core::Error,
    },

    #[error("login finished, but the server did not accept the new session")]
    SessionNotAccepted(#[source] holst_core::Error),

    #[error(transparent)]
    Unconfirmed(Unconfirmed),

    #[error("\"{item}\" is not key=value: expected key=value, e.g. limit=50")]
    QueryNoEquals { item: String },

    #[error("could not read {}", .path.display())]
    BodyFile {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("the items are not a JSON array or JSON lines")]
    ItemsNotJson {
        spot: JsonSpot,
        #[source]
        error: serde_json::Error,
    },
}

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "clacky's error is non-exhaustive; the rest are mistakes of the program, not of the person"
)]
impl From<clacky::Error> for Error {
    fn from(error: clacky::Error) -> Self {
        match error {
            clacky::Error::Interrupted => Error::Interrupted,
            clacky::Error::Cancelled => Error::Cancelled,
            // Only the signal watcher ends a question from outside.
            clacky::Error::Aborted => crate::terminal::signals::heard()
                .map_or(Error::Interrupted, Error::from),
            clacky::Error::Terminal(error) => Error::Terminal(error),
            clacky::Error::EchoNotHidden(error) => Error::EchoNotHidden(error),
            clacky::Error::NoInput { question } => Error::NoInput { question },
            clacky::Error::NoChoices => Error::NoInput {
                question: "a choice".to_owned(),
            },
            clacky::Error::Refused { question, reason } => {
                Error::InvalidAnswer { question, reason }
            }
            other => Error::Terminal(io::Error::other(other.to_string())),
        }
    }
}

/// Why an answer was refused, in words for the person typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub(crate) enum Reason {
    #[display("this cannot be empty")]
    Empty,
    #[display("enter an address like you@example.com")]
    NotEmail,
    #[display("that code did not match — check the latest email and type it again")]
    WrongCode,
    #[display("that password did not match — try again")]
    WrongPassword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub(crate) enum SecretKind {
    #[display("login code")]
    Code,
    #[display("password")]
    Password,
}

impl SecretKind {
    #[must_use]
    pub(crate) const fn wrong_answer(self) -> Reason {
        match self {
            SecretKind::Code => Reason::WrongCode,
            SecretKind::Password => Reason::WrongPassword,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub(crate) enum CommandLineHint {
    #[display("{_0}")]
    Core(&'static str),
    #[display("run `{LOGIN}` again and type the code from the new email")]
    NewCode,
    #[display("check the password, or log in with a login code")]
    CheckPassword,
    #[display("check the address, or sign up at app.holst.so")]
    CheckAddress,
    #[display("log in with an email code or a password — SSO is not supported")]
    NoSso,
    #[display("run it in a terminal so the form can ask again")]
    RunInTerminal,
    #[display("save it to a file, or redirect stdout")]
    UseOut,
    #[display(r#"pipe the code instead: `printf '%s\n' "$CODE" | {LOGIN}`"#)]
    PipeSecret,
}

impl Error {
    #[must_use]
    pub(crate) fn shown(self) -> Self {
        Error::Shown(Box::new(self))
    }

    #[must_use]
    pub(crate) fn hint(&self) -> Option<CommandLineHint> {
        let core = |hint: holst_core::error::hint::Hint| {
            CommandLineHint::Core(hint.text(Surface::Cli))
        };
        match self {
            Error::Holst(error)
            | Error::SessionNotAccepted(error)
            | Error::NotForgotten { error, .. } => error.hint().map(core),
            Error::Unconfirmed(error) => error.hint().map(core),
            Error::WrongSecret {
                secret: SecretKind::Code,
                ..
            } => Some(CommandLineHint::NewCode),
            Error::WrongSecret {
                secret: SecretKind::Password,
                ..
            } => Some(CommandLineHint::CheckPassword),
            Error::NoAccount { .. } => Some(CommandLineHint::CheckAddress),
            Error::SsoOnly { .. } => Some(CommandLineHint::NoSso),
            Error::InvalidAnswer { .. } | Error::NoInput { .. } => {
                Some(CommandLineHint::RunInTerminal)
            }
            Error::BinaryToTerminal | Error::SvgToTerminal => {
                Some(CommandLineHint::UseOut)
            }
            Error::EchoNotHidden(_) => Some(CommandLineHint::PipeSecret),
            Error::ReadInput(_)
            | Error::StdoutClosed
            | Error::WriteStdout(_)
            | Error::Terminal(_)
            | Error::Render(_)
            | Error::McpAlias { .. }
            | Error::Interrupted
            | Error::Terminated
            | Error::HungUp
            | Error::SessionStoreLost
            | Error::Cancelled
            | Error::Shown(_)
            | Error::NoDownloads
            | Error::QueryNoEquals { .. }
            | Error::ItemsNotJson { .. }
            | Error::BodyFile { .. }
            | Error::NotCleaned { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use holst_core::{
        domain::session::SessionStoreKind,
        error::{
            confirmation::RiskyChange,
            lookup::{LookupError, LookupKind, LookupMiss},
        },
        fake::candidates,
    };
    use rstest::rstest;

    use crate::error::{Error, Reason, SecretKind};

    #[test]
    fn not_forgotten_names_the_store() {
        let error = Error::NotForgotten {
            store: SessionStoreKind::File,
            error: holst_core::Error::LocalData(
                holst_core::error::local_data::LocalDataError::KeychainLocked,
            ),
        };
        assert_eq!(
            error.to_string(),
            "could not remove the saved login from the file"
        );
    }

    fn lookup(what: LookupKind, outcome: LookupMiss) -> Error {
        Error::Holst(holst_core::Error::Lookup(LookupError {
            kind: what,
            reference: "x".into(),
            miss: outcome,
        }))
    }

    fn status(code: u16) -> Error {
        Error::Holst(holst_core::Error::Http(holst_core::fake::http_status(
            code,
            holst_core::error::http::ReplyBody::Empty,
        )))
    }

    #[rstest]
    #[case::unauthorized(status(401), Some("holst auth login"))]
    #[case::ambiguous_board(
        lookup(LookupKind::Board, LookupMiss::ambiguous(candidates(&["a", "b"]))),
        Some("--workspace")
    )]
    #[case::folder_not_found(
        lookup(LookupKind::Folder, LookupMiss::not_found()),
        Some("holst b ls")
    )]
    #[case::trash_not_found(
        lookup(LookupKind::Trash, LookupMiss::not_found()),
        Some("holst b ls --view trash")
    )]
    #[case::folder_ambiguous(
        lookup(LookupKind::Folder, LookupMiss::ambiguous(candidates(&["a", "b"]))),
        Some("one of the ids")
    )]
    #[case::unconfirmed(
        Error::Unconfirmed(RiskyChange::DeleteTemplate { template: "Retro".into() }.into()),
        Some("`--yes`")
    )]
    #[case::closed_stdout(Error::StdoutClosed, None)]
    #[case::binary(Error::BinaryToTerminal, Some("save it to a file"))]
    #[case::svg(Error::SvgToTerminal, Some("save it to a file"))]
    fn hints(#[case] error: Error, #[case] expected: Option<&str>) {
        let got = error.hint().map(|hint| hint.to_string());
        assert!(hint_matches(got.as_deref(), expected), "{got:?}");
    }

    fn hint_matches(got: Option<&str>, expected: Option<&str>) -> bool {
        match (got, expected) {
            (Some(got), Some(want)) => got.contains(want),
            (got, want) => got == want,
        }
    }

    #[test]
    fn rejected_token_hints_by_its_cause() {
        let error = Error::SessionNotAccepted(holst_core::Error::Http(
            holst_core::fake::http_status(
                401,
                holst_core::error::http::ReplyBody::Empty,
            ),
        ));
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.to_string().contains("holst auth login"))
        );
    }

    #[test]
    fn refused_secret_maps_to_its_complaint() {
        assert_eq!(SecretKind::Code.wrong_answer(), Reason::WrongCode);
        assert_eq!(SecretKind::Password.wrong_answer(), Reason::WrongPassword);
        assert_eq!(SecretKind::Code.to_string(), "login code");
    }
}
