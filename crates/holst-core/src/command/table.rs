//! The command table: each command's group, verb, risk, help and arguments.

use std::path::PathBuf;

use serde::{
    Deserialize,
    Deserializer,
    de::{DeserializeOwned, Error as _},
};
use serde_json::{Map, Value};
use strum::{IntoEnumIterator as _, VariantNames};

use crate::{
    command::{
        api::ApiArgs,
        auth::AuthVerb,
        backup::BackupVerb,
        board::BoardVerb,
        comment::CommentVerb,
        object::ObjectVerb,
        search::SearchArgs,
        storage::StorageVerb,
        template::TemplateVerb,
        workspace::WorkspaceVerb,
    },
    domain::login::Email,
    effect::{board_write::WriteMode, request::QueryParameter},
    error::{Error, Result, hint::Surface, input::InputError},
};

/// A command: a group's verb on the command line, a tool's action on MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub group: CommandGroup,
    /// `None` when the group is the command itself (`search`, `api`).
    pub verb: Option<&'static str>,
    pub about: &'static str,
    pub help: &'static str,
    pub examples: &'static [&'static str],
    pub block: HelpBlock,
    pub risk: CommandRisk,
    pub surfaces: &'static [Surface],
    pub arguments: &'static [Argument],
}

impl Command {
    #[must_use]
    pub fn path(&self) -> String {
        let group = self.group.help().name;
        self.verb
            .map_or_else(|| group.to_owned(), |verb| format!("{group} {verb}"))
    }

    /// Returns the arguments that exist on `surface` for the subcommand `kind`:
    /// the ones not limited to kinds, and those limited to it.
    pub fn arguments_for(
        &self,
        surface: Surface,
        kind: &'static str,
    ) -> impl Iterator<Item = &'static Argument> + use<> {
        self.arguments_on(surface).filter(move |argument| {
            !argument.subcommand
                && (argument.kinds.is_empty() || argument.kinds.contains(&kind))
        })
    }

    /// Returns the arguments that exist on `surface`.
    pub fn arguments_on(
        &self,
        surface: Surface,
    ) -> impl Iterator<Item = &'static Argument> + use<> {
        self.arguments
            .iter()
            .filter(move |argument| argument.surfaces.contains(&surface))
    }
}

/// An argument of a command: a flag on the command line, a property on MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argument {
    /// The field, the MCP name and, in kebab case, the long flag.
    pub name: &'static str,
    pub letter: Option<char>,
    pub kind: ValueKind,
    pub required: bool,
    /// Typed without a flag, in the order of the fields (CLI only).
    pub positional: bool,
    pub value_name: &'static str,
    pub help: &'static str,
    pub tool_doc: Option<&'static str>,
    /// Where the argument exists; both by default.
    pub surfaces: &'static [Surface],
    /// One command-line subcommand per value (`object add sticker`).
    pub subcommand: bool,
    /// The subcommand values this is for; all when empty.
    pub kinds: &'static [&'static str],
}

