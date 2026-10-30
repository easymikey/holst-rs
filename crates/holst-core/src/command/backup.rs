//! The `backup` group: a board's snapshots, listed, downloaded and restored.

use std::{fmt, path::Path};

use holst_macros::CommandArgs;
use serde::{Deserialize, Serialize};

use crate::{
    command::{
        board::{file::ImportReport, picture::Delivery},
        board_file,
        change::{ChangeReport, gate},
        lookup,
        table::{Command, CommandArgs, Confirmation, NewFile},
    },
    domain::{
        backup::{Backup, Backups, DEFAULT_PROVIDER, ListedBackup, OTHER_PROVIDER},
        board_file::{
            compression::{self, Compression},
            document,
        },
        change::receipt::UpdateReceipt,
        text::joined,
    },
    effect::{board_write::WriteMode, socket::SocketConnector},
    error::{
        Error,
        Result,
        board_socket::BoardSocketError,
        confirmation::{ChangeRisk, Unconfirmed},
        http::{HTML_BODY, HttpError},
        input::InputError,
        lookup::{LookupError, LookupKind, LookupMiss},
    },
    remote::{api, client::Client, socket::write::connector},
    storage::file::{create_dir_all, write_new, write_numbered},
};

/// The verbs of `holst backup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum BackupVerb {
    Ls,
    Get,
    Restore,
}

impl BackupVerb {
    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            BackupVerb::Ls => &LsArgs::COMMAND,
            BackupVerb::Get => &GetArgs::COMMAND,
            BackupVerb::Restore => &RestoreArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Backup,
    verb = "ls",
    risk = Read,
    block = Commands,
    about = "List a board's backups with keys and dates",
    help = "List a board's backups with their keys and dates.\n\
            Use a key with `backup get` or `backup restore`.",
    example = "holst backup ls \"Roadmap\"",
    example = "holst backup ls \"Roadmap\" -o json"
)]
pub struct LsArgs {
    #[argument(positional, help = "The board whose backups to list")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Backup,
    verb = "get",
    risk = Write,
    block = Commands,
    local_files,
    about = "Download a backup by its key",
    help = "Download a backup by its key from `backup ls`; a gzip board file.\n\
            The file goes to ~/Downloads unless a file is given; `holst b import` reads it.",
    example = "holst backup get 1761552000",
    example = "holst backup get 1761552000 roadmap.ydoc --uncompressed"
)]
pub struct GetArgs {
    #[argument(positional, help = "The backup's key from `holst backup ls`")]
    pub key: String,
    #[argument(
        positional,
        value_name = "file",
        help = "A new file, never overwritten, - for stdout; ~/Downloads by default"
    )]
    pub file: Option<NewFile>,
    #[argument(
        value_name = "provider",
        help = "Where the server keeps the backup, as `backup ls` shows; the usual place by default"
    )]
    pub provider: Option<String>,
    #[argument(help = "Save without gzip, which adds .gz to the file name")]
    pub uncompressed: bool,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Backup,
    verb = "restore",
    risk = Destroy,
    block = Commands,
    about = "Write a backup back into its board",
    help = "Write a backup back into its board.\n\
            It replaces what is on the board now, so it needs -y; without it only the dry run is shown.",
    example = "holst backup restore \"Roadmap\" 1761552000 -y",
    example = "holst backup restore \"Roadmap\" 1761552000 -n"
)]
pub struct RestoreArgs {
    #[argument(positional, help = "The board to write the backup into")]
    pub board: String,
    #[argument(positional, help = "The backup's key from `holst backup ls`")]
    pub key: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; a terminal asks instead; without one only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// Lists the snapshots of a board, newest first as the server sends them.
///
/// # Errors
/// A board that is not found, and errors of the request.
pub async fn ls(client: &Client, args: &LsArgs) -> Result<Backups> {
    let id = lookup::board_id(client, &args.board, args.workspace.as_deref()).await?;
    api::backup::list(client, &id).await
}

/// A backup saved or returned, as asked.
#[derive(Debug, Serialize)]
pub struct Fetched {
    pub key: String,
    /// The size of the snapshot before compression.
    pub uncompressed_bytes: usize,
    #[serde(flatten)]
    pub delivery: Delivery,
}

impl fmt::Display for Fetched {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.delivery {
            Delivery::Written { file, bytes } => {
                let before = match self.uncompressed_bytes {
                    size if size == *bytes => String::new(),
                    size => format!(" ({size} before gzip)"),
                };
                write!(f, "saved {bytes} bytes{before} to {}", file.display())
            }
            Delivery::Inline { bytes } => write!(f, "{} bytes", bytes.len()),
        }
    }
}

