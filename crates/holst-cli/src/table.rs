//! The command table as clap commands, and clap's matches back as JSON.

use clap::{
    Arg,
    ArgAction,
    ArgMatches,
    Command as ClapCommand,
    builder::{PossibleValuesParser, ValueParser},
};
use holst_core::{
    command::table::{Argument, Command, CommandGroup, CommandName, ValueKind},
    error::hint::Surface,
};
use serde_json::{Map, Number, Value};

use crate::{
    error::{Error, Result},
    help::{self, kebab, long},
};

/// A command of the table with the arguments the command line gave it.
#[derive(Debug)]
pub(crate) struct TableCall {
    pub(crate) name: CommandName,
    pub(crate) arguments: Map<String, Value>,
}

/// Returns the clap commands of the table's groups, to add to `root`.
///
/// A group that is the command itself (`search`) comes as that command.
#[must_use]
pub(crate) fn groups(root: &ClapCommand) -> Vec<ClapCommand> {
    let inherited = inherited_flags(root);
    cli_groups()
        .into_iter()
        .map(|group| group_command(group, &inherited))
        .collect()
}

/// Returns the table's groups that have a command on the command line, each
/// once.
#[must_use]
pub(crate) fn cli_groups() -> Vec<CommandGroup> {
    help::first_seen(cli_commands().map(|command| command.group))
}

/// Returns the table's command that `matches` chose, with its arguments.
///
/// # Errors
/// [`Error::McpAlias`] when a reference argument is an MCP alias (`@3`).
#[must_use]
pub(crate) fn call(matches: &ArgMatches) -> Option<Result<TableCall>> {
    let (group, group_matches) = matches.subcommand()?;
    let (verb, verb_matches) = match group_matches.subcommand() {
        Some((verb, verb_matches)) => (Some(verb), verb_matches),
        None => (None, group_matches),
    };
    let name = CommandName::parse(group, verb)
        .filter(|name| name.command().surfaces.contains(&Surface::Cli))?;
    let command = name.command();
    let (kind, chosen) = match subcommand_argument(command) {
        Some(argument) => {
            let (word, kind_matches) = verb_matches.subcommand()?;
            let kind = kinds_of(argument)
                .iter()
                .copied()
                .find(|kind| kebab(kind) == word)?;
            (Some((argument.name, kind)), kind_matches)
        }
        None => (None, verb_matches),
    };
    Some(
        arguments(command, kind.map(|(_, kind)| kind), chosen).map(|own| TableCall {
            name,
            arguments: kind
                .map(|(field, kind)| (field.to_owned(), Value::String(kind.to_owned())))
                .into_iter()
                .chain(own)
                .collect(),
        }),
    )
}

/// Returns the argument whose values are subcommands of the command.
fn subcommand_argument(command: &Command) -> Option<&'static Argument> {
    command
        .arguments
        .iter()
        .find(|argument| argument.subcommand)
}

/// Returns the values of a subcommand argument.
fn kinds_of(argument: &Argument) -> &'static [&'static str] {
    match argument.kind {
        ValueKind::Choice(values) => values,
        ValueKind::Switch
        | ValueKind::Text
        | ValueKind::Integer
        | ValueKind::Number
        | ValueKind::NewFile
        | ValueKind::TextOrFile
        | ValueKind::Scale(_)
        | ValueKind::List(_)
        | ValueKind::OneOrMany(_)
        | ValueKind::Items
        | ValueKind::QueryParameter => &[],
    }
}

fn cli_commands() -> impl Iterator<Item = &'static Command> {
    CommandName::iter()
        .map(CommandName::command)
        .filter(|command| command.surfaces.contains(&Surface::Cli))
}

fn inherited_flags(root: &ClapCommand) -> Vec<String> {
    root.get_arguments()
        .filter(|argument| argument.is_global_set())
        .filter_map(Arg::get_long)
        .map(|long| format!("--{long}"))
        .chain([String::from("--help")])
        .collect()
}

fn group_command(group: CommandGroup, inherited: &[String]) -> ClapCommand {
    let help = group.help();
    let commands: Vec<&Command> = cli_commands()
        .filter(|command| command.group == group)
        .collect();
    match commands.as_slice() {
        [only] if only.verb.is_none() => command_of(only, help.name, inherited),
        _ => verbs_command(group, &commands, inherited),
    }
}

/// The clap command of a group with a verb for each command.
fn verbs_command(
    group: CommandGroup,
    commands: &[&Command],
    inherited: &[String],
) -> ClapCommand {
    let help = group.help();
    let base = ClapCommand::new(help.name)
        .about(help.about)
        .override_help(help::group(group, commands))
        .override_usage(format!("holst {} <command> [flags]", help.name))
        .subcommand_required(true);
    let aliased = match help.alias {
        Some(alias) => base.visible_alias(alias),
        None => base,
    };
    commands
        .iter()
        .filter_map(|command| verb_command(command, inherited))
        .fold(aliased, ClapCommand::subcommand)
}

fn verb_command(command: &Command, inherited: &[String]) -> Option<ClapCommand> {
    command
        .verb
        .map(|verb| command_of(command, verb, inherited))
}

/// The clap command of `command`, called `name`: its verb, or its group. One
/// with a subcommand argument has a command for each of its values.
fn command_of(
    command: &Command,
    name: &'static str,
    inherited: &[String],
) -> ClapCommand {
    subcommand_argument(command).map_or_else(
        || leaf_command(command, name.to_owned(), None, inherited),
        |argument| {
            let kinds = kinds_of(argument);
            kinds
                .iter()
                .map(|&kind| {
                    leaf_command(command, kebab(kind), Some(kind), inherited)
                        .about(help::kind_about(command, kind))
                })
                .fold(
                    ClapCommand::new(name)
                        .about(command.about)
                        .override_help(help::kinds(command, kinds, inherited))
                        .override_usage(help::usage(command, None))
                        .subcommand_required(true),
                    ClapCommand::subcommand,
                )
        },
    )
}

