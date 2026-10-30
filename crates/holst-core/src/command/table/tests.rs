#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use std::collections::{BTreeMap, BTreeSet};

use holst_macros::CommandArgs;
use itertools::Itertools as _;
use rstest::rstest;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    command::{
        auth::{LogoutMode, StatusArgs, StatusPart},
        storage::CleanArgs,
        table::{
            Argument,
            Changes as _,
            CommandArgs,
            CommandGroup,
            CommandName,
            CommandRisk,
            EnumArgument,
            GroupTools,
            Items,
            ValueKind,
            parse,
        },
    },
    effect::board_write::WriteMode,
    error::hint::Surface,
    fake::assert_none,
};

const GLOBAL_NAMES: [&str; 5] = ["output", "verbose", "base_url", "home", "no_cache"];
const GLOBAL_LETTERS: [char; 4] = ['o', 'v', 'h', 'V'];

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Auth,
    verb = "probe",
    risk = Read,
    about = "Probe the parse",
    help = "Takes one of each kind."
)]
struct ProbeArgs {
    #[argument(help = "A name")]
    name: String,
    #[argument(help = "Parts")]
    with: Vec<StatusPart>,
    #[argument(help = "Only here")]
    local_only: LogoutMode,
}

#[derive(Debug, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
enum Pick {
    Sticker,
    Shape,
}

impl EnumArgument for Pick {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "probe",
    risk = Read,
    about = "Probe the kinds",
    help = "Takes a kind, an argument of one kind and items."
)]
struct KindProbe {
    #[argument(help = "The kind", subcommand)]
    kind: Option<Pick>,
    #[argument(help = "Only a shape", kinds = [shape])]
    outline: Option<String>,
    #[argument(help = "Many in one")]
    #[serde(default)]
    items: Items<Value>,
}

