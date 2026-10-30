//! The `output` argument: which commands take it, and what it asks for.

use holst_core::{
    command::{board::BoardVerb, object::ObjectVerb, table::CommandName},
    error::input::InputError,
};
use serde_json::{Map, Value};

use crate::{error::Error, reply::compact::render_json};

/// The `-o` of the command line, which the table leaves alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    Text,
    Json,
}

impl Output {
    pub(crate) const NAMES: [&str; 2] = ["text", "json"];

    /// The reply: `text` for the model, or `listing` as the CLI prints it.
    pub(crate) fn reply(
        self,
        listing: &impl serde::Serialize,
        text: impl FnOnce() -> String,
    ) -> Result<String, Error> {
        match self {
            Output::Text => Ok(text()),
            Output::Json => render_json(listing),
        }
    }
}

/// Whether the command lists things, and so takes `output`.
pub(crate) fn on(name: CommandName) -> bool {
    matches!(
        name,
        CommandName::Board(BoardVerb::Ls | BoardVerb::Show)
            | CommandName::Object(ObjectVerb::Ls)
    )
}

/// Takes `output` out of the arguments of the command `name`.
pub(crate) fn take(
    arguments: &mut Map<String, Value>,
    name: CommandName,
) -> Result<Output, Error> {
    let given = arguments.remove("output").filter(|value| !value.is_null());
    match given.as_ref().map(Value::as_str) {
        None | Some(Some("text")) => Ok(Output::Text),
        Some(Some("json")) => Ok(Output::Json),
        Some(Some(_) | None) => {
            Err(holst_core::Error::from(InputError::NotArgumentValue {
                command: name.command().path(),
                name: "output",
                expected: "one of text, json".to_owned(),
            })
            .into())
        }
    }
}
