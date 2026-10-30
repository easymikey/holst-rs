//! The MCP tools built from the command table: one per group, or two.

use holst_core::{
    command::table::{
        Argument,
        Command,
        CommandName,
        CommandRisk,
        GroupTools,
        ValueKind,
    },
    error::hint::Surface,
};
use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use crate::tools::output::Output;

const CONFIRM: &str = "confirm";
const DRY_RUN: &str = "dry_run";

/// Hidden names: (alias, the key a command takes in its place).
const ALIASES: [(&str, &str); 2] = [("into", "frame"), ("to", "side")];

/// Why a call to a table tool names no command, in words for the model.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CallError {
    #[error("`{tool}` needs `action`: {actions}")]
    NoAction { tool: String, actions: String },

    #[error("`{tool}` has no action {action}; it has: {actions}")]
    NotAction {
        tool: String,
        action: Value,
        actions: String,
    },

    #[error("`{tool}` has no action {action}; {action} is a `{other}` action")]
    OtherTool {
        tool: String,
        action: String,
        other: String,
    },

    #[error("`{alias}` is another name for `{key}`: give one of them, not both")]
    BothNames {
        alias: &'static str,
        key: &'static str,
    },
}

/// One MCP tool: the commands of a group that share its permission prompt.
#[derive(Debug)]
pub(crate) struct TableTool {
    name: String,
    description: &'static str,
    commands: Vec<CommandName>,
}

impl TableTool {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Names the command a call runs, taking `action` out of the arguments.
    /// A tool of one command needs no `action`; it takes one equal to its verb.
    pub(crate) fn pick(
        &self,
        arguments: &mut Map<String, Value>,
    ) -> Result<CommandName, CallError> {
        let name = match self.commands.as_slice() {
            // A verbless command (`search`) has no `action` to take out.
            [only] if only.command().verb.is_none() => Ok(*only),
            _ => self.named(arguments),
        }?;
        rename_aliases(name.command(), arguments)?;
        drop_idle_switches(name.command(), arguments);
        Ok(name)
    }

    fn named(
        &self,
        arguments: &mut Map<String, Value>,
    ) -> Result<CommandName, CallError> {
        let given = arguments.remove("action");
        let verb = given.as_ref().and_then(Value::as_str);
        let found = verb.and_then(|verb| {
            self.commands
                .iter()
                .copied()
                .find(|name| name.command().verb == Some(verb))
        });
        match (found, given, self.commands.as_slice()) {
            (Some(found), ..) => Ok(found),
            (None, None, [only]) => Ok(*only),
            (None, None, _) => Err(self.no_action()),
            (None, Some(given), _) => Err(self.not_action(given)),
        }
    }

    fn no_action(&self) -> CallError {
        CallError::NoAction {
            tool: self.name.clone(),
            actions: self.verbs().join(", "),
        }
    }

    fn not_action(&self, given: Value) -> CallError {
        let other = given.as_str().and_then(|verb| self.sibling_with(verb));
        let tool = self.name.clone();
        match (other, given) {
            (Some(other), Value::String(action)) => CallError::OtherTool {
                tool,
                action,
                other,
            },
            (_, action) => CallError::NotAction {
                tool,
                action,
                actions: self.verbs().join(", "),
            },
        }
    }

    /// The other tool of this tool's group that has the verb.
    fn sibling_with(&self, verb: &str) -> Option<String> {
        let group = self.commands.first()?.command().group;
        tools()
            .into_iter()
            .filter(|each| each.name != self.name)
            .find(|each| {
                each.commands.iter().any(|name| {
                    let command = name.command();
                    command.group == group && command.verb == Some(verb)
                })
            })
            .map(|each| each.name)
    }