/// The values an argument takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Switch,
    Text,
    Integer,
    Number,
    /// The path of a file the command creates, never over an existing one.
    NewFile,
    /// Text; the command line also takes `@FILE` or `-`, which it reads itself.
    TextOrFile,
    Choice(&'static [&'static str]),
    /// A choice on the command line, text in the MCP schema.
    Scale(&'static [&'static str]),
    List(&'static ValueKind),
    /// A value, or a list of them: the JSON name takes either shape.
    OneOrMany(&'static ValueKind),
    /// A list of JSON objects, each with the arguments of one call: `--items`.
    Items,
    /// `NAME=VALUE` query parameters, repeated or as one object.
    QueryParameter,
}

impl ValueKind {
    // Recursion is bounded by the nesting of the field's type.
    fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (ValueKind::Switch, Value::Bool(_))
            | (
                ValueKind::Text
                | ValueKind::NewFile
                | ValueKind::TextOrFile
                | ValueKind::Scale(_),
                Value::String(_),
            )
            | (ValueKind::Number, Value::Number(_)) => true,
            (ValueKind::Integer, Value::Number(number)) => {
                number.is_i64() || number.is_u64()
            }
            (ValueKind::Choice(values), Value::String(text)) => {
                values.contains(&text.as_str())
            }
            (
                ValueKind::List(item) | ValueKind::OneOrMany(item),
                Value::Array(items),
            ) => items.iter().all(|each| item.accepts(each)),
            (ValueKind::OneOrMany(item), single) => item.accepts(single),
            (ValueKind::Items, Value::Array(items)) => {
                items.iter().all(Value::is_object)
            }
            (ValueKind::QueryParameter, Value::Object(pairs)) => pairs
                .values()
                .all(|pair| matches!(pair, Value::String(_) | Value::Number(_))),
            (
                ValueKind::Switch
                | ValueKind::Text
                | ValueKind::Integer
                | ValueKind::Number
                | ValueKind::NewFile
                | ValueKind::TextOrFile
                | ValueKind::Choice(_)
                | ValueKind::Scale(_)
                | ValueKind::List(_)
                | ValueKind::Items
                | ValueKind::QueryParameter,
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Array(_)
                | Value::Object(_),
            ) => false,
        }
    }

    fn expected(self) -> String {
        match self {
            ValueKind::Switch => String::from("true or false"),
            ValueKind::Text | ValueKind::TextOrFile | ValueKind::Scale(_) => {
                String::from("text")
            }
            ValueKind::Integer => String::from("a whole number"),
            ValueKind::Number => String::from("a number"),
            ValueKind::NewFile => String::from("the path of a new file"),
            ValueKind::Choice(values) => format!("one of {}", values.join(", ")),
            ValueKind::List(item) => format!("a list, each {}", item.expected()),
            ValueKind::OneOrMany(item) => {
                format!("{} or a list of them", item.expected())
            }
            ValueKind::Items => String::from("a list of objects, one per call"),
            ValueKind::QueryParameter => {
                String::from("an object of text or number values")
            }
        }
    }

    /// The kind of one value: what a name that takes one or many has in common
    /// with the same name that takes one.
    #[must_use]
    pub const fn one(self) -> Self {
        match self {
            ValueKind::OneOrMany(item) => *item,
            ValueKind::Switch
            | ValueKind::Text
            | ValueKind::Integer
            | ValueKind::Number
            | ValueKind::NewFile
            | ValueKind::TextOrFile
            | ValueKind::Choice(_)
            | ValueKind::Scale(_)
            | ValueKind::List(_)
            | ValueKind::Items
            | ValueKind::QueryParameter => self,
        }
    }

    fn absent(self) -> Option<Value> {
        match self {
            ValueKind::Switch => Some(Value::Bool(false)),
            ValueKind::List(_) => Some(Value::Array(Vec::new())),
            ValueKind::QueryParameter => Some(Value::Object(Map::new())),
            ValueKind::Text
            | ValueKind::Integer
            | ValueKind::Number
            | ValueKind::NewFile
            | ValueKind::TextOrFile
            | ValueKind::Choice(_)
            | ValueKind::Scale(_)
            | ValueKind::Items
            | ValueKind::OneOrMany(_) => None,
        }
    }
}

/// What a command may change: decides `--dry-run`, `--yes` and MCP hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRisk {
    Read,
    Write,
    /// Needs `confirm`: exactly the commands that take the argument.
    Destroy,
}

/// The heading a group's help lists a command under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpBlock {
    Commands,
    General,
    Targeted,
    Look,
    Change,
    Yours,
    RightsAndSharing,
}

/// A command group: `holst <group>` and its MCP tool or tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandGroup {
    Auth,
    Workspace,
    Board,
    Object,
    Template,
    Comment,
    Backup,
    Storage,
    Search,
    Api,
}

impl CommandGroup {
    /// Returns what the root help, the group help and the MCP tools say of the
    /// group.
    #[must_use]
    pub const fn help(self) -> &'static GroupHelp {
        match self {
            CommandGroup::Auth => &AUTH,
            CommandGroup::Workspace => &WORKSPACE,
            CommandGroup::Board => &BOARD,
            CommandGroup::Object => &OBJECT,
            CommandGroup::Template => &TEMPLATE,
            CommandGroup::Comment => &COMMENT,
            CommandGroup::Backup => &BACKUP,
            CommandGroup::Storage => &STORAGE,
            CommandGroup::Search => &SEARCH,
            CommandGroup::Api => &API,
        }
    }
}

