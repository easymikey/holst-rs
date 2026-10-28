//! `board edit`: a board's cover and note.

use std::path::PathBuf;

use futures_util::future::OptionFuture;
use serde::Serialize;
use serde_json::json;

use crate::{
    command::{
        board::{
            EditArgs,
            SnapArgs,
            picture::{Delivery, snapped},
            show::{Loading, account_cache},
        },
        board_load::{CacheUse, supported},
        change::ChangeReport,
        lookup,
        node,
        object::change::{Sending, send},
        table::{not_value, parse},
    },
    domain::{
        board::EditRight,
        change::value::{BoardChange, NoteContent},
        image::{ImageFormat, image_format},
        rich_text::{kept_note, template::NoteTemplate},
    },
    effect::{board_write::WriteMode, confirmation::ALWAYS, settings::CacheSettings},
    error::{Error, Result, board_change::BoardChangeError},
    remote::{
        api::{
            self,
            files::{UploadBind, UploadOptions},
        },
        client::Client,
        socket::write::{WriteOver, connector, open_with},
    },
    storage::{cache::Cache, file},
};

const EDIT: &str = "board edit";
/// The longest side of an automatic preview, in pixels.
const PREVIEW_SIDE: u32 = 640;

/// What a board's cover became, or in a dry run would become.
#[derive(Debug, Serialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum CoverEdit {
    /// A picture file the user chose.
    File {
        file: PathBuf,
        bytes: usize,
        /// The name the server gave the upload; none in a dry run.
        uploaded: Option<String>,
    },
    /// The automatic preview, drawn by holst.
    Auto { uploaded: Option<String> },
}

/// What a board's note became, or in a dry run would become.
#[derive(Debug, Serialize)]
pub struct NoteEdit {
    /// The built-in note it started from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<NoteTemplate>,
    /// What of the markdown stayed text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// A board's changed settings.
#[derive(Debug, Serialize)]
pub struct Edited {
    pub board: String,
    /// Where the board opens.
    pub link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<CoverEdit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<NoteEdit>,
    #[serde(flatten)]
    /// Nothing is changed or uploaded in a dry run.
    pub outcome: ChangeReport<()>,
}

impl Edited {
    /// What the edit did, or would do, in a line.
    #[must_use]
    pub fn phrase(&self) -> String {
        let verb = match self.outcome {
            ChangeReport::Sent => "set",
            ChangeReport::DryRun { .. } | ChangeReport::Held { .. } => "would set",
        };
        let cover = self.cover.as_ref().map(|cover| match cover {
            CoverEdit::File { file, bytes, .. } => {
                format!("the cover to {} ({bytes} bytes)", file.display())
            }
            CoverEdit::Auto { .. } => "the cover to the automatic preview".to_owned(),
        });
        let note = self.note.as_ref().map(|note| {
            let from = note.template.map_or_else(String::new, |template| {
                format!(" to the {template} template")
            });
            let kept = note
                .warning
                .as_ref()
                .map_or_else(String::new, |warning| format!(" ({warning})"));
            format!("the note{from}{kept}")
        });
        let parts: Vec<String> = cover.into_iter().chain(note).collect();
        format!("{verb} {}", parts.join(" and "))
    }
}

/// The cover as given, read and checked before anything is sent.
enum Cover {
    File {
        path: PathBuf,
        bytes: Vec<u8>,
        format: ImageFormat,
    },
    Auto,
}

/// Changes the cover and the note of a board. The cover is a picture file, or
/// `reset` for the automatic preview, which is drawn here and uploaded.
///
/// # Errors
/// Nothing to change, a cover file that is unreadable or no picture, a
/// board that is not found or read only, and errors of the requests.
pub async fn edit(
    client: &Client,
    cache: &CacheSettings,
    args: &EditArgs,
) -> Result<Edited> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    let loading = Loading {
        connector: connector(),
        cache: account.as_ref().map(Cache::as_port),
        cache_use: CacheUse::Use,
    };
    edited(client, loading, args).await
}