    fn verbs(&self) -> Vec<&'static str> {
        self.commands
            .iter()
            .filter_map(|name| name.command().verb)
            .collect()
    }

    /// The tool as `tools/list` shows it: compact JSON.
    pub(crate) fn tool(&self) -> Tool {
        let commands: Vec<&Command> =
            self.commands.iter().map(|name| name.command()).collect();
        let read_only = commands
            .iter()
            .all(|command| command.risk == CommandRisk::Read);
        // Every tool is about Holst and local files, never the open web.
        let hints = ToolAnnotations::new()
            .read_only(read_only)
            .open_world(false);
        let hints = if read_only {
            hints
        } else {
            hints.destructive(true)
        };
        Tool::new(self.name.clone(), self.description, self.schema(&commands))
            .annotate(hints)
    }

    fn schema(&self, commands: &[&Command]) -> Map<String, Value> {
        let action = (commands.len() > 1).then(|| {
            (
                "action".to_owned(),
                json!({ "type": "string", "enum": self.verbs() }),
            )
        });
        // The last of a name wins, so a name that takes one or many goes last.
        let mut arguments: Vec<&Argument> = commands
            .iter()
            .flat_map(|command| command.arguments_on(Surface::Mcp))
            .collect();
        arguments
            .sort_by_key(|argument| matches!(argument.kind, ValueKind::OneOrMany(_)));
        let output = self
            .commands
            .iter()
            .any(|name| crate::tools::output::on(*name))
            .then(|| ("output".to_owned(), kind(ValueKind::Choice(&Output::NAMES))));
        let properties: Map<String, Value> = action
            .clone()
            .into_iter()
            .chain(
                arguments
                    .into_iter()
                    .map(|argument| (argument.name.to_owned(), property(argument))),
            )
            .chain(output)
            .collect();
        let required: Vec<&str> = action
            .iter()
            .map(|_| "action")
            .chain(
                commands
                    .iter()
                    .take(1)
                    .flat_map(|command| command.arguments_on(Surface::Mcp))
                    .filter(|argument| required_by_all(commands, argument))
                    .map(|argument| argument.name),
            )
            .collect();
        let mut schema = Map::new();
        schema.insert("type".to_owned(), json!("object"));
        schema.insert("properties".to_owned(), Value::Object(properties));
        if !required.is_empty() {
            schema.insert("required".to_owned(), json!(required));
        }
        schema
    }
}

/// An alias stands for its key in a command that takes the key and not the
/// alias; both in one call are refused.
fn rename_aliases(
    command: &Command,
    arguments: &mut Map<String, Value>,
) -> Result<(), CallError> {
    let takes = |name: &str| {
        command
            .arguments_on(Surface::Mcp)
            .any(|each| each.name == name)
    };
    ALIASES
        .iter()
        .filter(|(alias, key)| !takes(alias) && takes(key))
        .try_for_each(|&(alias, key)| match arguments.remove(alias) {
            None | Some(Value::Null) => Ok(()),
            Some(_) if arguments.contains_key(key) => {
                Err(CallError::BothNames { alias, key })
            }
            Some(value) => {
                arguments.insert(key.to_owned(), value);
                Ok(())
            }
        })
}

/// A change that holds nothing takes `confirm: true`, and a command that
/// sends nothing takes `dry_run`; each does what it does without.
fn drop_idle_switches(command: &Command, arguments: &mut Map<String, Value>) {
    let holds = |name: &str| {
        command
            .arguments_on(Surface::Mcp)
            .any(|each| each.name == name)
    };
    let idle: Vec<&str> = [
        (CONFIRM, command.risk != CommandRisk::Read),
        (DRY_RUN, true),
    ]
    .into_iter()
    .filter(|&(name, applies)| {
        applies && !holds(name) && matches!(arguments.get(name), Some(Value::Bool(_)))
    })
    .map(|(name, _)| name)
    .collect();
    arguments.retain(|name, _| !idle.contains(&name.as_str()));
}

fn required_by_all(commands: &[&Command], argument: &Argument) -> bool {
    commands.iter().all(|command| {
        command
            .arguments_on(Surface::Mcp)
            .any(|each| each.name == argument.name && each.required)
    })
}

fn property(argument: &Argument) -> Value {
    let mut property = kind(argument.kind);
    if let (Some(doc), Some(map)) = (argument.tool_doc, property.as_object_mut()) {
        map.insert("description".to_owned(), json!(doc));
    }
    property
}

// Recursion is bounded by the nesting of the field's type.
fn kind(of: ValueKind) -> Value {
    match of {
        ValueKind::Switch => json!({ "type": "boolean" }),
        // A new file is the path of one.
        ValueKind::Text
        | ValueKind::NewFile
        | ValueKind::TextOrFile
        | ValueKind::Scale(_) => {
            json!({ "type": "string" })
        }
        ValueKind::Integer => json!({ "type": "integer" }),
        ValueKind::Number => json!({ "type": "number" }),
        ValueKind::Items => json!({ "type": "array", "items": { "type": "object" } }),
        ValueKind::QueryParameter => json!({
            "type": "object",
            "additionalProperties": { "type": ["string", "number"] }
        }),
        ValueKind::Choice(values) => json!({ "type": "string", "enum": values }),
        ValueKind::List(item) => json!({ "type": "array", "items": kind(*item) }),
        ValueKind::OneOrMany(item) => json!({
            "anyOf": [kind(*item), { "type": "array", "items": kind(*item) }]
        }),
    }
}

