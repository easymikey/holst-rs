//! Errors of the `holst` command and the hints shown under them; pure.

use std::{io, path::PathBuf};

use holst_core::{
    domain::login::Email,
    error::{
        confirmation::Unconfirmed,
        hint::{Advice, LOGIN, Surface},
    },
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Holst(#[from] holst_core::Error),

    #[error("could not read the input")]
    ReadInput(#[source] io::Error),

    #[error("no input: stdin ended before {what} was given")]
    NoInput { what: &'static str },

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

    // The form showed the inner error under its `└`; only its exit code is left.
    #[error("{0}; already shown")]
    Shown(Box<Error>),

    #[error("{question}: {reason}")]
    InvalidAnswer {
        question: &'static str,
        reason: InvalidAnswer,
    },

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
        store: &'static str,
        #[source]
        error: holst_core::Error,
    },

    #[error("could not remove {name}: {error}")]
    NotCleaned { name: String, error: String },

    #[error("login finished, but the server did not accept the new session")]
    SessionNotAccepted(#[source] holst_core::Error),

    #[error("{}", .0.on(Surface::Cli))]
    Unconfirmed(Unconfirmed),

    #[error("expected key=value, got \"{item}\"")]
    QueryNoEquals { item: String },

    #[error("could not read {}", .path.display())]
    BodyFile {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("the items are not a JSON array or JSON lines")]
    ItemsNotJson(#[source] serde_json::Error),
}

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum InvalidAnswer {
    #[error("enter an address like you@example.com")]
    NotEmail,
    #[error("this cannot be empty")]
    Empty,
    #[error("that code did not match — check the latest email and type it again")]
    WrongCode,
    #[error("that password did not match — try again")]
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
    pub(crate) const fn wrong_answer(self) -> InvalidAnswer {
        match self {
            SecretKind::Code => InvalidAnswer::WrongCode,
            SecretKind::Password => InvalidAnswer::WrongPassword,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub(crate) enum CommandLineHint {
    #[display("{_0}")]
    Core(Advice),
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
            CommandLineHint::Core(hint.on(Surface::Cli))
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
            | Error::ItemsNotJson(_)
            | Error::BodyFile { .. }
            | Error::NotCleaned { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use holst_core::error::{
        confirmation::Unconfirmed,
        lookup::{Candidate, Candidates, LookupError, LookupKind, LookupMiss},
    };
    use rstest::rstest;

    use crate::error::{Error, InvalidAnswer, SecretKind};

    fn handled(names: &[&str]) -> Candidates {
        names
            .iter()
            .map(|name| Candidate {
                name: (*name).to_owned(),
                ..Candidate::default()
            })
            .collect()
    }

    fn lookup(what: LookupKind, outcome: LookupMiss) -> Error {
        Error::Holst(holst_core::Error::Lookup(LookupError {
            kind: what,
            reference: "x".into(),
            miss: outcome,
        }))
    }

    fn status(code: u16) -> Error {
        Error::Holst(holst_core::Error::Http(
            holst_core::error::http::HttpError::Status {
                method: "GET",
                path: "/gapi/current_user".into(),
                status: code,
                detail: String::new(),
            },
        ))
    }

    #[rstest]
    #[case::unauthorized(status(401), Some("holst auth login"))]
    #[case::ambiguous_board(
        lookup(LookupKind::Board, LookupMiss::ambiguous(handled(&["a", "b"]))),
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
        lookup(LookupKind::Folder, LookupMiss::ambiguous(handled(&["a", "b"]))),
        Some("one of the ids")
    )]
    #[case::unconfirmed(
        Error::Unconfirmed(Unconfirmed::new(
            "deleting a template",
            holst_core::error::confirmation::ChangeRisk::Irreversible
        )),
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
            holst_core::error::http::HttpError::Status {
                method: "GET",
                path: "/gapi/current_user".into(),
                status: 401,
                detail: String::new(),
            },
        ));
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.to_string().contains("holst auth login"))
        );
    }

    #[test]
    fn refused_secret_maps_to_its_complaint() {
        assert_eq!(SecretKind::Code.wrong_answer(), InvalidAnswer::WrongCode);
        assert_eq!(
            SecretKind::Password.wrong_answer(),
            InvalidAnswer::WrongPassword
        );
        assert_eq!(SecretKind::Code.to_string(), "login code");
    }
}