async fn edited(
    client: &Client,
    loading: Loading<'_>,
    args: &EditArgs,
) -> Result<Edited> {
    let cover = args.cover.as_deref().map(cover_of).transpose()?;
    let (cover, note) = match (cover, note_of(args)?) {
        (None, None) => Err(not_value(
            EDIT,
            "cover",
            "given, or --note or --note-template",
        )),
        given => Ok(given),
    }?;
    let id = lookup::board_id(client, &args.board, args.workspace.as_deref()).await?;
    let sending = args.dry_run == WriteMode::Send;
    // The right is read once, before anything is sent.
    if sending {
        editable(client, loading, &id).await?;
    }
    // Uploads first: a refused upload leaves the board as it was.
    let cover = OptionFuture::from(
        cover.map(|cover| cover_uploaded(client, loading, &id, cover, sending)),
    )
    .await
    .transpose()?;
    let note = note_written(client, loading, &id, note, sending).await?;
    if let Some(cover) = &cover {
        cover_fixed(client, &id, cover).await?;
    }
    Ok(Edited {
        link: client.board_link(&id),
        board: id,
        cover,
        note,
        outcome: if sending {
            ChangeReport::Sent
        } else {
            ChangeReport::DryRun { request: () }
        },
    })
}

/// The note as given, checked before anything is sent.
fn note_of(args: &EditArgs) -> Result<Option<NoteContent>> {
    match (&args.note, args.note_template, args.note_format) {
        (Some(_), Some(_), _) => {
            Err(not_value(EDIT, "note-template", "left out with --note"))
        }
        (None, Some(_), Some(_)) => Err(not_value(
            EDIT,
            "note-format",
            "left out with --note-template",
        )),
        (None, None, Some(_)) => {
            Err(not_value(EDIT, "note-format", "given with --note"))
        }
        (Some(text), None, format) => Ok(Some(NoteContent::Text {
            text: text.0.clone(),
            format: format.unwrap_or_default(),
        })),
        (None, Some(template), None) => Ok(Some(NoteContent::Template(template))),
        (None, None, None) => Ok(None),
    }
}

/// Replaces the note; a dry run writes nothing.
async fn note_written(
    client: &Client,
    loading: Loading<'_>,
    id: &str,
    note: Option<NoteContent>,
    sending: bool,
) -> Result<Option<NoteEdit>> {
    OptionFuture::from(note.map(|content| async move {
        let edit = NoteEdit {
            template: match &content {
                NoteContent::Template(template) => Some(*template),
                NoteContent::Text { .. } => None,
            },
            warning: kept_note(&content.parsed().kept),
        };
        if sending {
            send(
                client,
                loading.cache,
                &WriteOver::sender(loading.connector),
                Sending {
                    board: id,
                    change: &BoardChange::Note(content),
                    release: ALWAYS,
                },
            )
            .await?;
        }
        Ok::<_, Error>(edit)
    }))
    .await
    .transpose()
}

fn cover_of(given: &str) -> Result<Cover> {
    match given {
        "reset" => Ok(Cover::Auto),
        path => {
            let bytes = file::read(path)?;
            image_format(&bytes)
                .filter(|format| *format != ImageFormat::Svg)
                .map(|format| Cover::File {
                    path: PathBuf::from(path),
                    bytes,
                    format,
                })
                .ok_or_else(|| {
                    not_value(
                        EDIT,
                        "cover",
                        "a png, jpg, gif or webp picture, or reset",
                    )
                })
        }
    }
}

/// Checks that the user may edit the board.
async fn editable(client: &Client, loading: Loading<'_>, id: &str) -> Result<()> {
    supported(client, id).await?;
    let connected = open_with(client, loading.connector, id).await?;
    if matches!(connected.right(), EditRight::Editable) {
        Ok(())
    } else {
        Err(Error::BoardChange(BoardChangeError::ReadOnly))
    }
}