/// Which tool and description a command belongs to; `None` when MCP has no
/// tool for it.
fn placement(command: &Command) -> Option<(String, &'static str)> {
    let group = command.group.help();
    let for_mcp = command.surfaces.contains(&Surface::Mcp);
    let placed = match (group.tools, command.risk) {
        (GroupTools::CliOnly, _) => None,
        (GroupTools::One { description }, _) => {
            Some((group.name.to_owned(), description))
        }
        (GroupTools::ReadAndWrite { read, .. }, CommandRisk::Read) => {
            Some((group.name.to_owned(), read))
        }
        (
            GroupTools::ReadAndWrite { write, .. },
            CommandRisk::Write | CommandRisk::Destroy,
        ) => Some((format!("{}_write", group.name), write)),
    };
    placed.filter(|_| for_mcp)
}

/// Every tool of the table, in the order of its commands.
pub(crate) fn tools() -> Vec<TableTool> {
    CommandName::iter()
        .filter_map(|name| {
            placement(name.command())
                .map(|(tool, description)| (tool, description, name))
        })
        // imperative: a command joins the tool its name already opened.
        .fold(
            Vec::new(),
            |mut tools: Vec<TableTool>, (tool, description, name)| {
                match tools.iter_mut().find(|each| each.name == tool) {
                    Some(open) => open.commands.push(name),
                    None => tools.push(TableTool {
                        name: tool,
                        description,
                        commands: vec![name],
                    }),
                }
                tools
            },
        )
}

#[cfg(test)]
mod tests {
    use holst_core::command::{
        auth::AuthVerb,
        object::ObjectVerb,
        table::{CommandGroup, HelpBlock},
    };
    use rstest::rstest;

    use super::*;

    #[test]
    fn search_is_picked_without_an_action() {
        let picked = tools()
            .iter()
            .find(|tool| tool.name == "search")
            .map(|tool| tool.pick(&mut Map::new()));
        assert_eq!(picked, Some(Ok(CommandName::Search)));
    }

    fn two_actions() -> TableTool {
        TableTool {
            name: "auth".into(),
            description: "",
            commands: vec![
                CommandName::Auth(AuthVerb::Status),
                CommandName::Auth(AuthVerb::Logout),
            ],
        }
    }

