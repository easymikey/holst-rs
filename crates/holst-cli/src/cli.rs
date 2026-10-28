use clap::{
    CommandFactory as _,
    FromArgMatches as _,
    Parser,
    builder::styling::{AnsiColor, Styles},
};

use crate::{
    args::settings::SettingsArgs,
    error::Result,
    help,
    output::OutputFormat,
    run::RunSettings,
    table::{self, TableCall},
};

#[derive(Debug, Parser)]
#[command(
    name = "holst",
    version,
    subcommand_required = true,
    styles = STYLES
)]
pub(crate) struct Cli {
    /// Output format: text, json or pretty; text by default
    #[arg(
        long,
        short = 'o',
        global = true,
        value_name = "format",
        default_value = "text"
    )]
    pub(crate) output: OutputFormat,

    /// Print request logs on stderr
    #[arg(long, short = 'v', global = true)]
    pub(crate) verbose: bool,

    #[command(flatten)]
    pub(crate) settings: SettingsArgs,

    /// The table's command the command line chose, or why it is refused.
    #[arg(skip)]
    pub(crate) table: Option<Result<TableCall>>,
}

impl Cli {
    /// Parses the command line: the table's groups.
    ///
    /// # Errors
    /// Clap's usage error: help, version or a command line it refuses.
    pub(crate) fn parse_from_args<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
        let matches = command()
            .try_get_matches_from(&args)
            .or_else(|err| help_of_missing_subcommand(err, &args))?;
        Self::from_arg_matches(&matches).map(|cli| Self {
            table: table::call(&matches),
            ..cli
        })
    }
}

/// The help of the deepest command given, for a command line that stops short
/// of a subcommand. Clap's own `arg_required_else_help` does not fire when an
/// environment variable (`HOLST_HOME`) fills a flag.
fn help_of_missing_subcommand(
    err: clap::Error,
    args: &[std::ffi::OsString],
) -> Result<clap::ArgMatches, clap::Error> {
    if err.kind() == clap::error::ErrorKind::MissingSubcommand {
        command().try_get_matches_from(args.iter().cloned().chain(["--help".into()]))
    } else {
        Err(err)
    }
}

pub(crate) fn command() -> clap::Command {
    let root = Cli::command().subcommand(exit_codes_topic());
    table::groups(&root)
        .into_iter()
        .fold(root, clap::Command::subcommand)
        .override_help(help::root(&table::cli_groups(), &Cli::command()))
}

/// The help topic `holst help exit-codes`: clap's `help` shows the help of a
/// subcommand, so the topic is a hidden one whose help is the exit code block.
fn exit_codes_topic() -> clap::Command {
    clap::Command::new("exit-codes")
        .hide(true)
        .override_help(crate::exit::help_block())
}

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default())
    .usage(AnsiColor::Green.on_default())
    .literal(AnsiColor::Cyan.on_default())
    .placeholder(AnsiColor::Cyan.on_default())
    .error(AnsiColor::Red.on_default())
    .valid(AnsiColor::Green.on_default())
    .invalid(AnsiColor::Yellow.on_default());

pub(crate) async fn run(
    table: Option<Result<TableCall>>,
    format: OutputFormat,
    run: &RunSettings,
) -> Result<()> {
    match table {
        Some(call) => {
            let call = call?;
            crate::commands::run(call.name, call.arguments, run, format).await
        }
        // Only the hidden `exit-codes` topic parses without a table command.
        None => crate::output::line(crate::exit::help_block()),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use holst_core::command::{board::BoardVerb, table::CommandName};
    use serde_json::json;

    use crate::{
        cli::{Cli, Result, command},
        output::board::in_trash_note,
    };

    #[test]
    fn cli_definition_is_valid() {
        command().debug_assert();
    }

    #[test]
    fn hinted_commands_parse() -> Result<(), clap::Error> {
        holst_core::error::hint::COMMANDS
            .iter()
            .try_for_each(|line| Cli::parse_from_args(line.split(' ')).map(drop))
    }
    #[test]
    fn the_trash_caution_names_a_command_that_parses() -> Result<(), clap::Error> {
        let id = "cccccccc-dddd-4eee-8fff-000000000000";
        let caution = in_trash_note(id);
        let command = caution.split('`').nth(1).unwrap_or_default();
        let called = Cli::parse_from_args(command.split(' '))?
            .table
            .and_then(Result::ok);
        assert_eq!(
            called.map(|call| (call.name, call.arguments.get("board").cloned())),
            Some((CommandName::Board(BoardVerb::Restore), Some(json!(id))))
        );
        Ok(())
    }

    #[test]
    fn help_exit_codes_prints_the_exit_code_block() {
        let block = crate::exit::help_block();
        let shown = Cli::parse_from_args(["holst", "help", "exit-codes"])
            .err()
            .filter(|err| err.exit_code() == 0)
            .map(|err| err.to_string());
        assert!(
            shown.as_deref().is_some_and(|text| text.contains(&block)),
            "{block}\n\nnot in:\n{shown:?}"
        );
    }
}
