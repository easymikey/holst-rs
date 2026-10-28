//! The `comment` tool: list, start, reply, resolve, delete, mark read.

use holst_core::{
    command::comment::{
        self,
        AddArgs,
        CommentReport,
        CommentVerb,
        LsArgs,
        ReadArgs,
        RemoveArgs,
        ReplyArgs,
        ResolveArgs,
    },
    domain::text::EMPTY,
    effect::board_write::WriteMode,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{login::HandedClient, server::McpServer, tools::said};

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
                self.parsed(arguments, async |args: AddArgs| {
                    self.sent(args.dry_run, async |client| {
                        comment::add(&client, &args).await
                    })
                    .await
                })
                .await
            }
            CommentVerb::Reply => {
                self.parsed(arguments, async |args: ReplyArgs| {
                    self.sent(args.dry_run, async |client| {
                        comment::reply(&client, &args).await
                    })
                    .await
                })
                .await
            }
            CommentVerb::Resolve => {
                self.parsed(arguments, async |args: ResolveArgs| {
                    self.sent(args.dry_run, async |client| {
                        comment::resolve(&client, &args).await
                    })
                    .await
                })
                .await
            }
            CommentVerb::Read => {
                self.parsed(arguments, async |args: ReadArgs| {
                    self.sent(args.dry_run, async |client| {
                        comment::read(&client, &args).await
                    })
                    .await
                })
                .await
            }
            CommentVerb::Rm => {
                self.parsed(arguments, async |args: RemoveArgs| {
                    self.remove_thread(&args).await
                })
                .await
            }
        }
    }

    /// Runs a change that needs no confirmation: a dry run only reads.
    async fn sent(
        &self,
        dry_run: WriteMode,
        change: impl AsyncFnOnce(HandedClient) -> holst_core::Result<CommentReport>,
    ) -> CallToolResult {
        let call =
            async |client: HandedClient| Ok(changed_text(&change(client).await?));
        self.by_mode(dry_run, call).await
    }

    async fn remove_thread(&self, args: &RemoveArgs) -> CallToolResult {
        let call = async |client: HandedClient| {
            let change = comment::rm(&client, args).await?;
            said(change.outcome.refusal(), || changed_text(&change))
        };
        self.by_mode(args.dry_run, call).await
    }
}

fn changed_text(changed: &CommentReport) -> String {
    changed.phrase() + "\n"
}
