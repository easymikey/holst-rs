//! Errors the form reports; the tools map them into their own.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum FormError {
    #[error("interrupted")]
    Interrupted,

    #[error("cancelled")]
    Cancelled,

    #[error("the terminal stopped responding")]
    Terminal(#[source] io::Error),

    #[error("could not read the input")]
    ReadInput(#[source] io::Error),

    #[error("no input: stdin ended before {what} was given")]
    NoInput { what: &'static str },

    #[error(
        "could not hide the input on this terminal, so the secret would be visible"
    )]
    EchoNotHidden(#[source] io::Error),

    #[error("{question}: {reason}")]
    InvalidAnswer {
        question: &'static str,
        reason: Reason,
    },
}

/// Why an answer was refused, in words for the person typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub struct Reason(pub &'static str);
pub type Result<T, E = FormError> = std::result::Result<T, E>;
