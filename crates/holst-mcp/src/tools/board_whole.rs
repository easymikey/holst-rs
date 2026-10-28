//! A whole board, or a part of it: show, snap, save, export and import.

use std::path::Path;

use base64::Engine as _;
use holst_core::{
    command::{
        board::{
            BoardVerb,
            ExportArgs,
            ImportArgs,
            PictureFormat,
            SaveArgs,
            ShowArgs,
            SnapArgs,
            file::{self, Exported, Imported, STDOUT},
            picture::{self, Delivery, Saved, Snapped},
            show::{self, BoardBrief, Briefs},
        },
        change::{ChangeReport, PlannedRequest},
        table::{CommandName, parse},
    },
    domain::{picture::MAX_SIDE_MODEL, text::printable},
};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::HandedClient,
    reply::objects,
    server::McpServer,
    tools::{
        files::{LocalFile, checked_download, checked_place},
        picture::inline_svg,
        said,
    },
};

impl McpServer {
    pub(super) async fn show_boards(
        &self,
        mut arguments: Map<String, Value>,
    ) -> CallToolResult {
        let taken =
            super::output::take(&mut arguments, CommandName::Board(BoardVerb::Show));
        let read = taken.and_then(|output| Ok((output, parse::<ShowArgs>(arguments)?)));
        match read {
            Ok((output, args)) => {
                self.run(async |client| {
                    let shown = show::show(&client, &self.cache, &args).await?;
                    output.reply(&shown, || self.boards_text(&shown))
                })
                .await
            }
            Err(err) => self.fail(&err),
        }
    }

    pub(super) async fn snap_board(
        &self,
        mut arguments: Map<String, Value>,
    ) -> CallToolResult {
        if arguments.get("max_side").is_none_or(Value::is_null) {
            // A whole float displays as its digits; no cast needed.
            let side: u32 =
                MAX_SIDE_MODEL.get().to_string().parse().unwrap_or_default();
            arguments.insert("max_side".to_owned(), Value::from(side));
        }
        match parse::<SnapArgs>(arguments) {
            Ok(args) => {
                self.run_blocks(async |client| {
                    let snapped =
                        picture::snap(&client, &self.cache, None, &args).await?;
                    Ok(snapped_blocks(snapped))
                })
                .await
            }
            Err(err) => self.fail(&err.into()),
        }
    }

    pub(super) async fn save_board(
        &self,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match parse::<SaveArgs>(arguments) {
            Ok(args) => match self.named_place(args.dir.as_deref()) {
                Ok(()) => {
                    self.run(async |client| {
                        let saved = picture::save(
                            &client,
                            &self.cache,
                            self.downloads.as_deref(),
                            &args,
                        )
                        .await?;
                        Ok(Self::saved_text(&saved))
                    })
                    .await
                }
                Err(err) => self.fail(&err),
            },
            Err(err) => self.fail(&err.into()),
        }
    }

    pub(super) async fn export_board(
        &self,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match parse::<ExportArgs>(arguments) {
            Ok(args) => match self.named_place(args.file.as_deref()) {
                Ok(()) => {
                    self.run(async |client| {
                        let exported =
                            file::export(&client, self.downloads.as_deref(), &args)
                                .await?;
                        Ok(Self::exported_text(&exported))
                    })
                    .await
                }
                Err(err) => self.fail(&err),
            },
            Err(err) => self.fail(&err.into()),
        }
    }

    pub(super) async fn import_board(
        &self,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match parse::<ImportArgs>(arguments) {
            Ok(args) => match LocalFile::check(Path::new(&args.file), &self.secrets) {
                Ok(()) => {
                    let call = async |client: HandedClient| {
                        let imported = file::import(&client, &args).await?;
                        said(imported.outcome.refusal(), || {
                            self.imported_text(&imported)
                        })
                    };
                    self.by_mode(args.dry_run, call).await
                }
                Err(err) => self.fail(&err),
            },
            Err(err) => self.fail(&err.into()),
        }
    }

    /// A place a call names for a new file or folder: not the command line's
    /// stdout, and not where the keys or the login are. With none, the
    /// downloads folder, which is held to the same.
    fn named_place(&self, place: Option<&str>) -> Result<()> {
        match place {
            None => checked_download(None, self.downloads.as_deref(), &self.secrets),
            Some(STDOUT) => Err(Error::NoArgument {
                what: "a path: - is for the command line",
            }),
            Some(place) => checked_place(place, &self.secrets),
        }
    }

    fn boards_text(&self, shown: &Briefs) -> String {
        let boards = shown.boards.iter().map(|board| self.board_text(board));
        let missing = shown.missing.iter().map(|gone| {
            self.state
                .alias_text(&format!("not found: {}\n", printable(&gone.reason)))
        });
        boards.chain(missing).collect()
    }

