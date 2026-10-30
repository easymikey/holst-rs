//! The commands, the signed-in client and the arguments they share.

pub(crate) mod api;
mod ask;
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
    command::{
        change::Outcome,
        table::{self, CommandArgs, CommandName, ValueKind},
    },
    error::confirmation::Unconfirmed,
    remote::client::Client,
};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    form::{Form, terminal::Console},
    output::OutputFormat,
    run::{Prompting, RunSettings, run_signed_in},
};

/// How many times a run may be repeated after an answer.
const ROUNDS: usize = 3;

/// Turns the refusal of a held change into the exit code.
pub(crate) fn refused(refusal: Option<&Unconfirmed>) -> Result<()> {
    refusal.map_or(Ok(()), |refusal| Err(Error::Unconfirmed(refusal.clone())))
}

/// Parses, makes one change and prints it. The plan of a change held for its
/// confirmation is shown first; the refusal is the exit code.
pub(crate) async fn change<A: CommandArgs, R: Outcome>(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
    make: impl AsyncFn(&Client, &A) -> holst_core::Result<R>,
    render: impl Fn(OutputFormat, &R) -> Result<()>,
) -> Result<()> {
    let args = table::parse::<A>(arguments)?;
    run_signed_in(run, async |client| {
        let done = make(client, &args).await?;
        render(format, &done)?;
        refused(done.refusal())
    })
    .await
}

#[expect(
    clippy::disallowed_methods,
    reason = "the command runner hands the process's stdin to the body-file readers"
)]
pub(crate) async fn run(
    name: CommandName,
    mut arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    read_files(name, &mut arguments, std::io::stdin().lock())?;
    rounds(name, arguments, run, format, ROUNDS).await
}

/// Runs the command; a held one that a person may answer is asked, and
/// repeated with the answer.
async fn rounds(
    name: CommandName,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
    left: usize,
) -> Result<()> {
    match group(name, arguments.clone(), run, format).await {
        Err(error) if left > 0 => match asked(&error, &arguments, run)? {
            Some(next) => {
                Box::pin(rounds(name, next, run, format, left.saturating_sub(1))).await
            }
            None => Err(error),
        },
        done => done,
    }
}

/// The arguments of the repeated run, or `None` when nothing is asked.
fn asked(
    error: &Error,
    arguments: &Map<String, Value>,
    run: &RunSettings,
) -> Result<Option<Map<String, Value>>> {
    match (run.prompting(), ask::question(error, arguments)) {
        (Prompting::Keys | Prompting::Lines, Some(question)) => {
            let form = Form::bare(Console::detect(run.terminal_type()));
            ask::put(&form, question)
                .map(|answer| Some(ask::answered(arguments.clone(), answer)))
        }
        (Prompting::Off, _) | (_, None) => Ok(None),
    }
}

async fn group(
    name: CommandName,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
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
#[expect(
    clippy::disallowed_methods,
    reason = "reads the user-named body file; its error is the CLI's BodyFile, not a storage one"
)]
pub(crate) fn read_source(source: &str, stdin: &mut impl Read) -> Result<String> {
    match source {
        "-" => {
            let mut text = String::new();
            stdin.read_to_string(&mut text).map_err(Error::ReadInput)?;
            Ok(text)
        }
        path => std::fs::read_to_string(path).map_err(|error| Error::BodyFile {
            path: path.into(),
            error,
        }),
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
