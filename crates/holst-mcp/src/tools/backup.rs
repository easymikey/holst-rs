//! The `backup` tool: a board's backups, listed, saved to a file and restored.

use holst_core::{
    command::{
        backup::{self, BackupVerb, GetArgs, LsArgs, RestoreArgs},
        table::NewFile,
    },
    domain::{
        backup::Backups,
        text::{EMPTY, printable},
    },
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    login::HandedClient,
    server::McpServer,
    tools::{
        files::{checked_download, out_file},
        said,
    },
};

/// The backups a listing shows, newest first as the server sends them.
const LIST_LIMIT: usize = 50;

impl McpServer {
    pub(super) async fn backup_command(
        &self,
        verb: BackupVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self.expanded_references(arguments) {
            Ok(arguments) => self.run_backup_verb(verb, arguments).await,
            Err(err) => self.fail(&err),
        }
    }

    async fn run_backup_verb(
        &self,
        verb: BackupVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            BackupVerb::Ls => {
                self.parsed(arguments, async |args: LsArgs| {
                    self.run(async |client| {
                        Ok(listing_text(&backup::ls(&client, &args).await?))
                    })
                    .await
                })
                .await
            }
            BackupVerb::Get => {
                self.parsed(arguments, async |args: GetArgs| {
                    self.save_backup(args).await
                })
                .await
            }
            BackupVerb::Restore => {
                self.parsed(arguments, async |args: RestoreArgs| {
                    self.restore_backup(&args).await
                })
                .await
            }
        }
    }

    /// Saves a backup into a new file, checked as a download is.
    async fn save_backup(&self, mut args: GetArgs) -> CallToolResult {
        let file = match args.file.take() {
            Some(file) => out_file(&file.0, "backup.ydoc", &self.secrets).map(Some),
            None => checked_download(None, self.downloads.as_deref(), &self.secrets)
                .map(|()| None),
        };
        match file {
            Err(err) => self.fail(&err),
            Ok(path) => {
                args.file = path.map(NewFile);
                self.run(async |client| {
                    let fetched =
                        backup::get(&client, self.downloads.as_deref(), &args).await?;
                    Ok(format!("{fetched}\n"))
                })
                .await
            }
        }
    }

    async fn restore_backup(&self, args: &RestoreArgs) -> CallToolResult {
        let call = async |client: HandedClient| {
            let restored = backup::restore(&client, args).await?;
            said(restored.outcome.refusal(), || {
                self.state.alias_text(&format!("{restored}\n"))
            })
        };
        self.by_mode(args.dry_run, call).await
    }
}

fn listing_text(backups: &Backups) -> String {
    match backups.as_slice() {
        [] => format!("{EMPTY}\n"),
        all => {
            let lines = all
                .iter()
                .take(LIST_LIMIT)
                .map(|each| format!("{}\n", printable(&each.line())));
            let more = (all.len() > LIST_LIMIT)
                .then(|| format!("{LIST_LIMIT} of {}\n", all.len()));
            lines.chain(more).collect()
        }
    }
}