    fn board_text(&self, board: &BoardBrief) -> String {
        let place = [board.workspace.as_deref(), board.folder.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" / ");
        let trash = if board.in_trash { " · in trash" } else { "" };
        let kinds = board
            .objects
            .iter()
            .map(|(kind, count)| format!("{kind} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let counts = if kinds.is_empty() {
            format!("{} objects", board.count)
        } else {
            format!("{} objects: {kinds}", board.count)
        };
        let rectangle = board
            .rectangle
            .map(|rect| format!("rectangle {}\n", objects::rounded(&rect)))
            .unwrap_or_default();
        let parts = [
            (
                "members",
                board
                    .members
                    .as_ref()
                    .and_then(|users| serde_json::to_string(users).ok()),
            ),
            (
                "rights",
                board
                    .rights
                    .as_ref()
                    .and_then(|users| serde_json::to_string(users).ok()),
            ),
            (
                "sharing",
                board
                    .sharing
                    .as_ref()
                    .and_then(|sharing| serde_json::to_string(sharing).ok()),
            ),
        ]
        .into_iter()
        .filter_map(|(name, text)| text.map(|text| format!("{name}: {text}\n")))
        .collect::<String>();
        let head = format!(
            "{} {} · {} · {place}{trash}\n{}\n{counts}\n{rectangle}{parts}",
            printable(&board.name),
            self.state.alias(&board.id),
            board.edit_right.as_str(),
            board.link,
        );
        // The parts name users by id.
        self.state.alias_text(&head) + "\n"
    }

    fn saved_text(saved: &Saved) -> String {
        let files = saved
            .files
            .iter()
            .map(|file| format!("{} {} bytes · {}", file.name, file.bytes, file.about));
        let originals: usize = saved.images.iter().map(|image| image.bytes).sum();
        let images =
            format!("images/ {} files · {originals} bytes", saved.images.len());
        let not_saved = saved
            .not_saved
            .iter()
            .map(|reason| format!("not saved: {reason}"));
        let lines = std::iter::once(format!("saved {}/", saved.dir.display()))
            .chain(files)
            .chain([images])
            .chain(not_saved)
            .collect::<Vec<_>>()
            .join("\n");
        format!("{}\n", printable(&lines))
    }

    fn exported_text(exported: &Exported) -> String {
        let (objects, bytes) = (exported.objects, exported.bytes);
        let before = if exported.state_bytes == bytes {
            String::new()
        } else {
            format!(" ({} before gzip)", exported.state_bytes)
        };
        let listing = exported.object_list.iter().map(|list| {
            format!("\nlisting: {} bytes: {}", list.bytes, list.file.display())
        });
        let text = format!(
            "exported {objects} objects, {bytes} bytes{before}: {}{}",
            exported.file.display(),
            listing.collect::<String>()
        );
        format!("{}\n", printable(&text))
    }

    fn imported_text(&self, imported: &Imported) -> String {
        let (verb, request, link) = match &imported.outcome {
            ChangeReport::Sent => ("imported", Vec::new(), imported.link.clone()),
            ChangeReport::DryRun { request } => (
                "would import",
                request.iter().flat_map(PlannedRequest::lines).collect(),
                None,
            ),
            ChangeReport::Held { .. } => ("would import", Vec::new(), None),
        };
        let board = imported.board.as_deref().map(|id| self.state.alias(id));
        let head = format!(
            "{verb} {} objects into {} / {}{}; the board had {}, now {}",
            imported.saved_objects,
            imported.workspace_name,
            imported.name,
            board.map(|alias| format!(" {alias}")).unwrap_or_default(),
            imported.objects_before,
            imported.objects_after
        );
        let receipt = imported.receipt.map(|receipt| receipt.to_string());
        let lines = std::iter::once(head)
            .chain(request)
            .chain(receipt)
            .chain(link)
            .collect::<Vec<_>>()
            .join("\n");
        self.state.alias_text(&format!("{}\n", printable(&lines)))
    }
}

/// The picture, then the words about it.
fn snapped_blocks(snapped: Snapped) -> Vec<ContentBlock> {
    let warning = snapped
        .warning
        .as_deref()
        .map(|warning| format!("warning: {warning}\n"))
        .unwrap_or_default();
    let about = format!(
        "rectangle {} · {} objects · {}x{} px · {}\n{warning}",
        objects::rounded(&snapped.rectangle),
        snapped.objects,
        snapped.width,
        snapped.height,
        snapped.link
    );
    let picture = match (snapped.delivery, snapped.format) {
        (Delivery::Inline { bytes }, PictureFormat::Png) => ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            "image/png",
        ),
        (Delivery::Inline { bytes }, PictureFormat::Svg) => {
            ContentBlock::text(inline_svg(String::from_utf8_lossy(&bytes).into_owned()))
        }
        (Delivery::Written { file, .. }, _) => {
            ContentBlock::text(format!("saved to {}", file.display()))
        }
    };
    vec![picture, ContentBlock::text(about)]
}
