//! The help of the table's groups and commands, in the style of `gh`.

use holst_core::{
    command::table::{
        Argument,
        Command,
        CommandGroup,
        GroupFlag,
        HelpBlock,
        RootSection,
        ValueKind,
    },
    error::hint::Surface,
};

/// How wide the `name:` column of a group's command list is.
const NAME_COLUMN: usize = 13;
const EXIT_CODES: &str = "Exit codes: `holst help exit-codes`.";

/// The commands the root help shows as examples.
pub(crate) const ROOT_EXAMPLES: [&str; 5] = [
    "holst auth login",
    "holst b ls -w easy",
    "holst b snap \"Roadmap\" --frame \"Q3\"",
    "holst o add sticker \"Roadmap\" \"Ask design\" -c yellow4",
    "holst search \"retro\"",
];

/// Returns the long flag of an argument: its name in kebab case, except
/// `confirm`, which the command line calls `--yes`.
pub(crate) fn long(name: &str) -> String {
    match name {
        "confirm" => String::from("yes"),
        _ => kebab(name),
    }
}

/// Returns the kebab case of a snake case name: the way a flag or a value is
/// typed.
#[must_use]
pub(crate) fn kebab(name: &str) -> String {
    name.replace('_', "-")
}

/// Returns the items without repeats, in the order they first appear.
#[must_use]
pub(crate) fn first_seen<T: PartialEq + Copy>(
    items: impl IntoIterator<Item = T>,
) -> Vec<T> {
    items.into_iter().fold(Vec::new(), |seen, item| {
        seen.iter()
            .copied()
            .chain((!seen.contains(&item)).then_some(item))
            .collect()
    })
}

/// Returns the help of `holst`: the groups by the section of the root help
/// they belong to, the flags every command has, examples.
#[must_use]
pub(crate) fn root(groups: &[CommandGroup], root: &clap::Command) -> String {
    let sections = [RootSection::Core, RootSection::More, RootSection::Local]
        .into_iter()
        .filter_map(|wanted| {
            let mut helps: Vec<_> = groups
                .iter()
                .map(|group| group.help())
                .filter(|help| help.section == wanted)
                .collect();
            helps.sort_by_key(|help| help.name);
            (!helps.is_empty()).then(|| {
                section(
                    root_heading(wanted),
                    helps.iter().map(|help| {
                        let name = format!("{}:", help.name);
                        let alias = help
                            .alias
                            .map(|alias| format!(" (alias: {alias})"))
                            .unwrap_or_default();
                        format!("{name:<NAME_COLUMN$}{}{alias}", help.about)
                    }),
                )
            })
        });
    let globals: Vec<_> = root
        .get_arguments()
        .filter(|argument| argument.is_global_set())
        .filter_map(|argument| {
            let long = argument.get_long()?;
            let value = argument
                .get_value_names()
                .filter(|_| argument.get_action().takes_values())
                .and_then(|names| names.first())
                .map(ToString::to_string);
            let help = argument.get_help().map(ToString::to_string)?;
            Some((argument.get_short(), long, value, help))
        })
        .collect();
    let flags = globals
        .iter()
        .map(|(letter, long, value, help)| FlagLine {
            letter: *letter,
            long: (*long).to_owned(),
            value: value.as_deref(),
            help,
        })
        .chain([
            FlagLine {
                letter: Some('V'),
                long: String::from("version"),
                value: None,
                help: "Show the version",
            },
            FlagLine {
                letter: Some('h'),
                long: String::from("help"),
                value: None,
                help: "Show help for a command",
            },
        ]);
    ["Work with Holst boards from the command line.".to_owned()]
        .into_iter()
        .chain([section(
            "USAGE",
            ["holst <command> <subcommand> [flags]"],
        )])
        .chain(sections)
        .chain(flags_section("FLAGS", flags))
        .chain(examples(ROOT_EXAMPLES.iter().copied()))
        .chain([section(
            "LEARN MORE",
            [
                "Use `holst <command> <subcommand> -h` for more information about a command.",
                "Learn about exit codes using `holst help exit-codes`.",
                "On a terminal holst asks before a held change or on several matches; `HOLST_NO_PROMPT=1` turns that off.",
            ],
        )])
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn root_heading(section: RootSection) -> &'static str {
    match section {
        RootSection::Core => "CORE COMMANDS",
        RootSection::More => "MORE COMMANDS",
        RootSection::Local => "LOCAL COMMANDS",
    }
}