/// Uploads the cover's picture; the board is not changed yet.
async fn cover_uploaded(
    client: &Client,
    loading: Loading<'_>,
    id: &str,
    cover: Cover,
    sending: bool,
) -> Result<CoverEdit> {
    match (cover, sending) {
        (Cover::File { path, bytes, .. }, false) => Ok(CoverEdit::File {
            file: path,
            bytes: bytes.len(),
            uploaded: None,
        }),
        (
            Cover::File {
                path,
                bytes,
                format,
            },
            true,
        ) => {
            let size = bytes.len();
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("cover");
            let name = uploaded(
                client,
                id,
                Upload {
                    key: None,
                    file_name,
                    content_type: format.mime(),
                },
                bytes,
            )
            .await?;
            Ok(CoverEdit::File {
                file: path,
                bytes: size,
                uploaded: Some(name),
            })
        }
        (Cover::Auto, false) => Ok(CoverEdit::Auto { uploaded: None }),
        (Cover::Auto, true) => {
            // Drawn and uploaded first: a board that cannot be drawn keeps its cover.
            let picture = preview(client, loading, id).await?;
            let key = format!("preview-{id}");
            let name = uploaded(
                client,
                id,
                Upload {
                    key: Some(&key),
                    file_name: "preview.png",
                    content_type: ImageFormat::Png.mime(),
                },
                picture,
            )
            .await?;
            Ok(CoverEdit::Auto {
                uploaded: Some(name),
            })
        }
    }
}

/// Fixes an uploaded cover on the board; a dry run uploaded nothing.
async fn cover_fixed(client: &Client, id: &str, cover: &CoverEdit) -> Result<()> {
    match cover {
        CoverEdit::File {
            uploaded: Some(name),
            ..
        } => {
            api::channel::set_cover(client, id, name).await?;
            Ok(())
        }
        CoverEdit::Auto {
            uploaded: Some(name),
        } => {
            api::channel::reset_cover(client, id).await?;
            api::channel::set_preview(client, id, name).await?;
            Ok(())
        }
        CoverEdit::File { uploaded: None, .. } | CoverEdit::Auto { uploaded: None } => {
            Ok(())
        }
    }
}

/// What names a cover's file besides its bytes.
struct Upload<'a> {
    key: Option<&'a str>,
    file_name: &'a str,
    content_type: &'a str,
}

/// Uploads a cover's file bound to the board; the server keeps it no longer
/// than the board's cover needs (freshness `0`).
async fn uploaded(
    client: &Client,
    id: &str,
    file: Upload<'_>,
    bytes: Vec<u8>,
) -> Result<String> {
    let options = UploadOptions {
        key: file.key,
        freshness: Some("0"),
        content_type: file.content_type,
    };
    api::files::upload(
        client,
        UploadBind::Channel(id),
        options,
        file.file_name,
        bytes,
    )
    .await?
    .name
    .ok_or_else(|| node::missing_field("name"))
}