/// Downloads a backup and packs it: gzip unless `uncompressed`. A given file
/// is named as `compression::file_name` says; with none, the file is
/// `backup-<key>.ydoc[.gz]` in `downloads`, and with no `downloads` either the
/// bytes come back inline. The file is new and never replaces one.
///
/// # Errors
/// A backup the server does not have, a file that exists, and errors of the
/// request and of the packing.
pub async fn get(
    client: &Client,
    downloads: Option<&Path>,
    args: &GetArgs,
) -> Result<Fetched> {
    let (packing, extension) = if args.uncompressed {
        (Compression::Plain, "ydoc")
    } else {
        (Compression::Gzip, "ydoc.gz")
    };
    let file = args
        .file
        .as_ref()
        .map(|file| compression::file_name(&file.0, packing))
        .transpose()?;
    let provider = match args.provider.as_deref() {
        Some(given) => known_provider("backup get", given)?,
        None => DEFAULT_PROVIDER,
    };
    let backup = download(client, &args.key, provider, packing).await?;
    let delivery = match (file, downloads) {
        (Some(file), _) => {
            write_new(&file, &backup.bytes)?;
            Delivery::Written {
                file,
                bytes: backup.bytes.len(),
            }
        }
        (None, Some(dir)) => {
            create_dir_all(dir)?;
            let stem = format!("backup-{}", file_part(&args.key));
            Delivery::Written {
                file: write_numbered(dir, &stem, extension, &backup.bytes)?,
                bytes: backup.bytes.len(),
            }
        }
        (None, None) => Delivery::Inline {
            bytes: backup.bytes,
        },
    };
    Ok(Fetched {
        key: args.key.clone(),
        uncompressed_bytes: backup.uncompressed_len,
        delivery,
    })
}

fn file_part(key: &str) -> String {
    key.chars()
        .map(|each| {
            if each.is_ascii_alphanumeric() || matches!(each, '-' | '_' | '.') {
                each
            } else {
                '_'
            }
        })
        .collect()
}

/// A backup written into its board, or in a dry run only planned.
#[derive(Debug, Serialize)]
pub struct Restored {
    pub board: String,
    pub name: String,
    pub workspace: String,
    pub workspace_name: String,
    /// Where the board opens.
    pub link: String,
    /// The backup, as the list shows it.
    pub backup: ListedBackup,
    /// The objects the backup holds.
    pub saved_objects: usize,
    pub objects_before: usize,
    pub objects_after: usize,
    #[serde(flatten)]
    pub receipt: Option<UpdateReceipt>,
    #[serde(flatten)]
    pub outcome: ImportReport,
}

impl fmt::Display for Restored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let written = matches!(self.outcome, ChangeReport::Sent);
        let receipt = self
            .receipt
            .filter(|_| written)
            .map(|each| each.to_string());
        let link = written.then(|| self.link.clone());
        let lines = [self.phrase()].into_iter().chain(receipt).chain(link);
        write!(f, "{}", joined(lines, "\n"))
    }
}

impl Restored {
    /// What the restore did, or would do, in a line.
    #[must_use]
    pub fn phrase(&self) -> String {
        let backup = self.backup.line();
        match &self.outcome {
            ChangeReport::Sent => format!(
                "restored {} to the backup {backup}: {} objects, were {}",
                self.name, self.objects_after, self.objects_before
            ),
            ChangeReport::DryRun { .. } | ChangeReport::Held { .. } => format!(
                "would restore {} to the backup {backup}: {} objects replace {}",
                self.name, self.saved_objects, self.objects_before
            ),
        }
    }
}

/// Returns a board to one of its backups: every part of the document, the
/// objects with their texts, the stores, the note and the comments, becomes
/// what the backup holds, and what the backup lacks is removed. It needs the
/// confirmation; a dry run and a missing confirmation change nothing and show
/// the backup and the object counts.
///
/// # Errors
/// A board or backup that is not found, a board that is read only, a backup
/// that is not a board file, [`BoardSocketError::UnsupportedFormat`] for a
/// backup of another provider than `hud` (stored in Yjs version 1, which this
/// version does not write; nothing is downloaded), and errors of the
/// requests.
pub async fn restore(client: &Client, args: &RestoreArgs) -> Result<Restored> {
    restored(client, connector(), args).await
}