/// Returns the help of `holst <group>`: its commands by block, flags, arguments
/// and examples.
#[must_use]
pub(crate) fn group(group: CommandGroup, commands: &[&Command]) -> String {
    let help = group.help();
    let blocks = first_seen(commands.iter().map(|command| command.block))
        .into_iter()
        .map(|block| {
            section(
                heading(block),
                commands
                    .iter()
                    .filter(|command| command.block == block)
                    .map(|command| {
                        let name = format!("{}:", command.verb.unwrap_or(help.name));
                        format!("{name:<NAME_COLUMN$}{}", command.about)
                    }),
            )
        });
    let flags = help.flags.iter().map(flag_line_of_group);
    let sections = [format!("{}.", help.about)]
        .into_iter()
        .chain([section(
            "USAGE",
            [format!("holst {} <command> [flags]", help.name)],
        )])
        .chain(blocks)
        .chain(flags_section("FLAGS", flags))
        .chain(
            help.arguments
                .map(|text| section("ARGUMENTS", text.lines())),
        )
        .chain(examples(help.examples.iter().copied()));
    sections.collect::<Vec<_>>().join("\n\n")
}

/// Returns the arguments of `command` on the command line: for a kind of its
/// subcommand only the ones that are for it.
#[must_use]
pub(crate) fn arguments(
    command: &Command,
    kind: Option<&'static str>,
) -> Vec<&'static Argument> {
    kind.map_or_else(
        || command.arguments_on(Surface::Cli).collect(),
        |kind| command.arguments_for(Surface::Cli, kind).collect(),
    )
}

/// Returns what a command with a subcommand does for one of its kinds.
#[must_use]
pub(crate) fn kind_about(command: &Command, kind: &str) -> String {
    let action = command.about.split_whitespace().next().unwrap_or_default();
    let objects = match kind {
        "sticker-stack" => "sticker stacks".to_owned(),
        "flipcard" => "flip cards".to_owned(),
        "mind-map" => "mind-map nodes".to_owned(),
        "stamp" => "text stamps".to_owned(),
        "code" => "code blocks".to_owned(),
        "dice" => "dice".to_owned(),
        "reaction-stamp" => "reaction stamps".to_owned(),
        "task" => "task cards".to_owned(),
        "drawing" => "freehand strokes".to_owned(),
        "wheel" => "spinner wheels".to_owned(),
        other => format!("{}s", kebab(other)),
    };
    format!("{action} {objects}")
}

/// Returns the help of `holst <group> <verb>`, or of its `kind`; `inherited`
/// names the flags every command has.
#[must_use]
pub(crate) fn command(
    command: &Command,
    kind: Option<&'static str>,
    inherited: &[String],
) -> String {
    let inherited_line = format!("{}   as in `holst -h`", inherited.join(", "));
    let arguments = arguments(command, kind);
    let sections = [command.help.to_owned()]
        .into_iter()
        .chain([section("USAGE", [usage(command, kind)])])
        .chain(positionals_section(&arguments))
        .chain(flags_section(
            "FLAGS",
            arguments
                .iter()
                .copied()
                .filter(|argument| !argument.positional)
                .map(flag_line_of_argument),
        ))
        .chain([section("INHERITED FLAGS", [inherited_line])])
        .chain(examples(command.examples.iter().copied().filter(
            |example| kind.is_none_or(|kind| is_of_kind(example, kind)),
        )))
        .chain([section("LEARN MORE", learn_more(command))]);
    sections.collect::<Vec<_>>().join("\n\n")
}

/// Whether a command-line example names `kind` as a word.
fn is_of_kind(example: &str, kind: &str) -> bool {
    example.split_whitespace().any(|word| word == kebab(kind))
}