/// The board drawn small by our renderer, as PNG.
async fn preview(client: &Client, loading: Loading<'_>, id: &str) -> Result<Vec<u8>> {
    let drawing = json!({ "board": id, "format": "png", "max_side": PREVIEW_SIDE });
    let args: SnapArgs = parse(drawing.as_object().cloned().unwrap_or_default())?;
    match snapped(client, loading, None, &args).await?.delivery {
        Delivery::Inline { bytes } => Ok(bytes),
        Delivery::Written { .. } => {
            Err(not_value(EDIT, "cover", "a picture drawn in memory"))
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "tests build and read real files in temp directories"
    )]
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::{path::PathBuf, sync::Arc};

    use serde_json::{Value, json};

    use crate::{
        Result,
        command::{
            board::{EditArgs, settings::edited, show::Loading},
            board_load::CacheUse,
            fixture::{arguments, client, directory, json_reply},
        },
        domain::board::EditRight,
        effect::http::HttpResponse,
        protocol::hud::{DocumentResponse, UpdatePayload},
        remote::{
            client::fake::Fake,
            socket::fake::{
                BOARD,
                Scripted,
                acking,
                answer,
                empty_board,
                script,
                sticker_by,
                update,
            },
        },
    };

    fn channel() -> HttpResponse {
        json_reply(&json!({
            "id": BOARD, "name": "Plan",
            "workspace_id": "00000001-0000-4000-8000-000000000001",
            "schema_version": 2, "server_provider_type": "hud",
        }))
    }

    fn loading(socket: &Scripted) -> Loading<'_> {
        Loading {
            connector: socket,
            cache: None,
            cache_use: CacheUse::Use,
        }
    }

    fn editable() -> Scripted {
        acking([Some(empty_board(EditRight::Editable))])
    }

    /// Two connections to a board of one sticker: the right is read on the
    /// first, the preview is drawn from the second.
    fn drawn() -> Scripted {
        let (state, _) = sticker_by(4, "a");
        let session = || {
            Some(vec![
                answer(DocumentResponse::Subscribed {
                    right: EditRight::Editable,
                }),
                update(0, UpdatePayload::Inline(state.clone())),
            ])
        };
        script([session(), session()])
    }

    /// A file with the signature of a PNG in a new directory.
    fn picture(name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let file = directory()?.join(name);
        std::fs::write(&file, bytes)?;
        Ok(file)
    }

    /// The method and path of each request that is not a read.
    #[cfg(not(feature = "png"))]
    fn writes(fake: &Fake) -> Vec<(String, String)> {
        fake.requests()
            .iter()
            .filter(|request| request.method.as_str() != "GET")
            .map(|request| {
                (
                    request.method.as_str().to_owned(),
                    request.url.path().to_owned(),
                )
            })
            .collect()
    }

    /// The query of the one upload as `name=value` pairs, and its body.
    fn upload_of(fake: &Fake) -> (Vec<(String, String)>, String) {
        fake.requests()
            .iter()
            .find(|request| request.url.path() == "/gapi/upload_file")
            .map(|request| {
                (
                    request
                        .url
                        .query_pairs()
                        .map(|(name, value)| (name.into_owned(), value.into_owned()))
                        .collect(),
                    request
                        .body
                        .as_ref()
                        .map(|body| String::from_utf8_lossy(&body.bytes).into_owned())
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    }

    async fn run(
        given: &Value,
        replies: impl IntoIterator<Item = HttpResponse>,
        socket: &Scripted,
    ) -> Result<(Arc<Fake>, Result<super::Edited>)> {
        let (fake, client) = client(replies)?;
        let done = edited(
            &client,
            loading(socket),
            &arguments::<EditArgs>(given.clone())?,
        )
        .await;
        Ok((fake, done))
    }

    fn uploaded() -> HttpResponse {
        json_reply(&json!({ "name": "u1.png" }))
    }

    fn channel_changed() -> HttpResponse {
        json_reply(&json!({ "id": BOARD }))
    }

    #[cfg(feature = "png")]
    #[tokio::test]
    async fn a_reset_whose_preview_is_refused_keeps_the_cover() -> Result<()> {
        let (fake, done) = run(
            &json!({ "board": BOARD, "cover": "reset" }),
            [
                channel(),
                channel(),
                crate::remote::client::fake::reply(413, &[], ""),
            ],
            &drawn(),
        )
        .await?;
        assert!(done.is_err());
        assert!(
            fake.requests()
                .iter()
                .all(|request| request.method.as_str() != "PATCH")
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_cover_upload_is_bound_to_the_board_with_a_key_and_its_type() -> Result<()>
    {
        let file = picture("cover.png", b"\x89PNG\r\n\x1a\n....")?;
        let (fake, done) = run(
            &json!({ "board": BOARD, "cover": file }),
            [channel(), uploaded(), channel_changed()],
            &editable(),
        )
        .await?;
        done?;
        let (query, body) = upload_of(&fake);
        let value = |name: &str| {
            query
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(
            (
                value("channel_id"),
                value("freshness_duration"),
                value("key")
                    .is_some_and(|key| !key.is_empty() && !key.starts_with("preview-")),
                query.len(),
                body.contains("Content-Type: image/png\r\n"),
            ),
            (Some(BOARD), Some("0"), true, 3, true)
        );
        Ok(())
    }

    #[cfg(feature = "png")]
    #[tokio::test]
    async fn a_reset_preview_is_stored_under_the_boards_preview_key() -> Result<()> {
        let (fake, done) = run(
            &json!({ "board": BOARD, "cover": "reset" }),
            [
                channel(),
                channel(),
                channel(),
                uploaded(),
                channel_changed(),
                channel_changed(),
            ],
            &drawn(),
        )
        .await?;
        done?;
        let (query, _) = upload_of(&fake);
        assert_eq!(
            query,
            [
                ("channel_id".to_owned(), BOARD.to_owned()),
                ("key".to_owned(), format!("preview-{BOARD}")),
                ("freshness_duration".to_owned(), "0".to_owned()),
            ]
        );
        Ok(())
    }

    /// A build that draws no PNG changes nothing.
    #[cfg(not(feature = "png"))]
    #[tokio::test]
    async fn a_reset_without_png_changes_nothing() -> Result<()> {
        let (fake, done) = run(
            &json!({ "board": BOARD, "cover": "reset" }),
            [
                channel(),
                channel(),
                channel(),
                uploaded(),
                channel_changed(),
                channel_changed(),
            ],
            &drawn(),
        )
        .await?;
        assert!(
            done.is_err_and(|error| error.to_string().contains("no PNG")),
            "no error about PNG"
        );
        assert_eq!(writes(&fake).len(), 0);
        Ok(())
    }

    #[rstest::rstest]
    #[case::nothing(
        json!({ "board": BOARD }),
        "cover of `board edit` must be given, or --note or --note-template"
    )]
    #[case::both(
        json!({ "board": BOARD, "note": "a", "note_template": "todo" }),
        "note-template of `board edit` must be left out with --note"
    )]
    #[case::format_of_template(
        json!({ "board": BOARD, "note_template": "todo", "note_format": "markdown" }),
        "note-format of `board edit` must be left out with --note-template"
    )]
    #[case::format_alone(
        json!({ "board": BOARD, "note_format": "markdown" }),
        "note-format of `board edit` must be given with --note"
    )]
    #[tokio::test]
    async fn a_note_asked_wrongly_is_refused_before_any_request(
        #[case] given: Value,
        #[case] message: &str,
    ) -> Result<()> {
        let (fake, done) = run(&given, [], &editable()).await?;
        assert_eq!(
            done.err().map(|error| error.to_string()).as_deref(),
            Some(message)
        );
        assert!(fake.requests().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_dry_run_note_reports_what_stayed_text_and_writes_nothing() -> Result<()>
    {
        let (fake, done) = run(
            &json!({ "board": BOARD, "note": "> quoted", "note_format": "markdown", "dry_run": true }),
            [channel()],
            &editable(),
        )
        .await?;
        let edited = done?;
        assert_eq!(
            (edited.phrase(), edited.cover.is_none()),
            ("would set the note (kept as text: quote)".to_owned(), true)
        );
        assert!(
            fake.requests()
                .iter()
                .all(|request| request.method.as_str() == "GET")
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_template_dry_run_names_the_template() -> Result<()> {
        let (_, done) = run(
            &json!({ "board": BOARD, "note_template": "agenda", "dry_run": true }),
            [channel()],
            &editable(),
        )
        .await?;
        assert_eq!(done?.phrase(), "would set the note to the agenda template");
        Ok(())
    }
}