fn arguments(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// The text of one problem, or of several as `Error::several` lists them.
fn listed(problems: &[&str]) -> String {
    match problems {
        [only] => (*only).to_owned(),
        several => format!(
            "{} problems:{}",
            several.len(),
            several
                .iter()
                .flat_map(|problem| ["\n  - ", problem])
                .collect::<String>()
        ),
    }
}

fn every_argument() -> impl Iterator<Item = (CommandGroup, &'static Argument)> {
    CommandName::iter().flat_map(|name| {
        let command = name.command();
        command
            .arguments
            .iter()
            .map(|argument| (command.group, argument))
    })
}

#[test]
fn every_command_is_found_by_its_names() {
    let found: Vec<_> = CommandName::iter()
        .map(|name| {
            let command = name.command();
            CommandName::parse(command.group.help().name, command.verb)
        })
        .collect();
    let wanted: Vec<_> = CommandName::iter().map(Some).collect();
    assert_eq!(found, wanted);
}

#[test]
fn a_letter_means_one_argument_everywhere() {
    let meanings: BTreeMap<char, BTreeSet<&str>> = every_argument()
        .filter_map(|(_, argument)| {
            argument.letter.map(|letter| (letter, argument.name))
        })
        .into_group_map()
        .into_iter()
        .map(|(letter, names)| (letter, names.into_iter().collect()))
        .collect();
    let doubled: Vec<_> = meanings
        .iter()
        .filter(|(_, names)| names.len() > 1)
        .collect();
    assert!(doubled.is_empty(), "{doubled:?}");
}

#[test]
fn a_name_has_one_kind_and_one_tool_doc_in_a_tool() {
    // A group with a read tool and a write tool has two namespaces.
    let shapes: BTreeMap<(&str, bool, &str), BTreeSet<String>> = CommandName::iter()
        // A command on no surface yet has no tool to share a schema with.
        .filter(|name| !name.command().surfaces.is_empty())
        .flat_map(|name| {
            let command = name.command();
            let writes = command.risk != CommandRisk::Read
                && matches!(
                    command.group.help().tools,
                    GroupTools::ReadAndWrite { .. }
                );
            command
                .arguments
                .iter()
                .map(move |argument| (command.group, writes, argument))
        })
        .filter(|(_, _, argument)| argument.surfaces.contains(&Surface::Mcp))
        .map(|(group, writes, argument)| {
            (
                (group.help().name, writes, argument.name),
                format!("{:?} {:?}", argument.kind.one(), argument.tool_doc),
            )
        })
        .into_group_map()
        .into_iter()
        .map(|(key, shapes)| (key, shapes.into_iter().collect()))
        .collect();
    let differing: Vec<_> = shapes
        .iter()
        .filter(|(_, shapes)| shapes.len() > 1)
        .collect();
    assert!(differing.is_empty(), "{differing:?}");
}

#[test]
fn no_argument_takes_a_global_name_or_letter() {
    let clashes: Vec<_> = every_argument()
        .filter(|(_, argument)| {
            GLOBAL_NAMES.contains(&argument.name)
                || argument
                    .letter
                    .is_some_and(|letter| GLOBAL_LETTERS.contains(&letter))
        })
        .map(|(_, argument)| argument.name)
        .collect();
    assert!(clashes.is_empty(), "{clashes:?}");
}

#[test]
fn the_derive_reads_names_kinds_and_texts_from_the_fields() {
    let shapes: Vec<_> = ProbeArgs::COMMAND
        .arguments
        .iter()
        .map(|argument| (argument.name, argument.kind, argument.required))
        .collect();
    assert_eq!(
        shapes,
        [
            ("name", ValueKind::Text, true),
            (
                "with",
                ValueKind::List(&ValueKind::Choice(&["tokens"])),
                false
            ),
            ("local_only", ValueKind::Switch, false),
        ]
    );
}

#[test]
fn left_out_switches_and_lists_are_filled() -> crate::Result<()> {
    let probe: ProbeArgs = parse(arguments(&json!({ "name": "x", "with": null })))?;
    assert_eq!(
        (probe.name.as_str(), probe.with, probe.local_only),
        ("x", Vec::new(), LogoutMode::Everywhere)
    );
    let status: StatusArgs = parse(Map::new())?;
    assert_eq!(status.with, Vec::<StatusPart>::new());
    Ok(())
}

#[test]
fn a_dry_run_field_gives_the_mode() -> crate::Result<()> {
    let modes = [json!({}), json!({ "dry_run": true })]
        .map(|given| parse::<CleanArgs>(arguments(&given)).map(|clean| clean.mode()));
    assert_eq!(
        modes.into_iter().collect::<crate::Result<Vec<_>>>()?,
        [WriteMode::Send, WriteMode::DryRun]
    );
    Ok(())
}

#[rstest]
#[case::unknown_argument(
    json!({ "name": "x", "cover": 1 }),
    &["\"cover\" is not an argument of `auth probe`: it takes name, with, local_only"]
)]
#[case::missing_required(json!({}), &["name is required by `auth probe`"])]
#[case::wrong_json_type(
    json!({ "name": 1, "local_only": "yes" }),
    &[
        "name of `auth probe` must be text",
        "local_only of `auth probe` must be true or false",
    ]
)]
#[case::value_not_a_choice(
    json!({ "name": "x", "with": ["tokens", "cookies"] }),
    &["with of `auth probe` must be a list, each one of tokens"]
)]
#[case::all_at_once(
    json!({ "with": "tokens", "cover": true }),
    &[
        "\"cover\" is not an argument of `auth probe`: it takes name, with, local_only",
        "name is required by `auth probe`",
        "with of `auth probe` must be a list, each one of tokens",
    ]
)]
fn wrong_arguments_are_reported_in_our_words(
    #[case] given: Value,
    #[case] wanted: &[&str],
) {
    let reported = parse::<ProbeArgs>(arguments(&given))
        .err()
        .map(|error| error.to_string());
    assert_eq!(reported, Some(listed(wanted)));
}
#[test]
fn items_are_a_list_of_objects() -> crate::Result<()> {
    let none: KindProbe = parse(Map::new())?;
    let two: KindProbe = parse(arguments(&json!({ "items": [{ "a": 1 }, {}] })))?;
    assert_eq!((none.items.0.len(), two.items.0.len()), (0, 2));
    let wrong = parse::<KindProbe>(arguments(&json!({ "items": [1] })))
        .err()
        .map(|error| error.to_string());
    assert_eq!(
        wrong.as_deref(),
        Some("items of `object probe` must be a list of objects, one per call")
    );
    Ok(())
}

