//! Which commands take `output`, and the error for a bad one.

use holst_core::{
    command::{board::BoardVerb, object::ObjectVerb, table::CommandName},
    error::input::InputError,
};
use serde_json::{Map, Value};

use crate::{error::Error, reply::compact::Output};

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
    Output::take(arguments).map_err(|_| {
        holst_core::Error::from(InputError::NotArgumentValue {
            command: name.command().path(),
            name: "output",
            expected: Output::EXPECTED.to_owned(),
        })
        .into()
    })
}
