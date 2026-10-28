//! What can go wrong in running a scenario.

use std::{path::PathBuf, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("the temporary home or the fake server failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{} is not there after building `holst-cli` and `holst-mcp`", path.display())]
    MissingBinary { path: PathBuf },
    #[error("building `holst-cli` and `holst-mcp` for the scenarios failed: {0}")]
    Build(#[from] BuildError),
    #[error("the binary printed something that is not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the MCP server did not start: {0}")]
    Start(Box<rmcp::service::ClientInitializeError>),
    #[error("the MCP call failed: {0}")]
    Call(Box<rmcp::ServiceError>),
    #[error("the MCP client did not shut down: {0}")]
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
    #[error("cargo did not run: {0}")]
    Spawn(Arc<std::io::Error>),
    #[error("{0}")]
    Failed(String),
}
