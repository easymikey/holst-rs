//! Copying a whole board document: export from one board, import into another.

use yrs::Doc;

use crate::{
    domain::{
        board::{BoardContent, EditRight},
        board_file::document::{ImportPlan, Overwrite},
        change::receipt::{Delivery, UpdateReceipt},
    },
    effect::{
        board_write::WriteMode,
        board_write_plan::AcceptedUpdate,
        socket::SocketConnector,
    },
    error::{Error, Result, board_change::BoardChangeError},
    remote::client::Client,
};

/// A board as exported: its listing and its whole document.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardFile {
    pub content: BoardContent,
    pub document: Vec<u8>,
}

/// Load the board fully over `connector` and take its whole state; only reads.
///
/// # Errors
/// As [`crate::remote::socket::load::load_with`]: an incomplete board is never
/// saved.
pub async fn export_with(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
) -> Result<BoardFile> {
    super::board_load::supported(client, board).await?;
    let loaded =
        crate::remote::socket::load::load_with(client, connector, board).await?;
    Ok(BoardFile {
        content: BoardContent {
            edit_right: loaded.right,
            objects: crate::domain::board::objects(&loaded.document),
        },
        document: crate::domain::board_file::document::encoded(&loaded.document),
    })
}

/// A saved state written into a board, or in a dry run only planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedBoardFile {
    pub plan: ImportPlan,
    pub update_id: u64,
    pub accepted: Option<AcceptedUpdate>,
}

impl ImportedBoardFile {
    /// The receipt of the update: built only, or confirmed.
    #[must_use]
    pub const fn receipt(&self) -> UpdateReceipt {
        UpdateReceipt {
            bytes: self.plan.update.len(),
            id: self.update_id,
            delivery: match self.accepted {
                Some(_) => Delivery::Accepted,
                None => Delivery::Built,
            },
        }
    }
}

/// What to import: the saved state, the target board and how to write.
#[derive(Debug, Clone, Copy)]
pub struct ImportRequest<'a> {
    pub board: &'a str,
    pub document: &'a [u8],
    pub overwrite: Overwrite,
    pub mode: WriteMode,
}

/// Load the target, plan the import and, unless dry, send it and await the ack.
///
/// # Errors
/// Decode errors,
/// [`crate::error::board_change::BoardChangeError::ReadOnly`],
/// [`crate::error::board_change::BoardChangeError::NotEmpty`], I/O.
pub async fn import_with(
    client: &Client,
    connector: &dyn SocketConnector,
    request: ImportRequest<'_>,
) -> Result<ImportedBoardFile> {
    let ImportRequest {
        board,
        document: state,
        overwrite: fill,
        mode,
    } = request;
    let saved = crate::domain::board_file::document::saved_objects(state)?;
    write_with(client, connector, board, mode, |document| {
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
pub async fn replace_with(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    state: &[u8],
    mode: WriteMode,
) -> Result<ImportedBoardFile> {
    let saved = crate::domain::board_file::document::saved_objects(state)?;
    write_with(client, connector, board, mode, |document| {
        crate::domain::board_file::document::replace_plan(document, state, saved)
    })
    .await
}

/// Load the board, let `plan` change its document, and, unless dry, send what
/// the change adds and await the ack.
///
/// # Errors
/// Those of `plan`,
/// [`crate::error::board_change::BoardChangeError::ReadOnly`], I/O.
pub async fn write_with(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    mode: WriteMode,
    plan: impl FnOnce(&Doc) -> Result<ImportPlan>,
) -> Result<ImportedBoardFile> {
    super::board_load::supported(client, board).await?;
    let connected =
        crate::remote::socket::write::open_with(client, connector, board).await?;
    let plan = if matches!(connected.right(), EditRight::Editable) {
        plan(connected.document())
    } else {
        Err(Error::BoardChange(BoardChangeError::ReadOnly))
    }?;
    let update_id = crate::domain::change::build::update_id(&client.new_id());
    let accepted = match mode {
        WriteMode::DryRun => None,
        WriteMode::Send => Some(
            crate::remote::socket::write::sent(
                connected,
                client,
                plan.update.clone(),
                update_id,
            )
            .await?,
        ),
    };
    Ok(ImportedBoardFile {
        plan,
        update_id,
        accepted,
    })
}