/// The help of a command group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupHelp {
    pub name: &'static str,
    pub alias: Option<&'static str>,
    pub about: &'static str,
    pub section: RootSection,
    pub tools: GroupTools,
    /// The flags its commands share, shown in the group help.
    pub flags: &'static [GroupFlag],
    /// The ARGUMENTS paragraph of the group help.
    pub arguments: Option<&'static str>,
    pub examples: &'static [&'static str],
}

/// The section of the root help a group is listed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSection {
    Core,
    More,
    Local,
    Advanced,
}

/// The MCP tools of a group, by their descriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupTools {
    CliOnly,
    One {
        description: &'static str,
    },
    /// A read tool named as the group and a `<group>_write` tool.
    ReadAndWrite {
        read: &'static str,
        write: &'static str,
    },
}

/// A flag the commands of a group share, as the group help shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupFlag {
    pub name: &'static str,
    pub letter: Option<char>,
    pub value_name: &'static str,
    pub help: &'static str,
}

const WORKSPACE_FLAG: &str = "workspace";

const AUTH: GroupHelp = GroupHelp {
    name: "auth",
    alias: None,
    about: "Log in, log out and check the saved login",
    section: RootSection::More,
    tools: GroupTools::One {
        description: "The account and its workspaces. Start here. login: a link for the user to open.",
    },
    flags: &[],
    arguments: None,
    examples: &[
        "holst auth login --email you@example.com",
        "holst auth status",
        "holst auth logout --local-only",
    ],
};

const WORKSPACE: GroupHelp = GroupHelp {
    name: "workspace",
    alias: Some("ws"),
    about: "Work with workspaces",
    section: RootSection::Core,
    tools: GroupTools::One {
        description: "Workspaces.",
    },
    flags: &[],
    arguments: Some(
        "A workspace can be given as its id or part of its name.\n\
         Left out, it is your only workspace.",
    ),
    examples: &["holst ws ls", "holst ws show easy --with members,activity"],
};

const BOARD: GroupHelp = GroupHelp {
    name: "board",
    alias: Some("b"),
    about: "Work with boards and folders",
    section: RootSection::Core,
    tools: GroupTools::ReadAndWrite {
        read: "Boards and folders: list, show, picture.",
        write: "Change boards and folders and who may use them; save to files.",
    },
    flags: &[GroupFlag {
        name: WORKSPACE_FLAG,
        letter: Some('w'),
        value_name: "ws",
        help: "Where a board name is looked up, or where a new board goes",
    }],
    arguments: Some(
        "A board can be given as its id, its link or part of its name;\n\
         several matches are listed, never guessed. A link to a frame or an object selects\n\
         that part where a command takes a part (show, snap, save).\n\
         A folder is given the same way; / is the root.",
    ),
    examples: &["holst b ls -w easy", "holst b rm \"Old draft\""],
};

const OBJECT: GroupHelp = GroupHelp {
    name: "object",
    alias: Some("o"),
    about: "Work with the objects on a board",
    section: RootSection::Core,
    tools: GroupTools::ReadAndWrite {
        read: "Objects on a board or a part of it: kind, place, size, color, text.",
        write: "Objects on a board: add or edit many in one call, copy, group, lock, remove, download.",
    },
    flags: &[GroupFlag {
        name: WORKSPACE_FLAG,
        letter: Some('w'),
        value_name: "ws",
        help: "Where a board name is looked up",
    }],
    arguments: Some(
        "The board is given as in `holst b -h`. Objects are given with --object: an id, a link,\n\
         an @N from a listing, or the first 8 characters of the id; a link to an object\n\
         as the board names that object too.",
    ),
    examples: &[
        "holst o ls \"Roadmap\" -k sticker --contains \"design\"",
        "holst o add sticker \"Roadmap\" \"Ask design\" -c yellow4",
        "holst o add frame \"Roadmap\" \"Q4\" --items q4.json",
        "holst o rm \"https://app.holst.so/board/…?object=…\"",
    ],
};

const TEMPLATE: GroupHelp = GroupHelp {
    name: "template",
    alias: Some("tpl"),
    about: "Work with board templates",
    section: RootSection::More,
    tools: GroupTools::One {
        description: "Board templates.",
    },
    flags: &[],
    arguments: None,
    examples: &[
        "holst tpl ls --view standard --category strategic-planning",
        "holst tpl create \"Roadmap\" --name \"Quarter plan\" --for workspace",
    ],
};

