//! `board export` and `board import`: a whole board to a file and back.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    command::{
        board::{
            ExportArgs,
            ImportArgs,
            picture::{Place, stem_of},
            show::{Loading, through_cache},
            tree_change::{Action, Plan, Stale, carry, node_of},
        },
        board_file::{self, ImportRequest},
        board_load::CacheUse,
        change::{ChangeReport, PlannedRequest, gate},
        lookup,
        node,
        table::{Confirmation, not_value},
    },
    domain::{
        board::Object,
        board_file::{
            compression::{self, Compression},
            document::{self, Overwrite},
        },
        change::receipt::UpdateReceipt,
        id::WorkspaceId,
        lookup::choice,
        node::{DeleteMode, NewNode, NodeBrief, NodeKind, NodeName},
    },
    effect::{board_write::WriteMode, settings::CacheSettings},
    error::{
        Error,
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
    },
    remote::{api, client::Client},
    storage::file::{read, write_new},
};

const EXPORT: &str = "board export";
const IMPORT: &str = "board import";

/// The file name that asks for the document itself, not a file.
pub const STDOUT: &str = "-";

/// A board saved as a file, and where it went.
#[derive(Debug, Serialize)]
pub struct Exported {
    /// The id of the board.
    pub board: String,
    pub objects: usize,
    /// The size of the document before gzip.
    pub state_bytes: usize,
    /// The size of the file.
    pub bytes: usize,
    /// Where the file went; [`STDOUT`] when the document is in `inline`.
    pub file: PathBuf,
    pub object_list: Option<ObjectList>,
    /// The document, when the file was [`STDOUT`].
    #[serde(skip)]
    pub inline: Option<Vec<u8>>,
}

/// The objects of an export, one per line, beside the file.
#[derive(Debug, Serialize)]
pub struct ObjectList {
    pub file: PathBuf,
    pub bytes: usize,
}

/// Saves the whole board as a file, gzip unless asked otherwise. A given file
/// keeps its name as given; the default name in `downloads` ends in `.gz` only
/// when the file is gzip. The file is new and never replaces one. The file
/// [`STDOUT`] writes nothing and returns the document.
///
/// # Errors
/// A file that exists, an object list with [`STDOUT`], no file when `downloads`
/// is none, a board that is not found or not loaded whole, and errors of the
/// requests.
pub async fn export(
    client: &Client,
    cache: &CacheSettings,
    downloads: Option<&Path>,
    args: &ExportArgs,
) -> Result<Exported> {
    through_cache(client, cache, CacheUse::Use, async |loading| {
        exported(client, loading, downloads, args).await
    })
    .await
}

async fn exported(
    client: &Client,
    loading: Loading<'_>,
    downloads: Option<&Path>,
    args: &ExportArgs,
) -> Result<Exported> {
    let id = lookup::board_id(client, &args.board, args.workspace.as_deref()).await?;
    let place = match (args.file.as_deref(), downloads) {
        (Some(STDOUT), _) if args.object_list => Err(not_value(
            EXPORT,
            "object_list",
            "left out when the file is -",
        ))?,
        (Some(STDOUT), _) => Wanted::Inline,
        (Some(file), _) => Wanted::File(Path::new(file)),
        (None, Some(dir)) => Wanted::Downloads(dir),
        (None, None) => Err(not_value(
            EXPORT,
            "file",
            "given: there is no home directory to find ~/Downloads in",
        ))?,
    };
    let (compression, extension) = if args.no_compress {
        (Compression::Plain, "ydoc")
    } else {
        (Compression::Gzip, "ydoc.gz")
    };
    let saved = board_file::export_with(client, loading, &id).await?;
    let place = match place {
        Wanted::Inline => None,
        Wanted::File(file) => Some(Place::File(file)),
        Wanted::Downloads(dir) => {
            let channel = match &saved.channel {
                Some(reply) => reply.clone(),
                None => api::channel::get(client, &id).await?,
            };
            let brief = NodeBrief::of(&channel)?;
            Some(Place::Numbered(dir, stem_of(brief.name(), &id)))
        }
    };
    let document = compression::compressed(&saved.document, compression)?;
    let lines = args
        .object_list
        .then(|| json_lines(&saved.content.objects))
        .transpose()?;
    let bytes = document.len();
    let (file, inline) = match place {
        Some(place) => (place.write(extension, &document)?, None),
        None => (PathBuf::from(STDOUT), Some(document.into_owned())),
    };
    let object_list = lines
        .map(|lines| {
            let beside = listing_path(&file);
            write_new(&beside, &lines)?;
            Ok::<_, Error>(ObjectList {
                file: beside,
                bytes: lines.len(),
            })
        })
        .transpose()?;
    Ok(Exported {
        board: id,
        objects: saved.content.objects.len(),
        state_bytes: saved.document.len(),
        bytes,
        file,
        object_list,
        inline,
    })
}