/// The clap command that takes the arguments of `command`, or of one `kind` of
/// it.
fn leaf_command(
    command: &Command,
    name: String,
    kind: Option<&'static str>,
    inherited: &[String],
) -> ClapCommand {
    help::arguments(command, kind)
        .into_iter()
        .map(flag)
        .fold(ClapCommand::new(name), ClapCommand::arg)
        .about(command.about)
        .override_help(help::command(command, kind, inherited))
        .override_usage(help::usage(command, kind))
}

fn flag(argument: &Argument) -> Arg {
    let base = Arg::new(argument.name)
        .help(argument.help)
        .value_name(argument.value_name)
        .required(argument.required);
    let lettered = match (argument.positional, argument.letter) {
        (true, _) => base,
        (false, Some(letter)) => base.long(long(argument.name)).short(letter),
        (false, None) => base.long(long(argument.name)),
    };
    match argument.kind {
        ValueKind::Switch => lettered.action(ArgAction::SetTrue),
        ValueKind::Text
        | ValueKind::NewFile
        | ValueKind::Integer
        | ValueKind::Number
        | ValueKind::TextOrFile
        | ValueKind::Items
        | ValueKind::Choice(_)
        | ValueKind::Scale(_) => lettered
            .action(ArgAction::Set)
            .value_parser(value_parser(argument.kind)),
        ValueKind::QueryParameter => lettered
            .action(ArgAction::Append)
            .value_parser(value_parser(argument.kind)),
        ValueKind::List(item) => lettered
            .action(ArgAction::Append)
            .value_delimiter(',')
            .value_parser(value_parser(*item)),
        // One or more values, however many the command line gives.
        ValueKind::OneOrMany(item) => lettered
            .action(ArgAction::Append)
            .num_args(1..)
            .value_parser(value_parser(*item)),
    }
}

// Recursion is bounded by the nesting of the argument's type.
fn value_parser(kind: ValueKind) -> ValueParser {
    match kind {
        ValueKind::Choice(values) | ValueKind::Scale(values) => {
            PossibleValuesParser::new(values.iter().map(|value| kebab(value))).into()
        }
        ValueKind::List(item) | ValueKind::OneOrMany(item) => value_parser(*item),
        ValueKind::Integer => clap::value_parser!(i64).into(),
        ValueKind::Number => clap::value_parser!(f64).into(),
        ValueKind::QueryParameter => ValueParser::new(key_value),
        ValueKind::Switch
        | ValueKind::Text
        | ValueKind::NewFile
        | ValueKind::TextOrFile
        | ValueKind::Items => ValueParser::string(),
    }
}

/// Checks that `text` is `KEY=VALUE` with a key.
fn key_value(text: &str) -> Result<String> {
    match text.split_once('=') {
        Some((key, _)) if !key.is_empty() => Ok(text.to_owned()),
        Some(_) | None => Err(Error::QueryNoEquals {
            item: text.to_owned(),
        }),
    }
}

/// Returns the value of `KEY=VALUE`: a number when it reads as one and
/// prints back as typed, else text.
fn pair_value(text: &str) -> Value {
    text.parse::<i64>()
        .ok()
        .filter(|number| number.to_string() == text)
        .map_or_else(|| Value::String(text.to_owned()), Value::from)
}

/// The arguments that name a thing, where an MCP alias (`@3`) is refused.
const REFERENCES: [&str; 8] = [
    "board",
    "workspace",
    "object",
    "user",
    "frame",
    "template",
    "from",
    "to",
];

/// Refuses a reference that is only an MCP alias; text is not touched.
fn refuse_alias(name: &str, value: &Value) -> Result<()> {
    match value {
        Value::String(text) if REFERENCES.contains(&name) => {
            if holst_core::domain::lookup::rules::is_alias(text) {
                Err(Error::McpAlias {
                    reference: text.clone(),
                })
            } else {
                Ok(())
            }
        }
        Value::Array(items) => {
            items.iter().try_for_each(|each| refuse_alias(name, each))
        }
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_) => Ok(()),
    }
}

/// Returns what the command line gave, as the object both surfaces parse.
fn arguments(
    command: &Command,
    kind: Option<&'static str>,
    matches: &ArgMatches,
) -> Result<Map<String, Value>> {
    help::arguments(command, kind)
        .into_iter()
        .filter_map(|argument| {
            given(argument.kind, argument.name, matches)
                .map(|value| (argument.name, value))
        })
        .map(|(name, value)| {
            refuse_alias(name, &value)?;
            Ok((name.to_owned(), value))
        })
        .collect()
}

fn given(kind: ValueKind, name: &str, matches: &ArgMatches) -> Option<Value> {
    match kind {
        ValueKind::Switch => matches
            .try_get_one::<bool>(name)
            .ok()
            .flatten()
            .copied()
            .filter(|given| *given)
            .map(Value::Bool),
        // The file of items: the command reads it before it parses.
        ValueKind::Text
        | ValueKind::NewFile
        | ValueKind::TextOrFile
        | ValueKind::Items
        | ValueKind::Choice(_)
        | ValueKind::Scale(_) => matches
            .try_get_one::<String>(name)
            .ok()
            .flatten()
            .map(|text| item(kind, text)),
        ValueKind::Integer => matches
            .try_get_one::<i64>(name)
            .ok()
            .flatten()
            .map(|number| Value::from(*number)),
        ValueKind::Number => matches
            .try_get_one::<f64>(name)
            .ok()
            .flatten()
            .copied()
            .and_then(Number::from_f64)
            .map(Value::Number),
        ValueKind::QueryParameter => matches
            .try_get_many::<String>(name)
            .ok()
            .flatten()
            .map(|pairs| {
                Value::Object(
                    pairs
                        .filter_map(|pair| pair.split_once('='))
                        .map(|(key, value)| (key.to_owned(), pair_value(value)))
                        .collect(),
                )
            }),
        ValueKind::List(each) | ValueKind::OneOrMany(each) => matches
            .try_get_many::<String>(name)
            .ok()
            .flatten()
            .map(|texts| Value::Array(texts.map(|text| item(*each, text)).collect())),
    }
}