#[test]
fn an_argument_for_one_kind_is_refused_for_another() -> crate::Result<()> {
    let shape: KindProbe =
        parse(arguments(&json!({ "kind": "shape", "outline": "x" })))?;
    let unsaid: KindProbe = parse(arguments(&json!({ "outline": "x" })))?;
    assert_eq!(
        (
            shape.outline.as_deref(),
            unsaid.outline.as_deref(),
            unsaid.kind.is_none()
        ),
        (Some("x"), Some("x"), true)
    );
    let refused =
        parse::<KindProbe>(arguments(&json!({ "kind": "sticker", "outline": "x" })))
            .err()
            .map(|error| error.to_string());
    assert_eq!(
        refused.as_deref(),
        Some(
            "\"outline\" is not an argument of `object probe sticker`: it takes kind, items"
        )
    );
    Ok(())
}

/// Whether the command's `kinds` do not fit its `subcommand` field.
fn kinds_are_flawed(command: &super::Command) -> bool {
    let subcommands: Vec<_> = command
        .arguments
        .iter()
        .filter(|argument| argument.subcommand)
        .collect();
    let mut limited = command
        .arguments
        .iter()
        .flat_map(|argument| argument.kinds.iter());
    match subcommands.as_slice() {
        [] => limited.next().is_some(),
        [only] => match only.kind {
            ValueKind::Choice(values) => limited.any(|kind| !values.contains(kind)),
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
            | ValueKind::QueryParameter => true,
        },
        _ => true,
    }
}

#[test]
fn a_subcommand_is_one_choice_and_its_kinds_are_among_the_choices() {
    let flawed: Vec<_> = CommandName::iter()
        .map(CommandName::command)
        .chain([&KindProbe::COMMAND])
        .filter(|command| kinds_are_flawed(command))
        .map(|command| command.verb)
        .collect();
    assert!(flawed.is_empty(), "{flawed:?}");
}

// The gh rules over the whole table. Each test lists every offender.

// The verbs a command may be named by.
const VERBS: [&str; 37] = [
    "login",
    "logout",
    "status",
    "ls",
    "show",
    "create",
    "rename",
    "log",
    "snap",
    "save",
    "export",
    "import",
    "edit",
    "mv",
    "cp",
    "group",
    "ungroup",
    "lock",
    "unlock",
    "align",
    "rm",
    "restore",
    "favorite",
    "unfavorite",
    "touch",
    "leave",
    "grant",
    "revoke",
    "transfer",
    "share",
    "add",
    "get",
    "reply",
    "resolve",
    "read",
    "clean",
    "tag",
];

// Commands on one surface only, each with its reason.
const ONE_SURFACE: [(&str, Surface, &str); 4] = [
    (
        "auth login",
        Surface::Cli,
        "the MCP server logs in in a terminal with `holst-mcp setup`",
    ),
    (
        "auth logout",
        Surface::Cli,
        "forgets the login this machine keeps",
    ),
    ("storage ls", Surface::Cli, "lists files on this machine"),
    (
        "storage clean",
        Surface::Cli,
        "removes files on this machine",
    ),
];

// Commands that write and still have no `dry_run`, each with its reason.
const KNOWN_WITHOUT_DRY_RUN: [(&str, &str); 6] = [
    (
        "auth login",
        "saves the login on this machine; `--no-save` is its check",
    ),
    (
        "auth logout",
        "forgets the login on this machine; nothing of the server's",
    ),
    (
        "board save",
        "writes a folder on this machine; the tree gives it no -n",
    ),
    (
        "board export",
        "writes a file on this machine; the tree gives it no -n",
    ),
    (
        "backup get",
        "writes a file on this machine; the tree gives it no -n",
    ),
    (
        "object get",
        "writes a file on this machine; the tree gives it no -n",
    ),
];

fn commands() -> impl Iterator<Item = &'static super::Command> {
    CommandName::iter().map(CommandName::command)
}

fn takes(command: &super::Command, name: &str) -> bool {
    command
        .arguments
        .iter()
        .any(|argument| argument.name == name)
}

#[test]
fn every_verb_is_on_the_verb_list() {
    let offenders: Vec<String> = commands()
        .filter(|command| command.verb.is_some_and(|verb| !VERBS.contains(&verb)))
        .map(super::Command::path)
        .collect();
    assert_none("a verb is not on the verb list", &offenders);
}

fn confirm_flaws(command: &super::Command) -> Vec<String> {
    let path = command.path();
    let destroys = command.risk == CommandRisk::Destroy;
    command
        .arguments
        .iter()
        .filter_map(|argument| match (argument.name, argument.letter) {
            ("confirm", Some('y')) if argument.kind == ValueKind::Switch => None,
            ("confirm", _) => Some(format!("{path}: `confirm` is not the switch -y")),
            (name, Some('y')) => Some(format!("{path}: -y is `{name}`, not `confirm`")),
            _ => None,
        })
        .chain(
            (takes(command, "confirm") != destroys).then(|| {
                format!("{path}: takes `confirm` or is `Destroy`, but not both")
            }),
        )
        .collect()
}

