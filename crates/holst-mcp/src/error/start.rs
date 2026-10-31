//! What can go wrong in starting `holst-mcp`.

use std::io;

use rmcp::service::ServerInitializeError;

use super::{HttpError, SetupError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum StartError {
    #[error(
        "unknown argument {arg}: the server speaks MCP on stdio; its commands are setup and --http"
    )]
    UnknownArg { arg: String },

    #[error(
        "unknown setup flag {flag}: setup takes --no-connect, --connect-only or --clients <list>"
    )]
    SetupFlag { flag: String },

    #[error("--clients needs a list such as claude,codex")]
    ClientsList,

    #[error("--clients names clients to connect, which --no-connect skips")]
    ClientsWithoutConnect,

    #[error(
        "stdin is not a terminal: run holst-mcp setup in a terminal \
         (in a container: docker exec -it <container> holst-mcp setup)"
    )]
    NotTerminal,

    #[error("give setup one of --no-connect and --connect-only, once")]
    OneFlag,

    #[error(
        "could not find the path of the running holst-mcp (it must be valid UTF-8)"
    )]
    Binary,

    #[error(transparent)]
    Http(#[from] HttpError),

    #[error("setup did not finish")]
    Setup(#[source] SetupError),

    #[error("could not write to stdout")]
    Stdout(#[source] io::Error),

    #[error("the server settings in the environment are not valid")]
    Settings(#[source] holst_core::Error),

    #[error("could not start the server on stdio")]
    Serve(#[source] Box<ServerInitializeError>),

    #[error("the server stopped with an error")]
    Stopped(#[source] tokio::task::JoinError),
}