async fn restored(
    client: &Client,
    socket: &dyn SocketConnector,
    args: &RestoreArgs,
) -> Result<Restored> {
    let target =
        lookup::exact_board(client, &args.board, args.workspace.as_deref()).await?;
    let listed = api::backup::list(client, &target.id).await?;
    let backup = listed
        .as_slice()
        .iter()
        .find(|each| each.key.as_deref() == Some(args.key.as_str()))
        .cloned()
        .ok_or_else(|| missing(&args.key))?;
    let provider = backup.download_provider();
    only_v2(provider)?;
    let snapshot = fetch(client, &args.key, provider).await?;
    let state = compression::uncompressed(snapshot.as_bytes())?;
    document::saved_objects(&state)?;
    let unconfirmed = Unconfirmed::new(
        "replacing the objects of a board with a backup",
        ChangeRisk::Irreversible,
    );
    let gated = gate(Some(unconfirmed), (args.dry_run, args.confirm), || None);
    let done = board_file::replace_with(
        client,
        socket,
        &target.id,
        &state,
        gated
            .as_ref()
            .err()
            .map_or(WriteMode::Send, |_| WriteMode::DryRun),
    )
    .await?;
    let outcome = gated.err().unwrap_or(ChangeReport::Sent);
    Ok(Restored {
        link: client.board_link(&target.id),
        board: target.id,
        name: target.name,
        workspace: target.workspace.id,
        workspace_name: target.workspace.name,
        backup,
        saved_objects: done.plan.saved,
        objects_before: done.plan.before,
        objects_after: done.plan.after,
        receipt: Some(done.receipt()),
        outcome,
    })
}

/// The provider as the server names them: `hud`, or `hocuspocus` for the
/// older boards.
fn known_provider<'a>(command: &str, given: &'a str) -> Result<&'a str> {
    match given {
        DEFAULT_PROVIDER | OTHER_PROVIDER => Ok(given),
        _ => Err(provider_error(
            command,
            format!("{DEFAULT_PROVIDER} or {OTHER_PROVIDER}"),
        )),
    }
}

fn provider_error(command: &str, expected: String) -> Error {
    InputError::NotArgumentValue {
        command: command.to_owned(),
        name: "provider",
        expected,
    }
    .into()
}

/// Only `hud` keeps Yjs v2, the format this version reads and writes.
fn only_v2(provider: &str) -> Result<()> {
    match provider {
        DEFAULT_PROVIDER => Ok(()),
        other => Err(Error::BoardSocket(BoardSocketError::UnsupportedFormat {
            schema: 1,
            provider: other.to_owned(),
        })),
    }
}

fn missing(key: &str) -> Error {
    Error::Lookup(LookupError {
        kind: LookupKind::Backup,
        reference: key.to_owned(),
        miss: LookupMiss::not_found(),
    })
}

/// A downloaded snapshot, packed as asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedBackup {
    pub bytes: Vec<u8>,
    /// The size of the snapshot before compression.
    pub uncompressed_len: usize,
}

/// Downloads the snapshot `key` of `provider`: gzip as the server keeps it, or
/// unpacked when `packing` is plain.
///
/// # Errors
/// [`LookupError`] when the server has no such backup; other
/// [`api::backup::download`] errors; those of [`compression::compressed`].
pub async fn download(
    client: &Client,
    key: &str,
    provider: &str,
    packing: Compression,
) -> Result<DownloadedBackup> {
    let backup = fetch(client, key, provider).await?;
    let state = compression::uncompressed(backup.as_bytes())?;
    let uncompressed_len = state.len();
    let bytes = match packing {
        Compression::Gzip => {
            compression::compressed(backup.as_bytes(), Compression::Gzip)?.into_owned()
        }
        Compression::Plain => state.into_owned(),
    };
    Ok(DownloadedBackup {
        bytes,
        uncompressed_len,
    })
}