/// Where the file goes; a folder's file name waits for the loaded name.
enum Wanted<'a> {
    Inline,
    File(&'a Path),
    Downloads(&'a Path),
}

/// `<file>.objects.jsonl`, without the `.gz` the file may end in.
fn listing_path(file: &Path) -> PathBuf {
    let mut name = compression::stem(file).into_os_string();
    name.push(".objects.jsonl");
    PathBuf::from(name)
}

fn json_lines(objects: &[Object]) -> Result<Vec<u8>> {
    objects.iter().try_fold(Vec::new(), |mut out, object| {
        serde_json::to_writer(&mut out, object)?;
        out.push(b'\n');
        Ok(out)
    })
}

/// A board file written into a board, or in a dry run only planned.
#[derive(Debug, Serialize)]
pub struct Imported {
    /// The id of the board; none when a dry run would make it.
    pub board: Option<String>,
    pub name: String,
    pub workspace: String,
    pub workspace_name: String,
    /// Where the board opens.
    pub link: Option<String>,
    /// The objects the file holds.
    pub saved_objects: usize,
    pub objects_before: usize,
    pub objects_after: usize,
    #[serde(flatten)]
    pub receipt: Option<UpdateReceipt>,
    #[serde(flatten)]
    pub outcome: ImportReport,
}

/// What came of an import; a dry run shows the request only for a new board.
pub type ImportReport = ChangeReport<Option<PlannedRequest>>;

/// Makes a board of a board file in a workspace and folder, or with `into`
/// writes the file into an existing board. Only `into` needs the confirmation.
///
/// # Errors
/// A file that cannot be read or is not a board file, `merge`, `folder` or
/// `name` where they do not apply, a name that is blank, a workspace, folder or
/// board that is not found, a board that is read only or, without `merge`, not
/// empty, and errors of the requests.
pub async fn import(
    client: &Client,
    cache: &CacheSettings,
    args: &ImportArgs,
) -> Result<Imported> {
    through_cache(client, cache, CacheUse::Use, async |loading| {
        imported(client, loading, args).await
    })
    .await
}

async fn imported(
    client: &Client,
    loading: Loading<'_>,
    args: &ImportArgs,
) -> Result<Imported> {
    refuse_stray(args)?;
    let file = read(&args.path)?;
    let state = compression::uncompressed(&file)?;
    let saved = document::saved_objects(&state)?;
    match &args.into {
        Some(into) => into_board(client, loading, into, &state, args).await,
        None => new_board(client, loading, &state, saved, args).await,
    }
}

/// Refuses what only the other kind of import takes.
fn refuse_stray(args: &ImportArgs) -> Result<()> {
    let stray = match &args.into {
        Some(_) => [
            ("folder", args.folder.is_some()),
            ("name", args.name.is_some()),
        ]
        .into_iter()
        .find_map(|(name, given)| given.then_some((name, "left out with into"))),
        None => args
            .merge
            .then_some(("merge", "left out unless into is given")),
    };
    stray.map_or(Ok(()), |(name, expected)| {
        Err(not_value(IMPORT, name, expected))
    })
}

async fn into_board(
    client: &Client,
    loading: Loading<'_>,
    into: &str,
    state: &[u8],
    args: &ImportArgs,
) -> Result<Imported> {
    let target = lookup::exact_board(client, into, args.workspace.as_deref()).await?;
    let unconfirmed =
        Unconfirmed::new("writing a board file into a board", ChangeRisk::Everyone);
    let gated = gate(Some(unconfirmed), (args.dry_run, args.confirm), || None);
    let done = board_file::import_with(
        client,
        loading,
        ImportRequest {
            board: &target.id,
            document: state,
            overwrite: if args.merge {
                Overwrite::Merge
            } else {
                Overwrite::EmptyOnly
            },
            mode: gated
                .as_ref()
                .err()
                .map_or(WriteMode::Send, |_| WriteMode::DryRun),
            channel: Some(&target.channel),
        },
    )
    .await?;
    let outcome = gated.err().unwrap_or(ChangeReport::Sent);
    Ok(Imported {
        link: Some(client.board_link(&target.id)),
        board: Some(target.id),
        name: target.name,
        workspace: target.workspace.id,
        workspace_name: target.workspace.name,
        saved_objects: done.plan.saved,
        objects_before: done.plan.before,
        objects_after: done.plan.after,
        receipt: Some(done.receipt()),
        outcome,
    })
}

async fn new_board(
    client: &Client,
    loading: Loading<'_>,
    state: &[u8],
    saved: usize,
    args: &ImportArgs,
) -> Result<Imported> {
    let name: NodeName = match &args.name {
        Some(name) => name.parse()?,
        None => named_after(Path::new(&args.path))?.parse()?,
    };
    let shown = name.to_string();
    let workspace = lookup::workspace(client, args.workspace.as_deref()).await?;
    let workspace_id: WorkspaceId = workspace.id.parse()?;
    let parent = node::parent(
        client,
        &workspace_id,
        args.folder.as_deref(),
        choice::listed,
    )
    .await?;
    let new = NewNode {
        name,
        kind: NodeKind::Board,
        workspace: workspace_id,
        parent,
    };
    let creation = node::plan_create(client, &new).await?;
    let made = carry(
        client,
        Plan::plain(
            creation,
            node_of(Action::Create, None, Some(NodeKind::Board)),
        )
        .staling(Stale::Workspace(workspace.id.clone())),
        (args.dry_run, Confirmation::Missing),
    )
    .await?;
    let imported = |board, link, receipt, plan: (usize, usize), outcome| Imported {
        board,
        name: shown.clone(),
        workspace: workspace.id.clone(),
        workspace_name: workspace.name.clone(),
        link,
        saved_objects: saved,
        objects_before: plan.0,
        objects_after: plan.1,
        receipt,
        outcome,
    };
    match made.outcome {
        ChangeReport::DryRun { request } | ChangeReport::Held { request, .. } => {
            Ok(imported(
                None,
                None,
                None,
                (0, saved),
                ChangeReport::DryRun {
                    request: Some(request),
                },
            ))
        }
        ChangeReport::Sent => {
            let id = made.node.id.ok_or_else(|| node::missing_field("id"))?;
            let filled = board_file::import_with(
                client,
                loading,
                ImportRequest {
                    board: &id,
                    document: state,
                    overwrite: Overwrite::EmptyOnly,
                    mode: WriteMode::Send,
                    channel: None,
                },
            )
            .await;
            match filled {
                Ok(done) => Ok(imported(
                    Some(id.clone()),
                    Some(client.board_link(&id)),
                    Some(done.receipt()),
                    (done.plan.before, done.plan.after),
                    ChangeReport::Sent,
                )),
                Err(error) => Err(unmade(client, &id, error).await),
            }
        }
    }
}

/// The import's error, after the board it made for the import is moved to the
/// trash; when that fails too, the error also names the board left behind.
async fn unmade(client: &Client, id: &str, error: Error) -> Error {
    match trashed(client, id).await {
        Ok(_) => error,
        Err(failed) => Error::from(InputError::Several {
            errors: vec![
                error,
                Error::from(InputError::ImportBoardLeft {
                    board: id.to_owned(),
                    failed: Box::new(failed),
                }),
            ],
        }),
    }
}

async fn trashed(client: &Client, id: &str) -> Result<serde_json::Value> {
    client
        .send_json(api::channel::delete_request(id, &DeleteMode::Trash.into())?)
        .await
}

/// The file's name without `.gz` and the extension before it.
fn named_after(file: &Path) -> Result<String> {
    compression::stem(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_owned)
        .ok_or_else(|| not_value(IMPORT, "name", "given: the file's name gives none"))
}

#[cfg(test)] mod tests;
