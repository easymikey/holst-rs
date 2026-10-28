//! The tools: the table's commands.

pub(crate) mod admin;
mod auth;
mod backup;
mod board;
pub(crate) mod board_load;
mod board_whole;
mod comment;
pub(crate) mod files;
pub(crate) mod input;
mod object;
pub(crate) mod output;
mod picture;
mod search;
mod template;
mod workspace;

use holst_core::{
    command::table::CommandName,
    effect::board_write::WriteMode,
    error::confirmation::Unconfirmed,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::HandedClient,
    server::McpServer,
};

/// The reply of a change: its text, or the refusal that names `confirm`.
pub(super) fn said(
    refusal: Option<&Unconfirmed>,
    text: impl FnOnce() -> String,
) -> Result<String> {
    refusal.map_or_else(|| Ok(text()), |r| Err(Error::Unconfirmed(r.clone())))
}

impl McpServer {
    /// Runs a change: a dry run reads only, every send drops what the server
    /// remembers (a held change costs one extra read, never a stale cache).
    pub(super) async fn by_mode(
        &self,
        mode: WriteMode,
        call: impl AsyncFnOnce(HandedClient) -> Result<String>,
    ) -> CallToolResult {
        match mode {
            WriteMode::DryRun => self.run(call).await,
            WriteMode::Send => self.write(call).await,
        }
    }

    /// Runs one command of the table; the match grows by a line per group.
    pub(crate) async fn command(
        &self,
        name: CommandName,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match name {
            CommandName::Auth(verb) => self.auth(verb, arguments).await,
            CommandName::Search => Box::pin(self.search(arguments)).await,
            CommandName::Comment(verb) => {
                Box::pin(self.comment_command(verb, arguments)).await
            }
            CommandName::Workspace(verb) => {
                self.workspace_command(verb, arguments).await
            }
            CommandName::Board(verb) => {
                Box::pin(self.board_command(verb, arguments)).await
            }
            CommandName::Template(verb) => {
                Box::pin(self.template_command(verb, arguments)).await
            }
            CommandName::Backup(verb) => self.backup_command(verb, arguments).await,
            // `storage` is for the CLI; the table gives it no tool.
            CommandName::Storage(verb) => self.fail(&Error::CliOnly {
                command: verb.command().path(),
            }),
            CommandName::Api => self.api_command(arguments).await,
            CommandName::Object(verb) => {
                Box::pin(self.object_command(verb, arguments)).await
            }
        }
    }
}

#[cfg(test)] mod tests;