/// Returns the help of a command that has a subcommand: its kinds, each with
/// what it does.
#[must_use]
pub(crate) fn kinds(command: &Command, kinds: &[&str], inherited: &[String]) -> String {
    let inherited_line = format!("{}   as in `holst -h`", inherited.join(", "));
    let column = kinds
        .iter()
        .map(|kind| kebab(kind).len() + 3)
        .max()
        .unwrap_or(NAME_COLUMN)
        .max(NAME_COLUMN);
    let rows = kinds.iter().map(|kind| {
        let name = format!("{}:", kebab(kind));
        format!("{name:<column$}{}", kind_about(command, kind))
    });
    let sections = [command.help.to_owned()]
        .into_iter()
        .chain([section("USAGE", [usage(command, None)])])
        .chain([section("KINDS", rows)])
        .chain([section("INHERITED FLAGS", [inherited_line])])
        .chain(examples(command.examples.iter().copied()))
        .chain([section("LEARN MORE", learn_more(command))]);
    sections.collect::<Vec<_>>().join("\n\n")
}

/// Returns the usage line of a command or of one of its kinds: its arguments
/// without a flag, its required flags, then `[flags]` when it has any. A
/// command with a subcommand names it, since the kind comes first.
#[must_use]
pub(crate) fn usage(command: &Command, kind: Option<&'static str>) -> String {
    let chosen = command
        .arguments
        .iter()
        .find(|argument| argument.subcommand);
    let (kind_word, listed) = match (kind, chosen) {
        (Some(kind), _) => (Some(kebab(kind)), arguments(command, Some(kind))),
        (None, Some(chosen)) => (Some(format!("<{}>", chosen.value_name)), Vec::new()),
        (None, None) => (None, arguments(command, None)),
    };
    let positionals =
        listed
            .iter()
            .filter(|argument| argument.positional)
            .map(|argument| {
                let many = if matches!(
                    argument.kind,
                    ValueKind::OneOrMany(_) | ValueKind::List(_)
                ) {
                    "..."
                } else {
                    ""
                };
                let value = format!("<{}>{many}", argument.value_name);
                if argument.required {
                    value
                } else {
                    format!("[{value}]")
                }
            });
    let required_flags = listed
        .iter()
        .filter(|argument| !argument.positional && argument.required)
        .map(|argument| {
            let flag = format!("--{}", long(argument.name));
            if matches!(argument.kind, ValueKind::Switch) {
                flag
            } else {
                format!("{flag} {}", argument.value_name)
            }
        });
    let flags = (chosen.is_some() && kind.is_none()
        || listed.iter().any(|argument| !argument.positional))
    .then(|| String::from("[flags]"));
    ["holst", command.group.help().name]
        .into_iter()
        .map(str::to_owned)
        .chain(command.verb.map(str::to_owned))
        .chain(kind_word)
        .chain(positionals)
        .chain(required_flags)
        .chain(flags)
        .collect::<Vec<_>>()
        .join(" ")
}

fn positionals_section(arguments: &[&Argument]) -> Option<String> {
    let rows: Vec<(&str, &str)> = arguments
        .iter()
        .filter(|argument| argument.positional)
        .map(|argument| (argument.value_name, argument.help))
        .collect();
    let width = rows.iter().map(|(name, _)| name.chars().count()).max()?;
    Some(section(
        "ARGUMENTS",
        rows.iter()
            .map(|(name, help)| format!("{name:<width$}   {help}")),
    ))
}

fn heading(block: HelpBlock) -> &'static str {
    match block {
        HelpBlock::Commands => "COMMANDS",
        HelpBlock::General => "GENERAL COMMANDS",
        HelpBlock::Targeted => "TARGETED COMMANDS",
        HelpBlock::Look => "LOOK",
        HelpBlock::Change => "CHANGE",
        HelpBlock::Yours => "YOURS",
        HelpBlock::RightsAndSharing => "RIGHTS AND SHARING",
    }
}

/// The pointer to the page that explains how a board is given: every command
/// that takes a `board` argument gets it, whatever its group.
fn learn_more(command: &Command) -> Vec<String> {
    let page = |group: CommandGroup| {
        let help = group.help();
        help.alias.unwrap_or(help.name)
    };
    let takes_board = command
        .arguments
        .iter()
        .any(|argument| argument.name == "board");
    let own = match command.group {
        _ if takes_board => Some(format!(
            "How a board is given: `holst {} -h`.",
            page(CommandGroup::Board)
        )),
        CommandGroup::Workspace => Some(format!(
            "How a workspace is given: `holst {} -h`.",
            page(CommandGroup::Workspace)
        )),
        CommandGroup::Board => Some(format!(
            "How a board is given: `holst {} -h`.",
            page(CommandGroup::Board)
        )),
        CommandGroup::Template => {
            Some(String::from("A template is given as its id or its name."))
        }
        CommandGroup::Object
        | CommandGroup::Comment
        | CommandGroup::Backup
        | CommandGroup::Auth
        | CommandGroup::Storage
        | CommandGroup::Search => None,
    };
    own.into_iter().chain([String::from(EXIT_CODES)]).collect()
}