const COMMENT: GroupHelp = GroupHelp {
    name: "comment",
    alias: None,
    about: "Read and write comment threads",
    section: RootSection::More,
    tools: GroupTools::One {
        description: "Comment threads of a board: list, start, reply, resolve, delete, mark read.",
    },
    flags: &[],
    arguments: None,
    examples: &[
        "holst comment ls \"Roadmap\"",
        "holst comment add \"Roadmap\" \"Is this final?\" --object a1b2c3d4",
    ],
};

const BACKUP: GroupHelp = GroupHelp {
    name: "backup",
    alias: None,
    about: "List, download and restore board backups",
    section: RootSection::More,
    tools: GroupTools::One {
        description: "A board's automatic backups: list, download, restore.",
    },
    flags: &[],
    arguments: None,
    examples: &[
        "holst backup ls \"Roadmap\"",
        "holst backup restore \"Roadmap\" <key>",
    ],
};

const STORAGE: GroupHelp = GroupHelp {
    name: "storage",
    alias: None,
    about: "See and clean what holst keeps on this machine",
    section: RootSection::Local,
    tools: GroupTools::CliOnly,
    flags: &[],
    arguments: None,
    examples: &["holst storage ls", "holst storage clean -n"],
};

const SEARCH: GroupHelp = GroupHelp {
    name: "search",
    alias: None,
    about: "Find boards by name and text on boards",
    section: RootSection::Core,
    tools: GroupTools::One {
        description: "Boards and folders by name, text on boards, or both; best first.",
    },
    flags: &[],
    arguments: None,
    examples: &[],
};

const API: GroupHelp = GroupHelp {
    name: "api",
    alias: None,
    about: "Send any request to the Holst API",
    section: RootSection::Advanced,
    tools: GroupTools::One {
        description: "Any Holst API endpoint; only GET unless a human set HOLST_MCP_RAW_WRITE=1.",
    },
    flags: &[],
    arguments: None,
    examples: &["holst api GET /gapi/current_user"],
};

/// Every command of the table, by group and verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandName {
    Auth(AuthVerb),
    Workspace(WorkspaceVerb),
    Board(BoardVerb),
    Comment(CommentVerb),
    Template(TemplateVerb),
    Backup(BackupVerb),
    Storage(StorageVerb),
    Object(ObjectVerb),
    /// A group that is the command itself.
    Search,
    Api,
}

impl CommandName {
    /// Returns every command, group by group.
    pub fn iter() -> impl Iterator<Item = Self> {
        AuthVerb::iter()
            .map(Self::Auth)
            .chain(WorkspaceVerb::iter().map(Self::Workspace))
            .chain(BoardVerb::iter().map(Self::Board))
            .chain(CommentVerb::iter().map(Self::Comment))
            .chain(TemplateVerb::iter().map(Self::Template))
            .chain(BackupVerb::iter().map(Self::Backup))
            .chain(StorageVerb::iter().map(Self::Storage))
            .chain(ObjectVerb::iter().map(Self::Object))
            .chain([Self::Search, Self::Api])
    }

    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            Self::Auth(verb) => verb.command(),
            Self::Workspace(verb) => verb.command(),
            Self::Board(verb) => verb.command(),
            Self::Comment(verb) => verb.command(),
            Self::Template(verb) => verb.command(),
            Self::Backup(verb) => verb.command(),
            Self::Storage(verb) => verb.command(),
            Self::Object(verb) => verb.command(),
            Self::Search => &SearchArgs::COMMAND,
            Self::Api => &ApiArgs::COMMAND,
        }
    }

    /// Returns the command named by a group's name and a verb.
    #[must_use]
    pub fn parse(group: &str, verb: Option<&str>) -> Option<Self> {
        Self::iter().find(|name| {
            let command = name.command();
            command.group.help().name == group && command.verb == verb
        })
    }
}

/// The arguments of one command, on both surfaces.
pub trait CommandArgs: DeserializeOwned {
    const COMMAND: Command;
}

/// A type a command argument can have.
pub trait ArgumentValue {
    const KIND: ValueKind;
    /// The kind of a list of this type.
    const LIST: ValueKind = ValueKind::List(&Self::KIND);
    const REQUIRED: bool = true;
}

/// A closed set of argument values, named by its snake case variants.
pub trait EnumArgument: VariantNames {}

impl<T: EnumArgument> ArgumentValue for T {
    const KIND: ValueKind = ValueKind::Choice(T::VARIANTS);
}

