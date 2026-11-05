//! What can go wrong in running a scenario.

use std::{path::PathBuf, sync::Arc};

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("the temporary home or the fake server failed")]
    Io(#[from] std::io::Error),
    #[error("{} is not there after building `holst-cli` and `holst-mcp`", path.display())]
    MissingBinary { path: PathBuf },
    #[error("building `holst-cli` and `holst-mcp` for the scenarios failed")]
    Build(#[from] BuildError),
    #[error("the binary printed something that is not JSON")]
    Json(#[from] serde_json::Error),
    #[error("the MCP server did not start")]
    Start(#[source] Box<rmcp::service::ClientInitializeError>),
    #[error("the MCP call failed")]
    Call(#[source] Box<rmcp::ServiceError>),
    #[error("the MCP client did not shut down")]
    Shutdown(#[from] tokio::task::JoinError),
    #[error("the fake Holst holds no board called {name:?}")]
    NoBoard { name: String },
    #[error("the fake Holst holds no object of kind {kind:?} on the board {board:?}")]
    NoObjectOfKind { board: String, kind: String },
    #[error("the fake Holst had no answer for: {requests:?}")]
    Unanswered { requests: Vec<String> },
}

impl From<rmcp::service::ClientInitializeError> for ScenarioError {
    fn from(error: rmcp::service::ClientInitializeError) -> Self {
        Self::Start(Box::new(error))
    }
}

impl From<rmcp::ServiceError> for ScenarioError {
    fn from(error: rmcp::ServiceError) -> Self {
        Self::Call(Box::new(error))
    }
}

/// Why the binaries could not be built; cloneable, as the one build is shared.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildError {
    #[error("cargo did not run")]
    Spawn(#[source] Arc<std::io::Error>),
    #[error("{stderr}")]
    Failed { stderr: String },
}

/// What can go wrong in the `holst-eval` command line.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error(
        "usage: holst-eval oracle | serve | agent --command '<agent command with {{prompt}} and {{config}}>' [--runs 3] --out <dir>"
    )]
    Usage,
    #[error(transparent)]
    Scenario(#[from] ScenarioError),
    #[error("`--runs` is not a number")]
    Runs(#[source] std::num::ParseIntError),
    #[error("{0} tasks not proven")]
    Unproven(usize),
    #[error(transparent)]
    Signal(std::io::Error),
}

/// Why a check refuses a run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Mismatch {
    #[error("fixture object {number} is gone")]
    FixtureGone { number: u32 },
    #[error("expected one new {kind} on {board}, found {found}")]
    NotOneAdded {
        kind: String,
        board: String,
        found: Value,
    },
    #[error("the board {name} changed")]
    BoardChanged { name: String },
    #[error("the boards were renamed or moved")]
    BoardsRenamed,
    #[error("the comment threads changed")]
    ThreadsChanged,
    #[error("expected {expected}, found {found}")]
    Unexpected {
        expected: &'static str,
        found: Value,
    },
    #[error("{tool} {arguments}: {text}")]
    CallRefused {
        tool: &'static str,
        arguments: Value,
        text: String,
    },
}