fn examples<'a>(examples: impl Iterator<Item = &'a str>) -> Option<String> {
    let lines: Vec<String> = examples.map(|example| format!("$ {example}")).collect();
    (!lines.is_empty()).then(|| section("EXAMPLES", lines))
}

fn section<T: AsRef<str>>(title: &str, lines: impl IntoIterator<Item = T>) -> String {
    lines.into_iter().fold(title.to_owned(), |text, line| {
        format!("{text}\n  {}", line.as_ref())
    })
}

/// One flag as the help writes it, before the columns are aligned.
struct FlagLine<'a> {
    letter: Option<char>,
    long: String,
    value: Option<&'a str>,
    help: &'a str,
}

impl FlagLine<'_> {
    fn left(&self, indent_letterless: bool) -> String {
        let long = self.value.map_or_else(
            || format!("--{}", self.long),
            |value| format!("--{} {value}", self.long),
        );
        match (self.letter, indent_letterless) {
            (Some(letter), _) => format!("-{letter}, {long}"),
            (None, true) => format!("    {long}"),
            (None, false) => long,
        }
    }
}

fn flag_line_of_argument(argument: &Argument) -> FlagLine<'_> {
    FlagLine {
        letter: argument.letter,
        long: long(argument.name),
        value: match argument.kind {
            ValueKind::Switch => None,
            ValueKind::Text
            | ValueKind::Integer
            | ValueKind::Number
            | ValueKind::NewFile
            | ValueKind::TextOrFile
            | ValueKind::Choice(_)
            | ValueKind::Scale(_)
            | ValueKind::List(_)
            | ValueKind::OneOrMany(_)
            | ValueKind::Items
            | ValueKind::QueryParameter => Some(argument.value_name),
        },
        help: argument.help,
    }
}

fn flag_line_of_group(flag: &GroupFlag) -> FlagLine<'_> {
    FlagLine {
        letter: flag.letter,
        long: kebab(flag.name),
        value: Some(flag.value_name).filter(|value| !value.is_empty()),
        help: flag.help,
    }
}

/// Returns the section of flags, `None` when there are none.
fn flags_section<'a>(
    title: &str,
    flags: impl Iterator<Item = FlagLine<'a>>,
) -> Option<String> {
    let flags: Vec<FlagLine<'a>> = flags.collect();
    let indent_letterless = flags.iter().any(|flag| flag.letter.is_some());
    let width = flags
        .iter()
        .map(|flag| flag.left(indent_letterless).chars().count())
        .max()?;
    Some(section(
        title,
        flags.iter().map(|flag| {
            format!("{:<width$}   {}", flag.left(indent_letterless), flag.help)
        }),
    ))
}

#[cfg(test)]
mod tests {
    use crate::{
        cli::{Cli, command},
        help::{ROOT_EXAMPLES, first_seen},
    };

    #[test]
    fn the_root_help_is_the_gh_page() {
        let shown = command()
            .try_get_matches_from(["holst", "-h"])
            .err()
            .map(|help| help.to_string())
            .unwrap_or_default();
        insta::assert_snapshot!("root", shown);
    }

    /// The words of a command line, a quoted text being one word.
    fn words(line: &str) -> Vec<&str> {
        line.split('"')
            .enumerate()
            .flat_map(|(at, part)| {
                if at % 2 == 1 {
                    vec![part]
                } else {
                    part.split_whitespace().collect()
                }
            })
            .collect()
    }

    #[test]
    fn the_root_examples_parse() -> Result<(), clap::Error> {
        ROOT_EXAMPLES
            .iter()
            .try_for_each(|line| Cli::parse_from_args(words(line)).map(drop))
    }

    #[test]
    fn repeats_are_left_out_in_order() {
        assert_eq!(first_seen([2, 1, 2, 3, 1]), [2, 1, 3]);
    }
}