    #[rstest]
    #[case::unknown(json!({ "action": "rm" }), CallError::NotAction {
        tool: "auth".into(), action: json!("rm"), actions: "status, logout".into()
    })]
    #[case::not_text(json!({ "action": 3 }), CallError::NotAction {
        tool: "auth".into(), action: json!(3), actions: "status, logout".into()
    })]
    #[case::missing(json!({}), CallError::NoAction {
        tool: "auth".into(), actions: "status, logout".into()
    })]
    fn a_call_without_a_known_action_says_what_the_tool_has(
        #[case] arguments: Value,
        #[case] error: CallError,
    ) {
        let mut arguments = arguments.as_object().cloned().unwrap_or_default();
        assert_eq!(two_actions().pick(&mut arguments), Err(error));
    }

    #[test]
    fn the_action_is_taken_out_of_the_arguments() {
        let mut arguments = json!({ "action": "logout", "local_only": true })
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            two_actions().pick(&mut arguments),
            Ok(CommandName::Auth(AuthVerb::Logout))
        );
        assert!(!arguments.contains_key("action"));
    }

    fn pick_on(
        tool: &str,
        arguments: &Value,
    ) -> (Option<Result<CommandName, CallError>>, Value) {
        let mut arguments = arguments.as_object().cloned().unwrap_or_default();
        let picked = tools()
            .iter()
            .find(|each| each.name == tool)
            .map(|each| each.pick(&mut arguments));
        (picked, Value::Object(arguments))
    }

    #[rstest]
    #[case::board("board", "rm", "board_write")]
    #[case::board_write("board_write", "ls", "board")]
    #[case::object("object", "rm", "object_write")]
    fn an_action_of_the_sibling_tool_names_that_tool(
        #[case] tool: &str,
        #[case] action: &str,
        #[case] other: &str,
    ) {
        assert_eq!(
            pick_on(tool, &json!({ "action": action })).0,
            Some(Err(CallError::OtherTool {
                tool: tool.to_owned(),
                action: action.to_owned(),
                other: other.to_owned(),
            }))
        );
    }

    #[test]
    fn the_sibling_error_reads_as_the_brief_says() {
        assert_eq!(
            pick_on("board", &json!({ "action": "rm" }))
                .0
                .and_then(Result::err)
                .map(|error| error.to_string()),
            Some("`board` has no action rm; rm is a `board_write` action".to_owned())
        );
    }

    #[test]
    fn an_action_no_tool_of_the_group_has_lists_the_tools_own() {
        assert_eq!(
            pick_on("object", &json!({ "action": "zap" })).0,
            Some(Err(CallError::NotAction {
                tool: "object".to_owned(),
                action: json!("zap"),
                actions: "ls".to_owned(),
            }))
        );
    }

    #[test]
    fn a_one_verb_tool_drops_the_action_equal_to_its_verb() {
        assert_eq!(
            pick_on("object", &json!({ "action": "ls", "board": "X" })),
            (
                Some(Ok(CommandName::Object(ObjectVerb::Ls))),
                json!({ "board": "X" })
            )
        );
    }

    #[rstest]
    #[case::add("add", json!({ "into": "Q3" }), json!({ "frame": "Q3" }))]
    #[case::edit("edit", json!({ "into": "@4" }), json!({ "frame": "@4" }))]
    #[case::align("align", json!({ "to": "left" }), json!({ "side": "left" }))]
    #[case::cp_keeps_its_own("cp", json!({ "into": "B" }), json!({ "into": "B" }))]
    #[case::a_null_alias("edit", json!({ "into": null, "frame": "Q" }), json!({ "frame": "Q" }))]
    fn an_alias_stands_for_the_key_the_action_takes(
        #[case] action: &str,
        #[case] given: Value,
        #[case] taken: Value,
    ) {
        let mut arguments = given.as_object().cloned().unwrap_or_default();
        arguments.insert("action".to_owned(), json!(action));
        let (picked, arguments) = pick_on("object_write", &Value::Object(arguments));
        assert_eq!(
            (picked.is_some_and(|picked| picked.is_ok()), arguments),
            (true, taken)
        );
    }

    #[test]
    fn an_alias_with_its_key_is_refused() {
        let arguments = json!({ "action": "add", "into": "Q3", "frame": "Q4" });
        assert_eq!(
            pick_on("object_write", &arguments).0,
            Some(Err(CallError::BothNames {
                alias: "into",
                key: "frame"
            }))
        );
    }

    #[test]
    fn dry_run_is_dropped_where_nothing_is_sent() {
        let arguments = json!({ "action": "get", "dry_run": true, "board": "X" });
        assert_eq!(
            pick_on("object_write", &arguments).1,
            json!({ "board": "X" })
        );
    }

    #[test]
    fn dry_run_stays_where_it_is_held() {
        let arguments = json!({ "action": "edit", "dry_run": true });
        assert_eq!(
            pick_on("object_write", &arguments).1,
            json!({ "dry_run": true })
        );
    }

    #[test]
    fn a_split_group_puts_its_changes_in_the_write_tool() {
        let command = |risk| Command {
            group: CommandGroup::Board,
            verb: Some("ls"),
            about: "",
            help: "",
            examples: &[],
            block: HelpBlock::Commands,
            risk,
            surfaces: &[Surface::Cli, Surface::Mcp],
            arguments: &[],
        };
        assert_eq!(
            [CommandRisk::Read, CommandRisk::Write, CommandRisk::Destroy]
                .map(|risk| placement(&command(risk)).map(|(name, _)| name)),
            [
                Some("board".to_owned()),
                Some("board_write".to_owned()),
                Some("board_write".to_owned())
            ]
        );
    }

    #[test]
    fn a_scale_is_plain_text_in_the_schema() {
        assert_eq!(
            kind(ValueKind::Scale(&["none", "viewer"])),
            json!({ "type": "string" })
        );
    }

    const fn argument(
        name: &'static str,
        kind: ValueKind,
        required: bool,
        surfaces: &'static [Surface],
    ) -> Argument {
        Argument {
            name,
            kind,
            required,
            surfaces,
            ..Argument::BLANK
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

    #[test]
    fn the_schema_has_a_property_for_every_kind_on_mcp_only() {
        let tool = TableTool {
            name: "probe".into(),
            description: "",
            commands: Vec::new(),
        };
        assert_eq!(
            Value::Object(tool.schema(&[&PROBE])),
            json!({
                "type": "object",
                "properties": {
                    "boards": {
                        "anyOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "scale": { "type": "number" },
                    "path": { "type": "string" },
                    "on_mcp": { "type": "string" }
                },
                "required": ["boards"]
            })
        );
    }

    #[test]
    fn a_write_action_takes_confirm_and_sends_what_it_sends_without() {
        let offenders: Vec<String> = tools()
            .iter()
            .flat_map(|tool| {
                tool.commands
                    .iter()
                    .filter(|name| name.command().risk != CommandRisk::Read)
                    .filter_map(|name| {
                        let verb = name.command().verb?;
                        let mut arguments = Map::from_iter([
                            ("action".to_owned(), json!(verb)),
                            ("confirm".to_owned(), json!(true)),
                        ]);
                        let picked = tool.pick(&mut arguments).ok()?;
                        let held = picked
                            .command()
                            .arguments_on(Surface::Mcp)
                            .any(|argument| argument.name == CONFIRM);
                        (held != arguments.contains_key(CONFIRM)).then(|| {
                            format!(
                                "{}: confirm is kept where nothing is held",
                                picked.command().path()
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        none("confirm on a write action", &offenders);
    }

    fn none(rule: &str, offenders: &[String]) {
        assert!(
            offenders.is_empty(),
            "{rule}: {} offenders\n  {}",
            offenders.len(),
            offenders.join("\n  ")
        );
    }

    #[test]
    fn a_command_has_a_tool_exactly_when_it_is_on_mcp() {
        let in_tools: Vec<CommandName> =
            tools().into_iter().flat_map(|tool| tool.commands).collect();
        let offenders: Vec<String> = CommandName::iter()
            .filter_map(|name| {
                let command = name.command();
                let on_mcp = command.surfaces.contains(&Surface::Mcp);
                (on_mcp != in_tools.contains(&name)).then(|| {
                    format!(
                        "{}: surfaces say MCP {on_mcp}, the tools say {}",
                        command.path(),
                        !on_mcp
                    )
                })
            })
            .collect();
        none("MCP tools", &offenders);
    }

    #[test]
    fn a_tool_property_is_an_mcp_name_of_the_table() {
        let offenders: Vec<String> = tools()
            .iter()
            .flat_map(|tool| {
                let schema = tool.schema(
                    &tool
                        .commands
                        .iter()
                        .map(|name| name.command())
                        .collect::<Vec<_>>(),
                );
                let listed: Vec<String> = schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|properties| properties.keys().cloned().collect())
                    .unwrap_or_default();
                let wanted: Vec<&str> = tool
                    .commands
                    .iter()
                    .flat_map(|name| name.command().arguments_on(Surface::Mcp))
                    .map(|argument| argument.name)
                    // `output` is the command line's `-o`, not a table argument.
                    .chain(
                        tool.commands
                            .iter()
                            .any(|name| crate::tools::output::on(*name))
                            .then_some("output"),
                    )
                    .collect();
                let name = tool.name.clone();
                let strange = listed
                    .iter()
                    .filter(|property| {
                        !property.chars().all(|each| {
                            each.is_ascii_lowercase()
                                || each.is_ascii_digit()
                                || each == '_'
                        })
                    })
                    .map(|property| format!("{name}: `{property}` is not snake case"))
                    .collect::<Vec<_>>();
                let outside = listed
                    .iter()
                    .filter(|property| {
                        property.as_str() != "action"
                            && !wanted.contains(&property.as_str())
                    })
                    .map(|property| {
                        format!("{name}: `{property}` is not an argument of the table")
                    })
                    .collect::<Vec<_>>();
                let absent = wanted
                    .iter()
                    .filter(|argument| {
                        !listed.iter().any(|property| property == **argument)
                    })
                    .map(|argument| {
                        format!(
                            "{name}: `{argument}` is in the table, not in the schema"
                        )
                    })
                    .collect::<Vec<_>>();
                [strange, outside, absent].concat()
            })
            .collect();
        none("MCP argument names", &offenders);
    }
}
