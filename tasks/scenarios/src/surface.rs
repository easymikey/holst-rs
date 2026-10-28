//! The two ways to reach Holst.

use serde_json::{Map, Value};

use crate::{holst::SendMode, observation::ErrorExit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Cli,
    Mcp,
}

impl Surface {
    /// Returns the signal of a refused or missing login, or of a change that
    /// needs a confirmation it did not get: exit code 77 on the CLI, an error
    /// reply from the tool.
    #[must_use]
    pub const fn permission_denied(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(77),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }

    /// Returns the signal of a request the user must change: exit code 64 on
    /// the CLI, an error reply from the tool.
    #[must_use]
    pub const fn usage(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(64),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }

    /// Returns the signal of data that cannot be read: exit code 65 on the CLI,
    /// an error reply from the tool.
    #[must_use]
    pub const fn data_error(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(65),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }

    /// Returns the signal of a board or name nobody has: exit code 66 on the
    /// CLI, an error reply from the tool.
    #[must_use]
    pub const fn not_found(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(66),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }

    /// Returns the signal of a file that is not what it should be: exit code
    /// 65 on the CLI, an error reply from the tool.
    #[must_use]
    pub const fn bad_data(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(65),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }

    /// Returns the signal of a server that refuses or cannot serve the request,
    /// as a board in a format this version cannot use: exit code 69 on the CLI,
    /// an error reply from the tool.
    #[must_use]
    pub const fn unavailable(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(69),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }

    /// Returns the signal of a new file or folder that is already there: exit
    /// code 73 on the CLI, an error reply from the tool.
    #[must_use]
    pub const fn already_there(self) -> ErrorExit {
        match self {
            Surface::Cli => ErrorExit::ExitCode(73),
            Surface::Mcp => ErrorExit::ToolError,
        }
    }
}

/// A command's other arguments, by MCP name and command-line text.
#[derive(Debug, Default)]
pub(crate) struct Words<'a> {
    pub(crate) positional: Vec<(&'a str, &'a str)>,
    pub(crate) options: Vec<(&'a str, &'a str)>,
    pub(crate) switches: Vec<&'a str>,
    /// The command removes something: a confirmed step confirms it.
    pub(crate) confirms: bool,
    /// The command changes something: a dry-running step only shows it.
    pub(crate) writes: bool,
}

/// Returns `words` as the arguments after the board on the command line and
/// as the fields of the MCP tool, with the confirmation and dry run `mode`
/// asks for.
pub(crate) fn arguments(
    words: &Words<'_>,
    mode: SendMode,
) -> (Vec<String>, Map<String, Value>) {
    let confirmed = words.confirms && mode == SendMode::Confirmed;
    let dry = words.writes && mode == SendMode::DryRun;
    let line = words
        .positional
        .iter()
        .map(|(_, word)| (*word).to_owned())
        .chain(
            words
                .options
                .iter()
                .flat_map(|(name, word)| [format!("--{name}"), (*word).to_owned()]),
        )
        .chain(
            words
                .switches
                .iter()
                .map(|name| format!("--{}", name.replace('_', "-"))),
        )
        .chain(confirmed.then(|| "--yes".to_owned()))
        .chain(dry.then(|| "--dry-run".to_owned()))
        .collect();
    let tool = words
        .positional
        .iter()
        .chain(&words.options)
        .map(|(name, word)| (*name, Value::from(*word)))
        .chain(words.switches.iter().map(|name| (*name, Value::Bool(true))))
        .chain(confirmed.then_some(("confirm", Value::Bool(true))))
        .chain(dry.then_some(("dry_run", Value::Bool(true))))
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    (line, tool)
}
