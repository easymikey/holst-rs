//! Uploaded files and the snapshots the server keeps of a board.

mod link;
mod transfer;

use std::io::Write as _;

use flate2::{Compression, write::GzEncoder};
use serde_json::json;

use self::{link::Pages, transfer::Transfers};
pub(crate) use self::{
    link::{FixturePage, PageOutcome},
    transfer::Download,
};
use crate::fake::{
    FixtureError,
    FixtureObject,
    board::snapshot_bytes,
    fixture::unknown,
    http::{Parts, Request, Response, found, refused},
};

/// A file the fake received, as a test sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    /// The name the storage keeps it under, which a cover field points to.
    pub stored: String,
    pub file_name: String,
    pub content: String,
    /// The id of the board it was uploaded into, if any.
    pub board: Option<String>,
}

/// The header of a PNG of `width` by `height` pixels, which is all the CLI
/// and the tool read of it.
#[must_use]
pub fn png(width: u32, height: u32) -> Vec<u8> {
    [
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13][..],
        b"IHDR",
        &width.to_be_bytes(),
        &height.to_be_bytes(),
        &[8, 2, 0, 0, 0],
    ]
    .concat()
}

/// A snapshot the fake holds when it starts, of the board called `board`.
#[derive(Debug, Clone)]
pub(crate) struct FixtureBackup {
    pub(crate) board: String,
    pub(crate) key: String,
    /// When it was taken, ISO 8601; the list is ordered by it.
    pub(crate) date: String,
    pub(crate) provider: String,
    pub(crate) content: Vec<u8>,
    /// A board with these objects replaces `content` once its id is known.
    pub(crate) objects: Option<Vec<FixtureObject>>,
}

/// Whether a download naming `provider` asks for the snapshot: the fixture's
/// own provider, or `hocuspocus`, which the client names for any but `hud`.
fn named(backup: &FixtureBackup, provider: &str) -> bool {
    provider == backup.provider
        || (provider == "hocuspocus" && backup.provider != "hud")
}

/// The snapshot as the server holds it for the board `id`.
fn held(backup: &FixtureBackup, id: &str) -> FixtureBackup {
    FixtureBackup {
        content: backup.objects.as_deref().map_or_else(
            || backup.content.clone(),
            |objects| snapshot_bytes(id, objects),
        ),
        ..backup.clone()
    }
}

pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    // writing into a `Vec` cannot fail
    drop(encoder.write_all(bytes));
    encoder.finish().unwrap_or_default()
}

#[derive(Debug, Default)]
pub(crate) struct Files {
    /// The files received, with their bytes as they came.
    uploads: Vec<(Upload, Vec<u8>)>,
    /// The snapshots, with the id of the board each belongs to.
    backups: Vec<(String, FixtureBackup)>,
    /// The most bytes a file may have; none when the server takes any size.
    limit: Option<usize>,
    pages: Pages,
    pub(crate) transfers: Transfers,
}

/// The file name and the bytes of the one file of a `multipart/form-data`
/// body.
fn file_of(body: &[u8]) -> Option<(String, Vec<u8>)> {
    let (head, rest) = body
        .split_at_checked(body.windows(4).position(|window| window == b"\r\n\r\n")?)?;
    let rest = rest.get(4..)?;
    let head = String::from_utf8_lossy(head);
    let file_name = head.split("filename=\"").nth(1)?.split('"').next()?;
    let end = rest.windows(4).rposition(|window| window == b"\r\n--")?;
    Some((file_name.to_owned(), rest.get(..end)?.to_vec()))
}

/// The name the storage gives the `number`th upload of `file_name`: a key and
/// the extension of the original file, if it has one.
fn storage_name(number: usize, file_name: &str) -> String {
    let extension = std::path::Path::new(file_name)
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    format!("{number}-{number:08x}{extension}")
}

