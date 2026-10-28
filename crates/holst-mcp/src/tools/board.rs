//! The `board` and `board_write` tools: the tree of boards and folders.

use std::path::Path;

use holst_core::{
    command::{
        board::{
            BoardVerb,
            CopyArgs,
            CreateArgs,
            EditArgs,
            FavoriteArgs,
            GrantArgs,
            LeaveArgs,
            LsArgs,
            MoveArgs,
            RemoveArgs,
            RenameArgs,
            RestoreArgs,
            RevokeArgs,
            ShareArgs,
            TouchArgs,
            TransferArgs,
            UnfavoriteArgs,
            own,
            rights::{self, Settled, Shared},
            settings::{self, Edited},
            tree::{ListedNode, Listing, ls},
            tree_change::{self, Changed},
        },
        change::ChangeReport,
        table::{CommandArgs, CommandName, parse},
    },
    domain::{
        node::NodeKind,
        text::{EMPTY, joined, printable},
    },
    effect::board_write::WriteMode,
    remote::client::Client,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    login::HandedClient,
    server::McpServer,
    tools::{files::LocalFile, said},
};

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
                let dry = |args: &CreateArgs| args.dry_run;
                self.apply(arguments, dry, tree_change::create).await
            }
            BoardVerb::Rename => {
                let dry = |args: &RenameArgs| args.dry_run;
                self.apply(arguments, dry, tree_change::rename).await
            }
            BoardVerb::Mv => {
                let dry = |args: &MoveArgs| args.dry_run;
                self.apply(arguments, dry, tree_change::mv).await
            }
            BoardVerb::Cp => {
                let dry = |args: &CopyArgs| args.dry_run;
                self.apply(arguments, dry, tree_change::cp).await
            }
            BoardVerb::Rm => {
                let dry = |args: &RemoveArgs| args.dry_run;
                self.apply(arguments, dry, tree_change::rm).await
            }
            BoardVerb::Restore => {
                let dry = |args: &RestoreArgs| args.dry_run;
                self.apply(arguments, dry, tree_change::restore).await
            }
            BoardVerb::Favorite => {
                let dry = |args: &FavoriteArgs| args.dry_run;
                self.apply(arguments, dry, own::favorite).await
            }
            BoardVerb::Unfavorite => {
                let dry = |args: &UnfavoriteArgs| args.dry_run;
                self.apply(arguments, dry, own::unfavorite).await
            }
            BoardVerb::Touch => {
                let dry = |args: &TouchArgs| args.dry_run;
                self.apply(arguments, dry, own::touch).await
            }
            BoardVerb::Leave => {
                let dry = |args: &LeaveArgs| args.dry_run;
                self.apply(arguments, dry, own::leave).await
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
                let dry = |args: &GrantArgs| args.dry_run;
                self.apply_sharing(arguments, dry, rights::grant).await
            }
            BoardVerb::Revoke => {
                let dry = |args: &RevokeArgs| args.dry_run;
                self.apply_sharing(arguments, dry, rights::revoke).await
            }
            BoardVerb::Share => {
                let dry = |args: &ShareArgs| args.dry_run;
                self.apply_sharing(arguments, dry, rights::share).await
            }
            BoardVerb::Transfer => {
                let dry = |args: &TransferArgs| args.dry_run;
                self.apply_sharing(arguments, dry, rights::transfer).await
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
            Some(file) => LocalFile::check(Path::new(file), &self.secrets),
        };
        let call = async |client: HandedClient| {
            cover?;
            let edited = settings::edit(&client, &self.cache, args).await?;
            Ok(self.state.alias_text(&edited_text(self, &edited)))
        };
        self.by_mode(args.dry_run, call).await
    }

    async fn apply<A: CommandArgs>(
        &self,
        arguments: Map<String, Value>,
        dry_run: impl Fn(&A) -> WriteMode,
        change: impl AsyncFnOnce(&Client, &A) -> holst_core::Result<Changed>,
    ) -> CallToolResult {
        self.parsed(arguments, async |args: A| {
            let call = async |client: HandedClient| {
                let changed = change(&client, &args).await?;
                said(changed.outcome.refusal(), || self.changed_text(&changed))
            };
            self.by_mode(dry_run(&args), call).await
        })
        .await
    }

    /// As [`Self::apply`], for a change of who may use a board.
    async fn apply_sharing<A: CommandArgs>(
        &self,
        arguments: Map<String, Value>,
        dry_run: impl Fn(&A) -> WriteMode,
        change: impl AsyncFnOnce(&Client, &A) -> holst_core::Result<Shared>,
    ) -> CallToolResult {
        self.parsed(arguments, async |args: A| {
            let call = async |client: HandedClient| {
                let shared = change(&client, &args).await?;
                said(shared.refusal(), || self.shared_text(&shared))
            };
            self.by_mode(dry_run(&args), call).await
        })
        .await
    }

    fn tree_text(&self, listing: &Listing) -> String {
        if listing.nodes.is_empty() {
            format!("{EMPTY}\n")
        } else {
            let several = listing.workspaces.len() > 1;
            let before = std::iter::once(None)
                .chain(listing.nodes.iter().map(|node| node.workspace.as_deref()));
            let lines = listing.nodes.iter().zip(before).flat_map(|(node, before)| {
                let heading = node
                    .workspace
                    .as_deref()
                    .filter(|current| several && Some(*current) != before)
                    .and_then(|current| self.heading(listing, current));
                heading.into_iter().chain([self.node_line(node)])
            });
            let more = listing.cursor.as_deref().map(|cursor| {
                format!("{cursor} of {}\nmore: cursor {cursor}\n", listing.total)
            });
            lines.chain(more).collect()
        }
    }

    fn heading(&self, listing: &Listing, id: &str) -> Option<String> {
        listing
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .map(|workspace| {
                format!("{} {}\n", printable(&workspace.name), self.state.alias(id))
            })
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