#[test]
fn yes_is_on_exactly_the_commands_that_take_confirm() {
    let offenders: Vec<String> = commands().flat_map(confirm_flaws).collect();
    assert_none("-y/--yes", &offenders);
}

fn dry_run_flaws(command: &super::Command) -> Vec<String> {
    let path = command.path();
    let writes = command.risk != CommandRisk::Read;
    let known = KNOWN_WITHOUT_DRY_RUN
        .iter()
        .any(|(known, _)| *known == path);
    command
        .arguments
        .iter()
        .filter_map(|argument| match (argument.name, argument.letter) {
            ("dry_run", Some('n')) if argument.kind == ValueKind::Switch => None,
            ("dry_run", _) => Some(format!("{path}: `dry_run` is not the switch -n")),
            (name, Some('n')) => Some(format!("{path}: -n is `{name}`, not `dry_run`")),
            _ => None,
        })
        .chain(
            (takes(command, "dry_run") != (writes && !known))
                .then(|| format!("{path}: takes `dry_run` or writes, but not both")),
        )
        .collect()
}

#[test]
fn dry_run_is_on_exactly_the_commands_that_write() {
    let offenders: Vec<String> = commands().flat_map(dry_run_flaws).collect();
    assert_none("-n/--dry-run", &offenders);
}

#[test]
fn a_known_command_without_dry_run_still_exists_and_writes() {
    let stale: Vec<String> = KNOWN_WITHOUT_DRY_RUN
        .iter()
        .filter(|(path, _)| {
            !commands().any(|command| {
                command.path() == *path
                    && command.risk != CommandRisk::Read
                    && !takes(command, "dry_run")
            })
        })
        .map(|(path, reason)| format!("{path}: {reason}"))
        .collect();
    assert_none("KNOWN_WITHOUT_DRY_RUN lists a command that has one", &stale);
}

/// What `-w` and `-q` are: the long name and the kind of value, everywhere.
const LETTERS: [(char, &str, ValueKind); 2] = [
    ('w', "workspace", ValueKind::Text),
    ('p', "param", ValueKind::QueryParameter),
];

fn letter_flaws(command: &super::Command) -> Vec<String> {
    let path = command.path();
    command
        .arguments
        .iter()
        .flat_map(|argument| {
            let path = path.clone();
            LETTERS
                .iter()
                .filter_map(move |(letter, name, kind)| {
                    let has = argument.letter == Some(*letter);
                    let is = argument.name == *name && argument.kind.one() == *kind;
                    // A positional of that name (`ws show <workspace>`) is not the flag.
                    let named = argument.name == *name && !argument.positional;
                    ((has || named) && !(has && is)).then(|| {
                        format!(
                            "{path}: `{}` ({:?}, letter {:?}) is not -{letter} --{name} ({kind:?})",
                            argument.name, argument.kind, argument.letter
                        )
                    })
                })
        })
        .collect()
}

#[test]
fn o_w_and_q_mean_one_thing_everywhere() {
    let offenders: Vec<String> = commands().flat_map(letter_flaws).collect();
    assert_none("-o, -w, -p", &offenders);
}

#[test]
fn a_command_is_on_both_surfaces_unless_marked_with_a_reason() {
    let both = |command: &super::Command| {
        [Surface::Cli, Surface::Mcp]
            .iter()
            .all(|surface| command.surfaces.contains(surface))
    };
    let unmarked = commands()
        .filter(|command| {
            !both(command)
                && !ONE_SURFACE.iter().any(|(path, surface, _)| {
                    *path == command.path() && command.surfaces == [*surface]
                })
        })
        .map(|command| {
            format!(
                "{}: on {:?} only, with no reason",
                command.path(),
                command.surfaces
            )
        });
    let stale = ONE_SURFACE
        .iter()
        .filter(|(path, surface, reason)| {
            reason.is_empty()
                || !commands().any(|command| {
                    command.path() == *path && command.surfaces == [*surface]
                })
        })
        .map(|(path, surface, _)| {
            format!("{path}: listed as {surface:?} only, but is not")
        });
    let offenders: Vec<String> = unmarked.chain(stale).collect();
    assert_none("a surface", &offenders);
}