impl Files {
    /// Fills the snapshots, the pages and the limit on the size of a file;
    /// `board_id` maps a board name to its id.
    ///
    /// # Errors
    /// [`FixtureError`] when a snapshot names a board the fixture does not
    /// hold.
    pub(crate) fn fixture(
        backups: &[FixtureBackup],
        (pages, limit): (&[FixturePage], Option<usize>),
        board_id: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, FixtureError> {
        Ok(Self {
            limit,
            pages: Pages::fixture(pages),
            transfers: Transfers::default(),
            uploads: Vec::new(),
            backups: backups
                .iter()
                .map(|backup| {
                    board_id(&backup.board)
                        .map(|id| (id.clone(), held(backup, &id)))
                        .ok_or_else(|| unknown("board", &backup.board))
                })
                .collect::<Result<_, FixtureError>>()?,
        })
    }

    pub(crate) fn uploads(&self) -> Vec<Upload> {
        self.uploads
            .iter()
            .map(|(upload, _)| upload.clone())
            .collect()
    }

    /// Answers the file endpoints; `exists` tells the boards that are there.
    pub(crate) fn respond(
        &mut self,
        request: &Request,
        exists: impl Fn(&str) -> bool,
    ) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("POST", ["", "gapi", "upload_file"]) => {
                // The server wants a key and exactly one thing to bind to.
                let binds = ["channel_id", "user_id", "workspace_id"]
                    .into_iter()
                    .filter(|name| parts.query(name).is_some())
                    .count();
                let keyed = parts.query("key").is_some_and(|key| !key.is_empty());
                Some(if binds == 1 && keyed {
                    self.upload(request, parts.query("channel_id"), exists)
                } else {
                    refused(400, "bad-request")
                })
            }
            ("GET", ["", "gapi", "link_preview"]) => {
                Some(self.pages.preview(parts.query("link")))
            }
            ("POST", ["", "gapi", "hud", "upload"]) => {
                Some(self.transfers.store(request, exists))
            }
            ("GET", ["", "gapi", "backup", "list"]) => {
                Some(self.listing(parts.query("name")))
            }
            _ => None,
        }
    }

    /// The snapshots of the board `board`, newest first.
    fn listing(&self, board: Option<&str>) -> Response {
        let mut held: Vec<&FixtureBackup> = self
            .backups
            .iter()
            .filter(|(owner, _)| Some(owner.as_str()) == board)
            .map(|(_, backup)| backup)
            .collect();
        held.sort_by(|first, second| second.date.cmp(&first.date));
        found(&json!({
            "result": held
                .iter()
                .map(|backup| json!({
                    "date": backup.date,
                    "key": backup.key,
                    "server_provider_type": backup.provider,
                }))
                .collect::<Vec<_>>()
        }))
    }

    /// Answers a download of bytes: an uploaded update, which needs no
    /// session, or a snapshot, which does; none for any other request.
    pub(crate) fn download(
        &self,
        request: &Request,
        signed_in: bool,
    ) -> Option<Download> {
        let parts = Parts::of(request);
        self.transfers.download(request).or_else(|| {
            match (parts.method, parts.segments.as_slice()) {
                // The storage is public, and holds the bytes as they came.
                ("GET", ["", "storage", name]) => self
                    .uploads
                    .iter()
                    .find(|(upload, _)| upload.stored == *name)
                    .map(|(_, bytes)| Download::Found(bytes.clone())),
                ("GET", ["", "gapi", "backup", "download"]) if signed_in => Some(
                    self.backups
                        .iter()
                        .map(|(_, backup)| backup)
                        .find(|backup| {
                            Some(backup.key.as_str()) == parts.query("key")
                                && parts
                                    .query("server_provider_type")
                                    .is_none_or(|provider| named(backup, provider))
                        })
                        .map_or(Download::Missing, |backup| {
                            Download::Found(backup.content.clone())
                        }),
                ),
                _ => None,
            }
        })
    }

    fn upload(
        &mut self,
        request: &Request,
        board: Option<&str>,
        exists: impl Fn(&str) -> bool,
    ) -> Response {
        match (file_of(&request.bytes), board.map(&exists)) {
            (_, Some(false)) => refused(404, "not-found"),
            (None, _) => refused(400, "bad-request"),
            (Some((_, content)), _)
                if self.limit.is_some_and(|most| content.len() > most) =>
            {
                refused(413, "file-too-large")
            }
            (Some((file_name, content)), _) => {
                let name = storage_name(self.uploads.len() + 1, &file_name);
                self.uploads.push((
                    Upload {
                        stored: name.clone(),
                        file_name,
                        content: String::from_utf8_lossy(&content).into_owned(),
                        board: board.map(str::to_owned),
                    },
                    content,
                ));
                found(&json!({ "name": name }))
            }
        }
    }
}