impl ArgumentValue for String {
    const KIND: ValueKind = ValueKind::Text;
}

impl ArgumentValue for Email {
    const KIND: ValueKind = ValueKind::Text;
}

impl ArgumentValue for QueryParameter {
    const KIND: ValueKind = ValueKind::QueryParameter;
    const LIST: ValueKind = ValueKind::QueryParameter;
}

/// Query parameters from an object of text or number values, in key order.
///
/// # Errors
/// The deserializer's, or a value that is neither text nor a number.
pub fn query_object<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<QueryParameter>, D::Error> {
    Map::<String, Value>::deserialize(deserializer)?
        .into_iter()
        .map(|(name, value)| match value {
            Value::String(value) => Ok(QueryParameter { name, value }),
            Value::Number(number) => Ok(QueryParameter {
                name,
                value: number.to_string(),
            }),
            Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => Err(
                D::Error::custom(format_args!("{name} must be text or a number")),
            ),
        })
        .collect()
}

impl ArgumentValue for bool {
    const KIND: ValueKind = ValueKind::Switch;
    const REQUIRED: bool = false;
}

impl ArgumentValue for u32 {
    const KIND: ValueKind = ValueKind::Integer;
}

impl ArgumentValue for i64 {
    const KIND: ValueKind = ValueKind::Integer;
}

impl ArgumentValue for f64 {
    const KIND: ValueKind = ValueKind::Number;
}

/// A file the command creates; it never replaces one that exists.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct NewFile(pub PathBuf);

impl ArgumentValue for NewFile {
    const KIND: ValueKind = ValueKind::NewFile;
}

/// Text that the command line may give as `@FILE` or `-`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct TextOrFile(pub String);

impl ArgumentValue for TextOrFile {
    const KIND: ValueKind = ValueKind::TextOrFile;
}

/// One value or a list of them; the JSON name takes a value or an array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneOrMany<T>(pub Vec<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OneOrMany<T> {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape<T> {
            Many(Vec<T>),
            One(T),
        }
        Ok(Self(match Shape::deserialize(deserializer)? {
            Shape::Many(many) => many,
            Shape::One(one) => vec![one],
        }))
    }
}

impl<T: ArgumentValue> ArgumentValue for OneOrMany<T> {
    const KIND: ValueKind = ValueKind::OneOrMany(&T::KIND);
}

/// Calls in one: a list of JSON objects, each the arguments of one call.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Items<T>(pub Vec<T>);

impl<T> Default for Items<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> ArgumentValue for Items<T> {
    const KIND: ValueKind = ValueKind::Items;
    const REQUIRED: bool = false;
}

/// Arguments several commands share, joined in by `#[argument(flatten)]`.
pub trait ArgumentGroup {
    const ARGUMENTS: &'static [Argument];
}

const BLANK: Argument = Argument {
    name: "",
    letter: None,
    kind: ValueKind::Switch,
    required: false,
    positional: false,
    value_name: "",
    help: "",
    tool_doc: None,
    surfaces: &[Surface::Cli, Surface::Mcp],
    subcommand: false,
    kinds: &[],
};

/// How many arguments `parts` hold together: the length of [`joined`].
#[must_use]
pub const fn joined_len(mut parts: &[&[Argument]]) -> usize {
    let mut total = 0;
    // imperative: a const fn has no iterators
    while let [first, rest @ ..] = parts {
        total += first.len();
        parts = rest;
    }
    total
}

/// The arguments of `parts` in one array, in order; `N` is [`joined_len`].
#[must_use]
#[expect(
    clippy::indexing_slicing,
    reason = "N is the sum of the lengths, so every index is in range"
)]
pub const fn joined<const N: usize>(mut parts: &[&[Argument]]) -> [Argument; N] {
    let mut joined = [BLANK; N];
    let mut filled = 0;
    // imperative: a const fn has no iterators
    while let [first, rest @ ..] = parts {
        let mut part: &[Argument] = first;
        // imperative: a const fn has no iterators
        while let [argument, more @ ..] = part {
            joined[filled] = *argument;
            filled += 1;
            part = more;
        }
        parts = rest;
    }
    joined
}

impl ArgumentValue for WriteMode {
    const KIND: ValueKind = ValueKind::Switch;
    const REQUIRED: bool = false;
}

/// `--yes`: the caller agrees to a change that reaches further than it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "bool")]
pub enum Confirmation {
    Missing,
    Given,
}