async fn fetch(client: &Client, key: &str, provider: &str) -> Result<Backup> {
    api::backup::download(client, key, provider)
        .await
        .and_then(Backup::try_from)
        .map_err(|error| match error {
            // An HTML 404 is a wrong address, not a missing backup.
            Error::Http(HttpError::Status {
                status: 404,
                detail: ref body,
                ..
            }) if body != HTML_BODY => missing(key),
            other @ (Error::Login(_)
            | Error::Http(_)
            | Error::Input(_)
            | Error::LocalData(_)
            | Error::Lookup(_)
            | Error::BoardSocket(_)
            | Error::BoardChange(_)
            | Error::Picture(_)
            | Error::StoredFile(_)) => other,
        })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use yrs::{
        Any,
        Doc,
        GetString as _,
        Map as _,
        MapPrelim,
        Out,
        Text as _,
        Transact as _,
        Update,
        XmlTextPrelim,
        updates::decoder::Decode as _,
    };

    use crate::{
        Error,
        Result,
        command::{
            backup::{GetArgs, RestoreArgs, get, restored},
            fixture::{arguments, client, json_reply},
        },
        domain::{
            board::EditRight,
            board_file::{compression, document},
        },
        effect::http::HttpResponse,
        error::{board_socket::BoardSocketError, input::InputError},
        protocol::hud::{DocumentResponse, OutgoingUpdate, UpdatePayload},
        remote::{
            client::fake::reply,
            socket::fake::{
                BOARD,
                Scripted,
                acking,
                answer,
                empty_board,
                sent,
                sticker_by,
                update,
            },
        },
    };

    const WORKSPACE: &str = "00000001-0000-4000-8000-000000000001";

    fn account() -> HttpResponse {
        json_reply(&json!({
            "id": "u1",
            "name": "Ann",
            "workspaces": [{ "id": WORKSPACE, "name": "easy" }],
        }))
    }

    fn channel() -> HttpResponse {
        json_reply(&json!({
            "id": BOARD, "name": "Plan", "workspace_id": WORKSPACE,
            "schema_version": 2, "server_provider_type": "hud",
        }))
    }

    /// The server's list reply: `result` holds the backups.
    fn listing() -> HttpResponse {
        listing_by("hud")
    }

    fn listing_by(provider: &str) -> HttpResponse {
        json_reply(&json!({ "result": [{
            "key": "k1", "date": "2026-10-01", "server_provider_type": provider,
        }] }))
    }

    /// A backup body: the saved state of a board with one sticker, gzip.
    fn body() -> Result<HttpResponse> {
        packed(&sticker_by(4, "a").0)
    }

    fn packed(state: &[u8]) -> Result<HttpResponse> {
        let mut response = reply(200, &[], "");
        response.body = compression::compressed(state, compression::Compression::Gzip)?
            .into_owned();
        Ok(response)
    }

    fn empty() -> Scripted {
        acking([Some(empty_board(EditRight::Editable))])
    }

    fn updates(socket: &Scripted) -> usize {
        sent(socket)
            .iter()
            .filter(|frame| crate::protocol::hud::read_update_frame(frame).is_ok())
            .count()
    }

    /// The provider a dry-run restore names in its download, when the list
    /// puts the backup in `listed`.
    async fn downloaded_from(listed: &str) -> Result<Option<String>> {
        let (fake, client) =
            client([channel(), account(), listing_by(listed), body()?, channel()])?;
        let args = json!({ "board": BOARD, "key": "k1", "dry_run": true });
        restored(&client, &empty(), &arguments::<RestoreArgs>(args)?).await?;
        Ok(fake
            .requests()
            .iter()
            .filter(|sent| sent.url.path().ends_with("backup/download"))
            .find_map(|sent| {
                sent.url
                    .query_pairs()
                    .find(|(name, _)| name == "server_provider_type")
                    .map(|(_, value)| value.into_owned())
            }))
    }

    #[tokio::test]
    async fn a_restore_downloads_from_hud_when_the_list_says_hud() -> Result<()> {
        assert_eq!(downloaded_from("hud").await?.as_deref(), Some("hud"));
        Ok(())
    }

    fn wrong_provider(error: Option<&Error>) -> bool {
        matches!(
            error,
            Some(Error::Input(InputError::NotArgumentValue {
                name: "provider",
                ..
            }))
        )
    }

    #[tokio::test]
    async fn an_unknown_provider_is_a_usage_error_before_any_request() -> Result<()> {
        let (fake, client) = client([channel()])?;
        let fetch = json!({ "key": "k1", "provider": "s3" });
        let getting = get(&client, None, &arguments::<GetArgs>(fetch)?)
            .await
            .err();
        assert!(wrong_provider(getting.as_ref()));
        assert!(fake.requests().is_empty());
        Ok(())
    }

    /// A confirmed restore of a backup listed in `listed` fails with the
    /// format error, downloads nothing and writes nothing.
    async fn refused(listed: &str) -> Result<()> {
        let socket = empty();
        let (fake, client) =
            client([channel(), account(), listing_by(listed), body()?])?;
        let args = json!({ "board": BOARD, "key": "k1", "confirm": true });
        let error = restored(&client, &socket, &arguments::<RestoreArgs>(args)?)
            .await
            .err();
        assert!(matches!(
            error,
            Some(Error::BoardSocket(BoardSocketError::UnsupportedFormat {
                schema: 1,
                ..
            }))
        ));
        assert!(
            fake.requests()
                .iter()
                .all(|sent| !sent.url.path().ends_with("backup/download"))
        );
        assert_eq!(updates(&socket), 0);
        Ok(())
    }

    #[tokio::test]
    async fn a_backup_listed_in_another_provider_is_refused() -> Result<()> {
        refused("hocuspocus").await
    }

    /// The saved state of a board with a sticker whose text is `text`.
    fn board_saying(text: &str) -> Doc {
        let doc = Doc::with_client_id(4);
        let objects = doc.get_or_insert_map("objects");
        let documents = doc.get_or_insert_map("documents");
        let mut txn = doc.transact_mut();
        let fields = [("id".to_owned(), Any::from("s1"))];
        objects.insert(&mut txn, "s1", fields.into_iter().collect::<MapPrelim>());
        documents.insert(&mut txn, "s1", XmlTextPrelim::new(text));
        drop(txn);
        doc
    }

    /// The board the backup came from, edited since: its text changed, an
    /// object and a store entry added.
    fn edited(live: &Doc) {
        let documents = live.get_or_insert_map("documents");
        let objects = live.get_or_insert_map("objects");
        let statuses = live.get_or_insert_map("task-card-statusesStore");
        let mut txn = live.transact_mut();
        if let Some(Out::YXmlText(text)) = documents.get(&txn, "s1") {
            text.remove_range(&mut txn, 0, 6);
            text.insert(&mut txn, 0, "today");
        }
        let fields = [("id".to_owned(), Any::from("s2"))];
        objects.insert(&mut txn, "s2", fields.into_iter().collect::<MapPrelim>());
        statuses.insert(&mut txn, "done", "Done");
    }

    /// The update the restore sent, inline.
    fn update_sent(socket: &Scripted) -> Vec<u8> {
        sent(socket)
            .iter()
            .find_map(|frame| crate::protocol::hud::read_update_frame(frame).ok())
            .and_then(|(_, _, payload)| match payload {
                OutgoingUpdate::Inline(bytes) => Some(bytes),
                OutgoingUpdate::Uploaded { .. } => None,
            })
            .unwrap_or_default()
    }

    /// The text of the sticker, the object ids and the store entries.
    fn served(doc: &Doc) -> (String, Vec<String>, u32) {
        let documents = doc.get_or_insert_map("documents");
        let objects = doc.get_or_insert_map("objects");
        let statuses = doc.get_or_insert_map("task-card-statusesStore");
        let txn = doc.transact();
        let text = match documents.get(&txn, "s1") {
            Some(Out::YXmlText(text)) => text.get_string(&txn),
            _ => String::new(),
        };
        let ids = objects.keys(&txn).map(str::to_owned).collect();
        (text, ids, statuses.len(&txn))
    }

    #[tokio::test]
    async fn a_restore_returns_texts_objects_and_stores_to_the_backup() -> Result<()> {
        let saved = document::encoded(&board_saying("backup"));
        let live = document::decoded(&saved)?;
        edited(&live);
        let socket = acking([Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update(0, UpdatePayload::Inline(document::encoded(&live))),
        ])]);
        let server = document::decoded(&document::encoded(&live))?;
        let (_, client) =
            client([channel(), account(), listing(), packed(&saved)?, channel()])?;
        let args = json!({ "board": BOARD, "key": "k1", "confirm": true });
        restored(&client, &socket, &arguments::<RestoreArgs>(args)?).await?;
        server
            .transact_mut()
            .apply_update(Update::decode_v2(&update_sent(&socket))?)?;
        assert_eq!(
            served(&server),
            ("backup".to_owned(), vec!["s1".to_owned()], 0)
        );
        Ok(())
    }
}
