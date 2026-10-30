//! The `comment` tool: list, start, reply, resolve, delete, mark read.

use holst_core::{
    command::comment::{self, AddArgs, CommentReport, CommentVerb, EditArgs, LsArgs},
    domain::text::EMPTY,
    remote::client::Client,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::server::McpServer;

impl McpServer {
    pub(super) async fn comment_command(
        &self,
        verb: CommentVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self.expanded_references(arguments) {
            Ok(arguments) => self.run_comment_verb(verb, arguments).await,
            Err(err) => self.fail(&err),
        }
    }

    async fn run_comment_verb(
        &self,
        verb: CommentVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            CommentVerb::Ls => {
                self.parsed(arguments, async |args: LsArgs| {
                    self.run(async |client| {
                        let list = comment::ls(&client, &args).await?;
                        Ok(if list.threads.is_empty() {
                            format!("{EMPTY}\n")
                        } else {
                            list.lines().join("\n") + "\n"
                        })
                    })
                    .await
                })
                .await
            }
            CommentVerb::Add => {
                let add = async |client: &Client, args: &AddArgs| {
                    comment::add(client, &self.cache, args).await
                };
                self.change(arguments, add, Self::phrased).await
            }
            CommentVerb::Reply => {
                self.change(arguments, comment::reply, Self::phrased).await
            }
            CommentVerb::Resolve => {
                self.change(arguments, comment::resolve, Self::phrased)
                    .await
            }
            CommentVerb::Edit => {
                let edit = async |client: &Client, args: &EditArgs| {
                    comment::edit(client, &self.cache, args).await
                };
                self.change(arguments, edit, Self::phrased).await
            }
            CommentVerb::Read => {
                self.change(arguments, comment::read, Self::phrased).await
            }
            CommentVerb::Rm => self.change(arguments, comment::rm, Self::phrased).await,
        }
    }

    fn phrased(_: &Self, changed: &CommentReport) -> String {
        changed.phrase() + "\n"
    }
}
