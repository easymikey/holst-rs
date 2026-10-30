//! The `board` and `board_write` tools: the tree of boards and folders.

use std::path::Path;

use holst_core::{
    command::{
        board::{
            BoardVerb,
            EditArgs,
            LsArgs,
            own,
            rights::{self, Settled, Shared},
            settings::{self, Edited},
            tree::{ListedNode, Listing, ls},
            tree_change::{self, Changed},
        },
        change::ChangeReport,
        table::{CommandName, parse},
    },
    domain::{
        node::NodeKind,
        text::{EMPTY, joined, printable},
    },
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{login::HandedClient, server::McpServer, tools::files::checked_upload};

/// The nodes a listing shows when the call gives no limit.
const LIST_DEFAULT: u32 = 300;

fn edited_text(server: &McpServer, edited: &Edited) -> String {
    let written = matches!(edited.outcome, ChangeReport::Sent);
    let place = written
        .then(|| format!("{} {}", server.state.alias(&edited.board), edited.link));
    let lines = [edited.phrase()].into_iter().chain(place);
    format!("{}\n", joined(lines, "\n"))
}

impl McpServer {
    pub(super) async fn board_command(
        &self,
        verb: BoardVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self.expanded_references(arguments) {
            Ok(expanded) => self.run_board_verb(verb, expanded).await,
            Err(err) => self.fail(&err),
        }
    }

    async fn run_board_verb(
        &self,
        verb: BoardVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            BoardVerb::Ls => self.list_boards(arguments).await,
            BoardVerb::Create => {
                self.change(arguments, tree_change::create, Self::changed_text)
                    .await
            }
            BoardVerb::Rename => {
                self.change(arguments, tree_change::rename, Self::changed_text)
                    .await
            }
            BoardVerb::Mv => {
                self.change(arguments, tree_change::mv, Self::changed_text)
                    .await
            }
            BoardVerb::Cp => {
                self.change(arguments, tree_change::cp, Self::changed_text)
                    .await
            }
            BoardVerb::Rm => {
                self.change(arguments, tree_change::rm, Self::changed_text)
                    .await
            }
            BoardVerb::Restore => {
                self.change(arguments, tree_change::restore, Self::changed_text)
                    .await
            }
            BoardVerb::Favorite => {
                self.change(arguments, own::favorite, Self::changed_text)
                    .await
            }
            BoardVerb::Unfavorite => {
                self.change(arguments, own::unfavorite, Self::changed_text)
                    .await
            }
            BoardVerb::Touch => {
                self.change(arguments, own::touch, Self::changed_text).await
            }
            BoardVerb::Leave => {
                self.change(arguments, own::leave, Self::changed_text).await
            }
            BoardVerb::Show => self.show_boards(arguments).await,
            BoardVerb::Snap => self.snap_board(arguments).await,
            BoardVerb::Save => self.save_board(arguments).await,
            BoardVerb::Export => self.export_board(arguments).await,
            BoardVerb::Import => self.import_board(arguments).await,
            BoardVerb::Edit => {
                self.parsed(arguments, async |args: EditArgs| {
                    self.edit_board(&args).await
                })
                .await
            }
            BoardVerb::Grant => {
                self.change(arguments, rights::grant, Self::shared_text)
                    .await
            }
            BoardVerb::Revoke => {
                self.change(arguments, rights::revoke, Self::shared_text)
                    .await
            }
            BoardVerb::Share => {
                self.change(arguments, rights::share, Self::shared_text)
                    .await
            }
            BoardVerb::Transfer => {
                self.change(arguments, rights::transfer, Self::shared_text)
                    .await
            }
        }
    }

    async fn list_boards(&self, mut arguments: Map<String, Value>) -> CallToolResult {
        let taken =
            super::output::take(&mut arguments, CommandName::Board(BoardVerb::Ls));
        let read = taken.and_then(|output| Ok((output, parse::<LsArgs>(arguments)?)));
        match read {
            Ok((output, mut args)) => {
                args.limit.get_or_insert(LIST_DEFAULT);
                self.run(async |client| {
                    let listing = ls(&client, &args).await?;
                    output.reply(&listing, || self.tree_text(&listing))
                })
                .await
            }
            Err(err) => self.fail(&err),
        }
    }

    /// Sets the cover and the note; the cover file is checked as an upload is.
    async fn edit_board(&self, args: &EditArgs) -> CallToolResult {
        let cover = match args.cover.as_deref() {
            None | Some("reset") => Ok(()),
            Some(file) => checked_upload(Path::new(file), &self.secrets),
        };
        let call = async |client: HandedClient| {
            cover?;
            let edited = settings::edit(&client, &self.cache, args).await?;
            Ok(self.state.alias_text(&edited_text(self, &edited)))
        };
        self.by_mode(args.dry_run, call).await
    }

    fn tree_text(&self, listing: &Listing) -> String {
        if listing.nodes.is_empty() {
            format!("{EMPTY}\n")
        } else {
            let lines = listing.sections().flat_map(|(heading, node)| {
                let heading = heading.map(|workspace| {
                    let alias = self.state.alias(&workspace.id);
                    format!("{} {alias}\n", printable(&workspace.name))
                });
                heading.into_iter().chain([self.node_line(node)])
            });
            let more = listing.cursor.as_deref().map(|cursor| {
                format!("{cursor} of {}\nmore: cursor {cursor}\n", listing.total)
            });
            lines.chain(more).collect()
        }
    }

    fn node_line(&self, node: &ListedNode) -> String {
        let name = if node.name.is_empty() {
            "(no name)".into()
        } else {
            printable(&node.name)
        };
        let slash = if node.kind == NodeKind::Folder.as_str() {
            "/"
        } else {
            ""
        };
        let trash = if node.in_trash { " [in trash]" } else { "" };
        format!(
            "{:indent$}{name}{slash} {}{trash}\n",
            "",
            self.state.alias(&node.id),
            indent = node.depth.saturating_mul(2)
        )
    }

    fn changed_text(&self, changed: &Changed) -> String {
        let node = &changed.node;
        match &changed.outcome {
            ChangeReport::DryRun { request } | ChangeReport::Held { request, .. } => {
                let lines = request.lines().join("\n");
                self.state
                    .alias_text(&format!("{}\n{lines}\n", node.phrase(true)))
            }
            ChangeReport::Sent => {
                let place: Vec<String> = node
                    .id
                    .as_deref()
                    .map(|id| self.state.alias(id))
                    .into_iter()
                    .chain(node.link.clone())
                    .collect();
                joined(
                    [node.phrase(false)]
                        .into_iter()
                        .chain((!place.is_empty()).then(|| place.join(" "))),
                    "\n",
                )
                .to_string()
                    + "\n"
            }
        }
    }

    fn shared_text(&self, shared: &Shared) -> String {
        let phrase = shared.phrase();
        match &shared.outcome {
            Settled::NotHeld { .. } => self.state.alias_text(&format!("{phrase}\n")),
            Settled::Carried(ChangeReport::Sent) => {
                let place = format!("{} {}", self.state.alias(&shared.id), shared.link);
                format!("{}\n", joined([phrase, place], "\n"))
            }
            Settled::Carried(
                ChangeReport::DryRun { .. } | ChangeReport::Held { .. },
            ) => {
                let lines = shared.plan_lines().join("\n");
                self.state.alias_text(&format!("{phrase}\n{lines}\n"))
            }
        }
    }
}
