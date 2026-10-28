//! The `backup` tool: a board's backups, listed, saved to a file and restored.

use holst_core::{
    command::{
        backup::{self, BackupVerb, Fetched, GetArgs, LsArgs, RestoreArgs, Restored},
        board::picture::Delivery,
        change::ChangeReport,
        table::NewFile,
    },
    domain::{
        backup::Backups,
        text::{EMPTY, joined, printable},
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
                    Ok(fetched_text(&fetched))
                })
                .await
            }
        }
    }

    async fn restore_backup(&self, args: &RestoreArgs) -> CallToolResult {
        let call = async |client: HandedClient| {
            let restored = backup::restore(&client, args).await?;
            said(restored.outcome.refusal(), || {
                self.state.alias_text(&restored_text(&restored))
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

fn fetched_text(fetched: &Fetched) -> String {
    match &fetched.delivery {
        Delivery::Written { file, bytes } => {
            let before = if fetched.uncompressed_bytes == *bytes {
                String::new()
            } else {
                format!(" ({} before gzip)", fetched.uncompressed_bytes)
            };
            format!("saved {bytes} bytes{before} to {}\n", file.display())
        }
        Delivery::Inline { bytes } => format!("{} bytes\n", bytes.len()),
    }
}

fn restored_text(restored: &Restored) -> String {
    let written = matches!(restored.outcome, ChangeReport::Sent);
    let receipt = restored
        .receipt
        .filter(|_| written)
        .map(|each| each.to_string());
    let link = written.then(|| restored.link.clone());
    let lines = [restored.phrase()].into_iter().chain(receipt).chain(link);
    format!("{}\n", joined(lines, "\n"))
}