/// Returns one typed value: a choice is typed in kebab case and sent in snake
/// case.
fn item(kind: ValueKind, text: &str) -> Value {
    Value::String(match kind {
        ValueKind::Choice(_) | ValueKind::Scale(_) => text.replace('-', "_"),
        ValueKind::Switch
        | ValueKind::Text
        | ValueKind::NewFile
        | ValueKind::Integer
        | ValueKind::Number
        | ValueKind::TextOrFile
        | ValueKind::QueryParameter
        | ValueKind::Items
        | ValueKind::List(_)
        | ValueKind::OneOrMany(_) => text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::{
            api::ApiArgs,
            auth::{AuthVerb, LoginArgs, LogoutArgs, StatusArgs},
            backup::{self, BackupVerb},
            board::{self, BoardVerb},
            comment::{self, CommentVerb},
            object::{self, ObjectVerb},
            search::SearchArgs,
            storage::{self, StorageVerb},
            table::{
                Argument,
                Command,
                CommandGroup,
                CommandName,
                CommandRisk,
                HelpBlock,
                ValueKind,
                parse,
            },
            template::{self, TemplateVerb},
            workspace::{
                CreateArgs,
                ListArgs,
                LogArgs,
                RenameArgs,
                ShowArgs,
                WorkspaceVerb,
            },
        },
        error::hint::Surface,
    };
    use rstest::rstest;
    use serde_json::json;

    use crate::{
        cli,
        error::Error,
        table::{
            TableCall,
            arguments,
            call,
            command_of,
            kinds_of,
            subcommand_argument,
        },
    };

    /// The words of `line`, a double-quoted one kept together.
    fn words(line: &str) -> Vec<String> {
        line.split('"')
            .enumerate()
            .flat_map(|(position, part)| {
                if position % 2 == 1 {
                    vec![part.to_owned()]
                } else {
                    part.split_whitespace().map(str::to_owned).collect()
                }
            })
            .collect()
    }

    fn matches(line: &str) -> Result<clap::ArgMatches, clap::Error> {
        cli::command().try_get_matches_from(words(line))
    }

    fn parsed(line: &str) -> Option<serde_json::Value> {
        let matches = matches(line).ok()?;
        call(&matches)?
            .ok()
            .map(|call| serde_json::Value::Object(call.arguments))
    }

    /// What `-h` prints: clap reports help as an error carrying the text.
    fn help_of(line: &str) -> String {
        matches(line)
            .err()
            .map(|help| help.to_string())
            .unwrap_or_default()
    }

    fn reads_object(
        verb: ObjectVerb,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> holst_core::Result<()> {
        match verb {
            ObjectVerb::Ls => parse::<object::LsArgs>(arguments).map(drop),
            ObjectVerb::Add => parse::<object::AddArgs>(arguments).map(drop),
            ObjectVerb::Edit => parse::<object::EditArgs>(arguments).map(drop),
            ObjectVerb::Cp => parse::<object::CopyArgs>(arguments).map(drop),
            ObjectVerb::Rm => parse::<object::RemoveArgs>(arguments).map(drop),
            ObjectVerb::Get => parse::<object::GetArgs>(arguments).map(drop),
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm per command of the table")]
    fn reads(mut call: TableCall) -> holst_core::Result<()> {
        // The file of items is read by the command, not by the table.
        call.arguments.remove("items");
        match call.name {
            CommandName::Auth(AuthVerb::Login) => {
                parse::<LoginArgs>(call.arguments).map(drop)
            }
            CommandName::Auth(AuthVerb::Logout) => {
                parse::<LogoutArgs>(call.arguments).map(drop)
            }
            CommandName::Auth(AuthVerb::Status) => {
                parse::<StatusArgs>(call.arguments).map(drop)
            }
            CommandName::Workspace(WorkspaceVerb::Ls) => {
                parse::<ListArgs>(call.arguments).map(drop)
            }
            CommandName::Workspace(WorkspaceVerb::Show) => {
                parse::<ShowArgs>(call.arguments).map(drop)
            }
            CommandName::Workspace(WorkspaceVerb::Create) => {
                parse::<CreateArgs>(call.arguments).map(drop)
            }
            CommandName::Workspace(WorkspaceVerb::Rename) => {
                parse::<RenameArgs>(call.arguments).map(drop)
            }
            CommandName::Workspace(WorkspaceVerb::Log) => {
                parse::<LogArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Ls) => {
                parse::<board::LsArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Create) => {
                parse::<board::CreateArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Show) => {
                parse::<board::ShowArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Snap) => {
                parse::<board::SnapArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Save) => {
                parse::<board::SaveArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Export) => {
                parse::<board::ExportArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Import) => {
                parse::<board::ImportArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Edit) => {
                parse::<board::EditArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Rename) => {
                parse::<board::RenameArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Mv) => {
                parse::<board::MoveArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Cp) => {
                parse::<board::CopyArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Rm) => {
                parse::<board::RemoveArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Restore) => {
                parse::<board::RestoreArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Favorite) => {
                parse::<board::FavoriteArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Unfavorite) => {
                parse::<board::UnfavoriteArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Touch) => {
                parse::<board::TouchArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Leave) => {
                parse::<board::LeaveArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Grant) => {
                parse::<board::GrantArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Revoke) => {
                parse::<board::RevokeArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Share) => {
                parse::<board::ShareArgs>(call.arguments).map(drop)
            }
            CommandName::Board(BoardVerb::Transfer) => {
                parse::<board::TransferArgs>(call.arguments).map(drop)
            }
            CommandName::Comment(verb) => comment_reads(verb, call.arguments),
            CommandName::Template(TemplateVerb::Ls) => {
                parse::<template::LsArgs>(call.arguments).map(drop)
            }
            CommandName::Template(TemplateVerb::Show) => {
                parse::<template::ShowArgs>(call.arguments).map(drop)
            }
            CommandName::Template(TemplateVerb::Create) => {
                parse::<template::CreateArgs>(call.arguments).map(drop)
            }
            CommandName::Template(TemplateVerb::Edit) => {
                parse::<template::EditArgs>(call.arguments).map(drop)
            }
            CommandName::Template(TemplateVerb::Rm) => {
                parse::<template::RemoveArgs>(call.arguments).map(drop)
            }
            CommandName::Backup(BackupVerb::Ls) => {
                parse::<backup::LsArgs>(call.arguments).map(drop)
            }
            CommandName::Backup(BackupVerb::Get) => {
                parse::<backup::GetArgs>(call.arguments).map(drop)
            }
            CommandName::Backup(BackupVerb::Restore) => {
                parse::<backup::RestoreArgs>(call.arguments).map(drop)
            }
            CommandName::Storage(StorageVerb::Ls) => {
                parse::<storage::LsArgs>(call.arguments).map(drop)
            }
            CommandName::Storage(StorageVerb::Clean) => {
                parse::<storage::CleanArgs>(call.arguments).map(drop)
            }
            CommandName::Object(verb) => reads_object(verb, call.arguments),
            CommandName::Search => parse::<SearchArgs>(call.arguments).map(drop),
            CommandName::Api => parse::<ApiArgs>(call.arguments).map(drop),
        }
    }

    /// The comment arms of [`reads`], apart to keep each function short.
    fn comment_reads(
        verb: CommentVerb,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> holst_core::Result<()> {
        match verb {
            CommentVerb::Ls => parse::<comment::LsArgs>(arguments).map(drop),
            CommentVerb::Add => parse::<comment::AddArgs>(arguments).map(drop),
            CommentVerb::Reply => parse::<comment::ReplyArgs>(arguments).map(drop),
            CommentVerb::Resolve => parse::<comment::ResolveArgs>(arguments).map(drop),
            CommentVerb::Rm => parse::<comment::RemoveArgs>(arguments).map(drop),
            CommentVerb::Read => parse::<comment::ReadArgs>(arguments).map(drop),
        }
    }

    #[test]
    fn arguments_come_back_as_the_json_both_surfaces_parse() {
        assert_eq!(parsed("holst auth status"), Some(json!({})));
        assert_eq!(
            parsed("holst auth status --with tokens"),
            Some(json!({ "with": ["tokens"] }))
        );
        assert_eq!(
            parsed("holst auth login --email a@b.c --no-save"),
            Some(json!({ "email": "a@b.c", "no_save": true }))
        );
        assert_eq!(
            parsed("holst auth logout --local-only"),
            Some(json!({ "local_only": true }))
        );
    }

    #[test]
    fn a_group_that_is_the_command_takes_its_arguments_without_a_verb() {
        assert_eq!(
            parsed(
                "holst search \"road map\" --mode board-text -w easy -k sticker -l 5"
            ),
            Some(json!({
                "query": "road map",
                "mode": "board_text",
                "workspace": "easy",
                "kind": "sticker",
                "limit": 5,
            }))
        );
        assert_eq!(
            parsed("holst search retro --trash"),
            Some(json!({ "query": "retro", "trash": true }))
        );
        assert_eq!(parsed("holst search"), None);
        assert_eq!(parsed("holst search retro --mode all"), None);
    }

    #[test]
    fn comment_arguments_come_back_by_name() {
        assert_eq!(
            parsed("holst comment add Roadmap \"Is it final?\" --at 120,80 -n"),
            Some(json!({
                "board": "Roadmap",
                "text": "Is it final?",
                "at": "120,80",
                "dry_run": true,
            }))
        );
        assert_eq!(
            parsed("holst comment rm Roadmap weVza -y"),
            Some(json!({ "board": "Roadmap", "thread": "weVza", "confirm": true }))
        );
        assert_eq!(
            parsed("holst comment read Roadmap weVza --time 2026-10-27T09:30:00Z"),
            Some(json!({
                "board": "Roadmap",
                "thread": "weVza",
                "time": "2026-10-27T09:30:00Z",
            }))
        );
    }

    /// The alias that `call` refuses, if it does.
    fn refused(line: &str) -> Option<String> {
        match call(&matches(line).ok()?)? {
            Err(Error::McpAlias { reference }) => Some(reference),
            Ok(_) | Err(_) => None,
        }
    }

    #[rstest]
    #[case::reference("@3", true)]
    #[case::padded_reference(" @12 ", true)]
    #[case::name_with_at("@team plans", false)]
    #[case::bare_at("@", false)]
    #[case::id("ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b", false)]
    #[case::id_prefix_as_listed("@dbc79d73", false)]
    fn only_at_and_a_short_number_is_an_alias(#[case] text: &str, #[case] alias: bool) {
        assert_eq!(holst_core::domain::lookup::rules::is_alias(text), alias);
    }

    #[rstest]
    #[case::board("holst b restore @3", "@3")]
    #[case::workspace("holst b ls -w @2", "@2")]
    #[case::object("holst o edit Plans --object @2", "@2")]
    #[case::second_object("holst o rm Plans --object a1 --object @4 -y", "@4")]
    #[case::from("holst o add arrow Plans --from @1 --to #2", "@1")]
    fn a_reference_that_is_an_mcp_alias_is_refused(
        #[case] line: &str,
        #[case] reference: &str,
    ) {
        assert_eq!(refused(line).as_deref(), Some(reference));
    }

    #[test]
    fn text_that_starts_like_an_alias_is_kept() {
        assert_eq!(refused("holst o add sticker Plans \"@3 apples\""), None);
        assert_eq!(
            parsed("holst o add sticker Plans \"@3 apples\""),
            Some(json!({ "kind": "sticker", "board": "Plans", "text": "@3 apples" }))
        );
        assert!(parsed("holst b restore @three").is_some());
    }

    #[test]
    fn a_thread_handle_is_not_an_mcp_alias() {
        assert_eq!(refused("holst comment rm Roadmap @1 -y"), None);
        assert_eq!(
            parsed("holst comment rm Roadmap @1 -y"),
            Some(json!({ "board": "Roadmap", "thread": "@1", "confirm": true }))
        );
    }

    #[test]
    fn an_unknown_choice_is_a_usage_error() {
        assert_eq!(parsed("holst auth status --with sessions"), None);
    }

    #[rstest]
    #[case::grant("holst b grant Roadmap anna --right owner")]
    #[case::link("holst b share Roadmap --link owner")]
    #[case::members("holst b share Roadmap --members owner")]
    fn an_unknown_right_is_a_usage_error_listing_the_scale(#[case] line: &str) {
        let error = matches(line).err();
        assert_eq!(
            error.as_ref().map(clap::Error::kind),
            Some(clap::error::ErrorKind::InvalidValue)
        );
        assert!(error.is_some_and(|error| {
            error
                .to_string()
                .contains("none, viewer, commentator, editor, full")
        }));
    }

    #[test]
    fn a_right_is_typed_as_a_word_and_sent_as_text() {
        assert_eq!(
            parsed("holst b grant Roadmap anna --right editor"),
            Some(json!({ "board": "Roadmap", "user": "anna", "right": "editor" }))
        );
        assert_eq!(
            parsed("holst b share Roadmap --link none"),
            Some(json!({ "board": "Roadmap", "link": "none" }))
        );
    }

    #[test]
    fn every_example_parses_and_its_arguments_are_read() {
        CommandName::iter()
            .map(CommandName::command)
            .filter(|command| command.surfaces.contains(&Surface::Cli))
            .flat_map(|command| {
                command
                    .examples
                    .iter()
                    .chain(command.group.help().examples)
                    .copied()
            })
            .for_each(|example| {
                let chosen = matches(example)
                    .ok()
                    .and_then(|matches| call(&matches))
                    .and_then(Result::ok);
                assert!(chosen.is_some(), "{example}: does not parse");
                assert!(
                    chosen.map(reads).is_some_and(|read| read.is_ok()),
                    "{example}: its arguments are refused"
                );
            });
    }

    #[test]
    fn every_help_page_matches_its_snapshot() {
        let pages = CommandName::iter()
            .map(CommandName::command)
            .filter(|command| command.surfaces.contains(&Surface::Cli))
            .flat_map(|command| {
                let group = format!("{:?}", command.group).to_lowercase();
                let (page, line) = command.verb.map_or_else(
                    || (group.clone(), format!("holst {group}")),
                    |verb| (format!("{group}_{verb}"), format!("holst {group} {verb}")),
                );
                let kinds = subcommand_argument(command)
                    .map(kinds_of)
                    .unwrap_or_default();
                [
                    (group.clone(), format!("holst {group}")),
                    (page.clone(), line.clone()),
                ]
                .into_iter()
                .chain(kinds.iter().map(move |kind| {
                    (format!("{page}_{kind}"), format!("{line} {kind}"))
                }))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let failed = pages
            .iter()
            .filter(|(name, line)| {
                std::panic::catch_unwind(|| {
                    insta::assert_snapshot!(
                        name.as_str(),
                        help_of(&format!("{line} -h"))
                    );
                })
                .is_err()
            })
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        assert!(failed.is_empty(), "help snapshots differ: {failed:?}");
    }

    #[test]
    fn a_board_edit_takes_no_note() {
        assert!(matches("holst b edit Plan --note a").is_err());
    }

    #[test]
    fn a_template_command_takes_its_arguments_as_json() {
        assert_eq!(
            parsed("holst tpl create Roadmap --name Q --for workspace --object a,b"),
            Some(json!({
                "board": "Roadmap",
                "name": "Q",
                "for": "workspace",
                "object": ["a", "b"],
            }))
        );
        assert_eq!(
            parsed("holst tpl edit t1 --body @body.json -y"),
            Some(json!({ "template": "t1", "body": "@body.json", "confirm": true }))
        );
    }

    #[test]
    fn board_arguments_come_back_as_the_json_the_core_parses() {
        assert_eq!(
            parsed("holst b rename \"Old plans\" Plans -w easy -n"),
            Some(
                json!({ "board": "Old plans", "name": "Plans", "workspace": "easy", "dry_run": true })
            )
        );
        assert_eq!(
            parsed("holst b mv Plans / -y"),
            Some(json!({ "board": "Plans", "folder": "/", "confirm": true }))
        );
        assert_eq!(
            parsed("holst b ls --view trash -l 5"),
            Some(json!({ "view": "trash", "limit": 5 }))
        );
        assert_eq!(parsed("holst b mv Plans"), None);
        assert_eq!(parsed("holst b ls --view sticker"), None);
    }

    #[test]
    fn an_object_kind_is_a_subcommand_that_comes_back_as_the_kind_argument() {
        assert_eq!(
            parsed("holst o add sticker \"Road map\" \"Ask design\" -c yellow4 -n"),
            Some(json!({
                "kind": "sticker",
                "board": "Road map",
                "text": "Ask design",
                "color": "yellow4",
                "dry_run": true,
            }))
        );
        assert_eq!(
            parsed("holst o add shape Plans --shape rounded-rectangle --items q.json"),
            Some(json!({
                "kind": "shape",
                "board": "Plans",
                "shape": "rounded_rectangle",
                "items": "q.json",
            }))
        );
        assert_eq!(
            parsed("holst o add arrow Plans --from a1 --to #2"),
            Some(
                json!({ "kind": "arrow", "board": "Plans", "from": "a1", "to": "#2" })
            )
        );
    }

    #[test]
    fn a_flag_is_only_on_the_kinds_it_is_for() {
        assert!(parsed("holst o add shape Plans --shape ellipse").is_some());
        assert_eq!(parsed("holst o add sticker Plans --shape ellipse"), None);
        assert_eq!(parsed("holst o add arrow Plans --size 10x10"), None);
        assert_eq!(parsed("holst o add text Plans hi --from a1"), None);
    }

    #[test]
    fn an_object_kind_is_needed_and_must_be_known() {
        assert_eq!(parsed("holst o add Plans hi"), None);
        assert_eq!(parsed("holst o add"), None);
        assert_eq!(parsed("holst o add photo Plans"), None);
    }

    #[test]
    fn a_link_an_image_and_a_file_take_their_source_as_an_argument() {
        assert_eq!(
            parsed("holst o add link Plans https://example.com"),
            Some(json!({
                "kind": "link", "board": "Plans", "url": "https://example.com"
            }))
        );
        assert_eq!(
            parsed("holst o add image Plans mock.png"),
            Some(json!({ "kind": "image", "board": "Plans", "path": "mock.png" }))
        );
        assert_eq!(
            parsed("holst o add file Plans a.pdf"),
            Some(json!({ "kind": "file", "board": "Plans", "path": "a.pdf" }))
        );
        assert_eq!(parsed("holst o add file Plans a.pdf --size 10x10"), None);
        assert_eq!(parsed("holst o add link Plans https://a.b -c red"), None);
    }

    #[test]
    fn object_arguments_come_back_as_the_json_the_core_parses() {
        assert_eq!(
            parsed("holst o get Plans out.png --object a5 --refresh"),
            Some(json!({
                "board": "Plans",
                "file": "out.png",
                "object": ["a5"],
                "refresh": true,
            }))
        );
        assert_eq!(
            parsed("holst o rm Plans --object a3 --object a4 -y"),
            Some(json!({ "board": "Plans", "object": ["a3", "a4"], "confirm": true }))
        );
        assert_eq!(
            parsed("holst o edit Plans --tag a --tag b --order front"),
            Some(json!({ "board": "Plans", "tag": ["a", "b"], "order": "front" }))
        );
    }

    #[test]
    fn repeated_pairs_come_back_as_one_object_with_numbers_kept() {
        assert_eq!(
            parsed("holst ws log -q limit=50 -q after=007 -q kind=a=b"),
            Some(json!({ "query": { "limit": 50, "after": "007", "kind": "a=b" } }))
        );
        assert_eq!(parsed("holst ws log -q limit"), None);
    }

    const fn argument(
        name: &'static str,
        kind: ValueKind,
        required: bool,
        surfaces: &'static [Surface],
    ) -> Argument {
        Argument {
            name,
            letter: None,
            kind,
            required,
            positional: false,
            value_name: "",
            help: "",
            tool_doc: None,
            surfaces,
            subcommand: false,
            kinds: &[],
        }
    }

    const PROBE: Command = Command {
        group: CommandGroup::Board,
        verb: Some("probe"),
        about: "",
        help: "",
        examples: &[],
        block: HelpBlock::Look,
        risk: CommandRisk::Read,
        surfaces: &[Surface::Cli, Surface::Mcp],
        arguments: &[
            argument(
                "boards",
                ValueKind::OneOrMany(&ValueKind::Text),
                true,
                &[Surface::Cli, Surface::Mcp],
            ),
            argument(
                "scale",
                ValueKind::Number,
                false,
                &[Surface::Cli, Surface::Mcp],
            ),
            argument("file", ValueKind::NewFile, false, &[Surface::Cli]),
            argument(
                "path",
                ValueKind::NewFile,
                false,
                &[Surface::Cli, Surface::Mcp],
            ),
            argument("on_mcp", ValueKind::Text, false, &[Surface::Mcp]),
        ],
    };

    fn probed(line: &str) -> Option<serde_json::Value> {
        let matches = command_of(&PROBE, "probe", &[])
            .try_get_matches_from(words(line))
            .ok()?;
        arguments(&PROBE, None, &matches)
            .ok()
            .map(serde_json::Value::Object)
    }

    #[test]
    fn one_or_many_numbers_and_new_files_are_read_from_the_command_line() {
        assert_eq!(
            probed("probe --boards Plans Roadmap --scale 1.5 --path out.png"),
            Some(
                json!({ "boards": ["Plans", "Roadmap"], "scale": 1.5, "path": "out.png" })
            )
        );
        assert_eq!(
            probed("probe --boards Plans --boards Roadmap"),
            Some(json!({ "boards": ["Plans", "Roadmap"] }))
        );
        assert_eq!(probed("probe --boards Plans --scale wide"), None);
        assert_eq!(probed("probe"), None);
    }

    #[test]
    fn an_argument_is_a_flag_only_on_the_surfaces_it_names() {
        assert_eq!(
            probed("probe --boards Plans --file out.png"),
            Some(json!({ "boards": ["Plans"], "file": "out.png" }))
        );
        assert_eq!(probed("probe --boards Plans --on-mcp x"), None);
    }

    // The gh rules over the built command line.
    mod rules {
        use std::collections::BTreeSet;

        use clap::{Arg, builder::PossibleValue};
        use holst_core::{
            command::table::{Command, CommandName},
            error::hint::Surface,
        };

        use super::{cli, help_of};
        use crate::{
            help::{self, kebab},
            table::kinds_of,
        };

        /// The sections of a command page in their order.
        const LEAF_ORDER: &[&[&str]] = &[
            &["USAGE"],
            &["ARGUMENTS"],
            &["FLAGS"],
            &["INHERITED FLAGS"],
            &["EXAMPLES"],
            &["LEARN MORE"],
        ];
        /// The sections of the page of a command with a subcommand (`o add`).
        const KINDS_ORDER: &[&[&str]] = &[
            &["USAGE"],
            &["KINDS"],
            &["INHERITED FLAGS"],
            &["EXAMPLES"],
            &["LEARN MORE"],
        ];
        /// The sections of a group page: its blocks, then the shared parts.
        const GROUP_ORDER: &[&[&str]] = &[
            &["USAGE"],
            &["COMMANDS", "GENERAL COMMANDS"],
            &["TARGETED COMMANDS"],
            &["LOOK"],
            &["CHANGE"],
            &["YOURS"],
            &["RIGHTS AND SHARING"],
            &["FLAGS"],
            &["ARGUMENTS"],
            &["EXAMPLES"],
        ];

        /// The letters that mean one flag everywhere.
        const LETTERS: [(char, &str); 3] =
            [('o', "output"), ('w', "workspace"), ('q', "query")];

        fn none(rule: &str, offenders: &[String]) {
            assert!(
                offenders.is_empty(),
                "{rule}: {} offenders\n  {}",
                offenders.len(),
                offenders.join("\n  ")
            );
        }

        /// Every command of the tree below `parent` with its path, groups
        /// first.
        fn nodes<'a>(
            parent: &'a clap::Command,
            path: &str,
        ) -> Vec<(String, &'a clap::Command)> {
            parent
                .get_subcommands()
                .filter(|child| child.get_name() != "help" && !child.is_hide_set())
                .flat_map(|child| {
                    let path = format!("{path} {}", child.get_name())
                        .trim_start()
                        .to_owned();
                    let below = nodes(child, &path);
                    std::iter::once((path, child))
                        .chain(below)
                        .collect::<Vec<_>>()
                })
                .collect()
        }

        fn built() -> clap::Command {
            // imperative: `build` needs `&mut` to hand the global flags down.
            let mut root = cli::command();
            root.build();
            root
        }

        fn is_leaf(command: &clap::Command) -> bool {
            command
                .get_subcommands()
                .all(|child| child.get_name() == "help")
        }

        /// The table command a path of the tree names, and the kind of it.
        fn table_command(
            path: &str,
        ) -> Option<(&'static Command, Option<&'static str>)> {
            let mut words = path.split(' ');
            let group = words.next()?;
            let (verb, kind) = (words.next(), words.next());
            let command = CommandName::parse(group, verb)
                .or_else(|| CommandName::parse(group, None))?
                .command();
            let chosen = kind.and_then(|word| {
                command
                    .arguments
                    .iter()
                    .find(|argument| argument.subcommand)
                    .and_then(|argument| {
                        kinds_of(argument)
                            .iter()
                            .copied()
                            .find(|k| kebab(k) == word)
                    })
            });
            Some((command, chosen))
        }

        /// The paths of the leaves the table gives the command line.
        fn table_leaves() -> BTreeSet<String> {
            CommandName::iter()
                .map(CommandName::command)
                .filter(|command| command.surfaces.contains(&Surface::Cli))
                .flat_map(|command| {
                    let path = command.path();
                    command
                        .arguments
                        .iter()
                        .find(|argument| argument.subcommand)
                        .map_or_else(
                            || vec![path.clone()],
                            |argument| {
                                kinds_of(argument)
                                    .iter()
                                    .map(|kind| format!("{path} {}", kebab(kind)))
                                    .collect()
                            },
                        )
                })
                .collect()
        }

        #[test]
        fn the_command_line_has_the_commands_of_the_table() {
            let root = built();
            let all = nodes(&root, "");
            let leaves: BTreeSet<String> = all
                .iter()
                .filter(|(_, command)| is_leaf(command))
                .map(|(path, _)| path.clone())
                .collect();
            let wanted = table_leaves();
            let missing = wanted
                .iter()
                .filter(|path| !leaves.contains(*path))
                .map(|path| format!("{path}: in the table, not on the command line"));
            let unlisted = leaves
                .iter()
                .filter(|path| !wanted.contains(*path))
                .map(|path| format!("{path}: not in the table"));
            let offenders: Vec<String> = missing.chain(unlisted).collect();
            none("a command", &offenders);
        }

        fn is_title(line: &str) -> bool {
            !line.is_empty()
                && line
                    .chars()
                    .all(|each| each.is_ascii_uppercase() || each == ' ')
        }

        fn order_flaws(path: &str, order: &[&[&str]]) -> Vec<String> {
            let text = help_of(&format!("holst {path} -h"));
            let titles: Vec<&str> =
                text.lines().filter(|line| is_title(line)).collect();
            let ranks: Vec<usize> = titles
                .iter()
                .filter_map(|title| order.iter().position(|each| each.contains(title)))
                .collect();
            let unknown = titles
                .iter()
                .filter(|title| !order.iter().any(|each| each.contains(title)))
                .map(|title| format!("{path}: section {title} is not in the order"));
            let misplaced = ranks
                .iter()
                .zip(ranks.iter().skip(1))
                .any(|(before, after)| before >= after)
                .then(|| format!("{path}: sections out of order: {titles:?}"));
            let first = (titles.first() != Some(&"USAGE"))
                .then(|| format!("{path}: does not start with USAGE: {titles:?}"));
            unknown.chain(misplaced).chain(first).collect()
        }

        fn order_of(leaf: bool, depth: usize) -> &'static [&'static [&'static str]] {
            match (leaf, depth) {
                (true, _) => LEAF_ORDER,
                (false, 1) => GROUP_ORDER,
                (false, _) => KINDS_ORDER,
            }
        }

        #[test]
        fn help_sections_come_in_the_order_of_the_preview() {
            let root = built();
            let offenders: Vec<String> = nodes(&root, "")
                .iter()
                .flat_map(|(path, command)| {
                    let order = order_of(is_leaf(command), path.split(' ').count());
                    order_flaws(path, order)
                })
                .collect();
            none("help sections", &offenders);
        }

        fn flags_of(command: &clap::Command) -> impl Iterator<Item = &Arg> {
            command
                .get_arguments()
                .filter(|arg| arg.get_id() != "help" && arg.get_id() != "version")
        }

        #[test]
        fn yes_and_dry_run_are_the_same_flag_everywhere() {
            let root = built();
            let offenders: Vec<String> = nodes(&root, "")
                .iter()
                .filter(|(_, command)| is_leaf(command))
                .flat_map(|(path, command)| {
                    let table = table_command(path);
                    let takes = move |name: &str| {
                        table.is_some_and(|(each, kind)| {
                            help::arguments(each, kind)
                                .iter()
                                .any(|argument| argument.name == name)
                        })
                    };
                    let flaws: Vec<String> = [('y', "yes", "confirm"), ('n', "dry-run", "dry_run")]
                        .into_iter()
                        .filter(|(letter, long, name)| {
                            let flag = flags_of(command).find(|arg| {
                                arg.get_long() == Some(*long)
                                    || arg.get_short() == Some(*letter)
                            });
                            let exact = flag.is_none_or(|arg| {
                                arg.get_long() == Some(*long)
                                    && arg.get_short() == Some(*letter)
                            });
                            !(exact && flag.is_some() == takes(name))
                        })
                        .map(|(letter, long, name)| {
                            format!("{path}: -{letter} --{long} is not exactly on the commands taking `{name}`")
                        })
                        .collect();
                    flaws
                })
                .collect();
            none("-y/--yes, -n/--dry-run", &offenders);
        }

        fn signature(arg: &Arg) -> String {
            format!(
                "{:?} {:?} {:?}",
                arg.get_action(),
                arg.get_num_args(),
                arg.get_possible_values()
                    .iter()
                    .map(PossibleValue::get_name)
                    .collect::<Vec<_>>()
            )
        }

        #[test]
        fn o_w_and_q_are_the_same_flag_everywhere() {
            let root = built();
            let all = nodes(&root, "");
            let leaves: Vec<_> =
                all.iter().filter(|(_, command)| is_leaf(command)).collect();
            let wrong = leaves.iter().flat_map(|(path, command)| {
                flags_of(command).flat_map(move |arg| {
                    LETTERS.iter().filter_map(move |(letter, long)| {
                        let short = arg.get_short() == Some(*letter);
                        let named = arg.get_long() == Some(*long);
                        (short != named).then(|| {
                            format!("{path}: -{letter} is --{long} on every command, but here {:?} / {:?}", arg.get_short(), arg.get_long())
                        })
                    })
                })
            });
            let differing = LETTERS.iter().filter_map(|(letter, long)| {
                let kinds: BTreeSet<String> = leaves
                    .iter()
                    .flat_map(|(_, command)| flags_of(command))
                    .filter(|arg| arg.get_short() == Some(*letter))
                    .map(signature)
                    .collect();
                (kinds.len() > 1).then(|| {
                    format!("-{letter} --{long} takes different values: {kinds:?}")
                })
            });
            let offenders: Vec<String> = wrong.chain(differing).collect();
            none("-o, -w, -q", &offenders);
        }

        /// The MCP name of a long flag: its dashes are underscores, and `--yes`
        /// is `confirm`.
        fn mcp_name(long: &str) -> String {
            match long {
                "yes" => String::from("confirm"),
                other => other.replace('-', "_"),
            }
        }

        #[test]
        fn a_long_flag_is_the_mcp_name_with_dashes() {
            let root = built();
            let offenders: Vec<String> = nodes(&root, "")
                .iter()
                .filter(|(_, command)| is_leaf(command))
                .flat_map(|(path, command)| {
                    let own: Vec<&Arg> = flags_of(command)
                        .filter(|arg| !arg.is_global_set())
                        .collect();
                    let renamed = own.iter().filter_map(|arg| {
                        let name = arg.get_long().map(mcp_name)?;
                        (name != arg.get_id().as_str()).then(|| {
                            format!(
                                "{path}: --{} is the MCP name `{}`, not `{name}`",
                                arg.get_long().unwrap_or_default(),
                                arg.get_id()
                            )
                        })
                    });
                    renamed.collect::<Vec<_>>()
                })
                .collect();
            none("MCP names", &offenders);
        }
    }
}
