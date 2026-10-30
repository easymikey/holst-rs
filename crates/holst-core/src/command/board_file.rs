//! Copying a whole board document: export from one board, import into another.

use futures_util::future::OptionFuture;
use serde::Deserialize as _;
use serde_json::Value;
use yrs::Doc;

use crate::{
    command::{
        board::show::Loading,
        board_load::{self, RemoteBoard},
    },
    domain::{
        board::{BoardContent, EditRight, objects_root},
        board_file::document::{ImportPlan, Overwrite},
        change::receipt::{Delivery, UpdateReceipt},
    },
    effect::board_write::WriteMode,
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        board_socket::BoardSocketError,
    },
    protocol::{channel::BoardFormat, document::OBJECTS},
    remote::client::Client,
};

/// A board as exported: its listing and its whole document.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::command) struct BoardFile {
    pub(in crate::command) content: BoardContent,
    pub(in crate::command) document: Vec<u8>,
    /// The `channel/{id}` reply the load read, if it read one.
    pub(in crate::command) channel: Option<Value>,
}

/// Loads the whole board via `loading`'s cache; only reads.
///
/// # Errors
/// As [`crate::remote::socket::load::load_with`]; never saves incomplete.
pub(in crate::command) async fn export_with(
    client: &Client,
    loading: Loading<'_>,
    board: &str,
) -> Result<BoardFile> {
    let (loaded, _, channel) = board_load::load_with_channel(
        RemoteBoard {
            client,
            connector: loading.connector,
            board,
        },
        loading.cache,
        loading.cache_use,
    )
    .await?;
    v2_only(&loaded.document)?;
    Ok(BoardFile {
        content: BoardContent {
            edit_right: loaded.right,
            objects: crate::domain::board::objects(&loaded.document),
        },
        document: crate::domain::board_file::document::encoded(&loaded.document),
        channel,
    })
}

/// Whole-board files are v2 only; a version 3 document has `objects_v3`.
fn v2_only(document: &Doc) -> Result<()> {
    match objects_root(document) {
        OBJECTS => Ok(()),
        _ => Err(Error::BoardSocket(BoardSocketError::UnsupportedFormat {
            schema: 3,
            provider: "hud".to_owned(),
        })),
    }
}

/// A saved state written into a board, or in a dry run only planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::command) struct ImportedBoardFile {
    pub(in crate::command) plan: ImportPlan,
    pub(in crate::command) update_id: u64,
    pub(in crate::command) delivery: Delivery,
}

impl ImportedBoardFile {
    /// The receipt of the update: built only, or confirmed.
    pub(in crate::command) const fn receipt(&self) -> UpdateReceipt {
        UpdateReceipt {
            bytes: self.plan.update.len(),
            id: self.update_id,
            delivery: self.delivery,
        }
    }
}

/// What to import: the saved state, the target board and how to write.
#[derive(Debug, Clone, Copy)]
pub(in crate::command) struct ImportRequest<'a> {
    pub(in crate::command) board: &'a str,
    pub(in crate::command) document: &'a [u8],
    pub(in crate::command) overwrite: Overwrite,
    pub(in crate::command) mode: WriteMode,
    /// The `channel/{id}` reply of the board, if the caller has read it.
    pub(in crate::command) channel: Option<&'a Value>,
}

/// Load the target, plan the import and, unless dry, send it and await the ack.
///
/// # Errors
/// Decode errors,
/// [`crate::error::board_change::BoardChangeError::ReadOnly`],
/// [`crate::error::board_change::BoardChangeError::NotEmpty`], I/O.
pub(in crate::command) async fn import_with(
    client: &Client,
    loading: Loading<'_>,
    request: ImportRequest<'_>,
) -> Result<ImportedBoardFile> {
    let ImportRequest {
        board,
        document: state,
        overwrite: fill,
        mode,
        channel,
    } = request;
    let saved = crate::domain::board_file::document::saved_objects(state)?;
    write_with(client, loading, (board, channel), mode, |document| {
        crate::domain::board_file::document::import_plan(document, state, saved, fill)
    })
    .await
}

/// Like [`import_with`], but the whole document of the board becomes exactly
/// the saved state.
///
/// # Errors
/// Decode errors, [`crate::error::board_change::BoardChangeError::ReadOnly`],
/// I/O.
pub(in crate::command) async fn replace_with(
    client: &Client,
    loading: Loading<'_>,
    board: &str,
    channel: &Value,
    state: &[u8],
    mode: WriteMode,
) -> Result<ImportedBoardFile> {
    let saved = crate::domain::board_file::document::saved_objects(state)?;
    write_with(client, loading, (board, Some(channel)), mode, |document| {
        crate::domain::board_file::document::replace_plan(document, state, saved)
    })
    .await
}

/// Load the board, let `plan` change its document, and, unless dry, send what
/// Adds the change, awaits the ack; format checked on `target`'s channel reply.
///
/// # Errors
/// Those of `plan`, [`crate::error::board_change::BoardChangeError::ReadOnly`],
/// I/O.
pub(in crate::command) async fn write_with(
    client: &Client,
    loading: Loading<'_>,
    (board, channel): (&str, Option<&Value>),
    mode: WriteMode,
    plan: impl FnOnce(&Doc) -> Result<ImportPlan>,
) -> Result<ImportedBoardFile> {
    match channel {
        Some(reply) => BoardFormat::deserialize(reply)?.whole_file()?,
        None => board_load::whole_file(client, board).await?,
    }
    let connected =
        crate::remote::socket::write::open_with(client, loading.connector, board)
            .await?;
    let plan = if matches!(connected.right(), EditRight::Editable) {
        plan(connected.document())
    } else {
        Err(Error::BoardChange(BoardChangeError::ReadOnly))
    }?;
    let update_id = crate::domain::change::build::update_id(&client.new_id());
    let delivery = match mode {
        WriteMode::DryRun => Delivery::Built,
        WriteMode::Send => {
            crate::remote::socket::write::sent(
                connected,
                client,
                plan.update.clone(),
                update_id,
            )
            .await?;
            // The cached copy is the board before this write.
            OptionFuture::from(
                loading.cache.map(|cache| board_load::forget(cache, board)),
            )
            .await;
            Delivery::Accepted
        }
    };
    Ok(ImportedBoardFile {
        plan,
        update_id,
        delivery,
    })
}
