//! The commands, the signed-in client and the arguments they share.

pub(crate) mod api;
pub(crate) mod auth;
pub(crate) mod backup;
pub(crate) mod board;
pub(crate) mod comment;
#[cfg(test)] pub(crate) mod fake;
pub(crate) mod object;
pub(crate) mod search;
pub(crate) mod storage;
pub(crate) mod template;
pub(crate) mod workspace;

use std::io::Read;

use holst_core::{
    command::table::{CommandName, ValueKind},
    error::confirmation::Unconfirmed,
};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    output::OutputFormat,
    run::RunSettings,
};

/// Turns the refusal of a held change into the exit code.
pub(crate) fn refused(refusal: Option<&Unconfirmed>) -> Result<()> {
    refusal.map_or(Ok(()), |refusal| Err(Error::Unconfirmed(refusal.clone())))
}

/// Runs a command of the table with the arguments the command line gave it.
pub(crate) async fn run(
    name: CommandName,
    mut arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    read_files(name, &mut arguments, std::io::stdin().lock())?;
    match name {
        CommandName::Auth(verb) => auth::run(verb, arguments, run, format).await,
        CommandName::Workspace(verb) => {
            workspace::run(verb, arguments, run, format).await
        }
        CommandName::Board(verb) => board::run(verb, arguments, run, format).await,
        CommandName::Comment(verb) => comment::run(verb, arguments, run, format).await,
        CommandName::Template(verb) => {
            template::run(verb, arguments, run, format).await
        }
        CommandName::Backup(verb) => backup::run(verb, arguments, run, format).await,
        CommandName::Storage(verb) => storage::run(verb, arguments, run, format).await,
        CommandName::Object(verb) => object::run(verb, arguments, run, format).await,
        CommandName::Search => search::run(arguments, run, format).await,
        CommandName::Api => api::run(arguments, run, format).await,
    }
}

/// Replaces the `@FILE` and `-` that a text-or-file argument was given as with
/// the text they stand for.
fn read_files(
    name: CommandName,
    arguments: &mut Map<String, Value>,
    mut stdin: impl Read,
) -> Result<()> {
    name.command()
        .arguments
        .iter()
        .filter(|argument| argument.kind == ValueKind::TextOrFile)
        .try_for_each(|argument| match arguments.get_mut(argument.name) {
            Some(Value::String(given)) => {
                *given = text_of(given, &mut stdin)?;
                Ok(())
            }
            Some(_) | None => Ok(()),
        })
}

fn text_of(given: &str, stdin: &mut impl Read) -> Result<String> {
    match (given, given.strip_prefix('@')) {
        ("-", _) => read_source("-", stdin),
        (_, Some(path)) => read_source(path, stdin),
        (text, None) => Ok(text.to_owned()),
    }
}

/// Returns the text of the file, or of stdin for `-`.
pub(crate) fn read_source(source: &str, stdin: &mut impl Read) -> Result<String> {
    if source == "-" {
        let mut text = String::new();
        stdin.read_to_string(&mut text).map_err(Error::ReadInput)?;
        Ok(text)
    } else {
        std::fs::read_to_string(source).map_err(|error| Error::BodyFile {
            path: source.into(),
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use holst_core::command::{table::CommandName, template::TemplateVerb};
    use serde_json::{Map, Value, json};

    use crate::{commands::read_files, error::Error};

    fn read(given: &str, stdin: &str) -> Result<Value, Error> {
        let mut arguments = Map::new();
        arguments.insert("template".to_owned(), json!("t1"));
        arguments.insert("body".to_owned(), json!(given));
        read_files(
            CommandName::Template(TemplateVerb::Edit),
            &mut arguments,
            stdin.as_bytes(),
        )?;
        Ok(Value::Object(arguments))
    }

    #[test]
    fn a_text_is_kept_as_given() {
        assert_eq!(
            read("{}", "").ok(),
            Some(json!({ "template": "t1", "body": "{}" }))
        );
    }

    #[test]
    fn a_dash_reads_stdin() {
        assert_eq!(
            read("-", "{\"a\":1}").ok(),
            Some(json!({ "template": "t1", "body": "{\"a\":1}" }))
        );
    }

    #[test]
    fn a_missing_file_is_named() {
        assert!(matches!(
            read("@/nowhere/body.json", ""),
            Err(Error::BodyFile { .. })
        ));
    }
}
