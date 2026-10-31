//! What can go wrong in a call to an MCP tool before the command runs.

use std::fmt;

use holst_core::domain::text::joined;
use serde_json::Value;

/// The actions of a tool, as the model is told them.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Verbs(pub(crate) Vec<&'static str>);

impl fmt::Display for Verbs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        joined(&self.0, ", ").fmt(f)
    }
}

/// Why a call to a table tool names no command, in words for the model.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CallError {
    #[error("`{tool}` needs `action`: {actions}")]
    NoAction { tool: String, actions: Verbs },

    #[error("`{tool}` has no action {action}; it has: {actions}")]
    NotAction {
        tool: String,
        action: Value,
        actions: Verbs,
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{alias} is not from this session (did the server restart?): take an @N from a fresh \
     search or list"
)]
pub(crate) struct UnknownAlias {
    pub(crate) alias: String,
}