impl From<bool> for Confirmation {
    fn from(yes: bool) -> Self {
        if yes {
            Confirmation::Given
        } else {
            Confirmation::Missing
        }
    }
}

impl ArgumentValue for Confirmation {
    const KIND: ValueKind = ValueKind::Switch;
    const REQUIRED: bool = false;
}

impl<T: ArgumentValue> ArgumentValue for Vec<T> {
    const KIND: ValueKind = T::LIST;
    const REQUIRED: bool = false;
}

impl<T: ArgumentValue> ArgumentValue for Option<T> {
    const KIND: ValueKind = T::KIND;
    const REQUIRED: bool = false;
}

/// Reads a command's arguments from one JSON object, checked against the table.
///
/// # Errors
/// Every [`InputError::NotArgument`], [`InputError::NoArgument`] and
/// [`InputError::NotArgumentValue`] at once;
/// [`InputError::NotCommandArguments`].
pub fn parse<A: CommandArgs>(arguments: Map<String, Value>) -> Result<A> {
    let command = &A::COMMAND;
    let given: Map<String, Value> = arguments
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .collect();
    let problems = problems(command, &given);
    if problems.is_empty() {
        read(command, given)
    } else {
        Err(Error::several(problems))
    }
}

/// The value of the command's subcommand argument, when it is given and one of
/// the argument's choices.
fn chosen_kind<'a>(
    command: &Command,
    given: &'a Map<String, Value>,
) -> Option<&'a str> {
    let argument = command
        .arguments
        .iter()
        .find(|argument| argument.subcommand)?;
    given
        .get(argument.name)
        .filter(|value| argument.kind.accepts(value))
        .and_then(Value::as_str)
}

fn problems(command: &Command, given: &Map<String, Value>) -> Vec<Error> {
    let chosen = chosen_kind(command, given);
    let path = chosen.map_or_else(
        || command.path(),
        |kind| format!("{} {kind}", command.path()),
    );
    let applicable: Vec<&Argument> = command
        .arguments
        .iter()
        .filter(|argument| {
            argument.kinds.is_empty()
                || chosen.is_none_or(|kind| argument.kinds.contains(&kind))
        })
        .collect();
    let known = |name: &str| applicable.iter().any(|argument| argument.name == name);
    let unknown =
        given
            .keys()
            .filter(|name| !known(name))
            .map(|name| InputError::NotArgument {
                command: path.clone(),
                name: name.clone(),
                taken: applicable.iter().map(|argument| argument.name).collect(),
            });
    let missing = applicable
        .iter()
        .filter(|argument| argument.required && !given.contains_key(argument.name))
        .map(|argument| InputError::NoArgument {
            command: path.clone(),
            name: argument.name,
        });
    let mistyped = applicable.iter().filter_map(|argument| {
        given
            .get(argument.name)
            .filter(|value| !argument.kind.accepts(value))
            .map(|_| InputError::NotArgumentValue {
                command: path.clone(),
                name: argument.name,
                expected: argument.kind.expected(),
            })
    });
    unknown
        .chain(missing)
        .chain(mistyped)
        .map(Error::from)
        .collect()
}

fn read<A: DeserializeOwned>(
    command: &Command,
    given: Map<String, Value>,
) -> Result<A> {
    let left_out: Vec<(String, Value)> = command
        .arguments
        .iter()
        .filter(|argument| !given.contains_key(argument.name))
        .filter_map(|argument| {
            argument
                .kind
                .absent()
                .map(|value| (argument.name.to_owned(), value))
        })
        .collect();
    serde_json::from_value(Value::Object(given.into_iter().chain(left_out).collect()))
        .map_err(|error| Error::from(InputError::NotCommandArguments(error)))
}

/// The argument `name` of `command` has a value it cannot take.
pub(in crate::command) fn not_value(
    command: &str,
    name: &'static str,
    expected: &str,
) -> Error {
    Error::from(InputError::NotArgumentValue {
        command: command.to_owned(),
        name,
        expected: expected.to_owned(),
    })
}

/// `text` without its edges, unless it is blank.
pub(in crate::command) fn visible(
    command: &str,
    name: &'static str,
    text: &str,
) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        Err(not_value(command, name, &format!("a visible {name}")))
    } else {
        Ok(text.to_owned())
    }
}

#[cfg(test)] mod tests;
