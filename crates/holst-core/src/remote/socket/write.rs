//! A change sent over the open socket and confirmed by the server.

use std::time::Duration;

use futures_util::future;
use yrs::{
    ClientID,
    Doc,
    ReadTxn as _,
    StateVector,
    Transact as _,
    Update,
    updates::decoder::Decode as _,
};

use crate::{
    domain::{
        board::EditRight,
        change::{receipt::Delivery, value::ChangedObject},
    },
    effect::{
        board_write::{LoadSource, OpenedBoard, WriteInterpreter, WriteMode},
        board_write_plan::{Release, WritePlan},
        http::BoxFuture,
        socket::{BoardSocket, SocketConnector},
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        board_socket::BoardSocketError,
        http::HttpError,
    },
    protocol::{
        hud,
        hud::{DocumentResponse, SocketFrame},
    },
    remote::{
        client::Client,
        socket::{load::connected, stream::TungsteniteConnector},
    },
};

pub const ACCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// A board loaded over a socket that stays open for one update.
pub struct ConnectedBoard {
    socket: Box<dyn BoardSocket>,
    board: String,
    right: EditRight,
    document: Doc,
}

impl std::fmt::Debug for ConnectedBoard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectedBoard")
            .field("board", &self.board)
            .finish_non_exhaustive()
    }
}

/// An update sent and not yet confirmed.
pub struct SentUpdate {
    socket: Box<dyn BoardSocket>,
    board: String,
    id: u64,
}

impl std::fmt::Debug for SentUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SentUpdate")
            .field("board", &self.board)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Load the board over `connector` and keep the socket open.
///
/// # Errors
/// As [`load_with`](fn@crate::remote::socket::load::load_with).
pub async fn open_with(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
) -> Result<ConnectedBoard> {
    let seed = writer_document(client, None)?;
    open_seeded(client, connector, board, seed)
        .await
        .map(|(board, _)| board)
}

/// A document to write from: a fresh 31-bit client id, the saved `state` in it.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::Decode`] or
/// [`crate::error::board_socket::BoardSocketError::UpdateNotMerged`] for a
/// broken state.
pub fn writer_document(client: &Client, state: Option<&[u8]>) -> Result<Doc> {
    // A 31-bit client id like the web client's; `own_client` checks it is free.
    let doc = Doc::with_client_id(crate::domain::change::build::client_id(
        &client.new_id(),
        |_| false,
    ));
    state
        .map(|state| {
            doc.transact_mut()
                .apply_update(Update::decode_v2(state)?)
                .map_err(Error::from)
        })
        .transpose()?;
    Ok(doc)
}

/// Load the board into `seed` (see [`writer_document`]) and keep the socket
/// open.
///
/// # Errors
/// As [`load_with`](fn@crate::remote::socket::load::load_with).
pub async fn open_seeded(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    seed: Doc,
) -> Result<(ConnectedBoard, LoadSource)> {
    let (socket, loaded, fetched) = connected(client, connector, board, seed).await?;
    let connected = ConnectedBoard {
        socket,
        board: board.to_owned(),
        right: loaded.right,
        document: with_free_client_id(&loaded.document)?,
    };
    Ok((connected, fetched))
}

pub(crate) fn with_free_client_id(doc: &Doc) -> Result<Doc> {
    let taken = doc.transact().state_vector();
    let used = |id: u64| taken.contains_client(&ClientID::new(id));
    let mine = doc.client_id().get();
    let kept = (!used(mine)).then(|| doc.clone());
    let seed = mine.to_string();
    kept.map_or_else(
        || moved(doc, crate::domain::change::build::client_id(&seed, used)),
        Ok,
    )
}

fn moved(doc: &Doc, id: u64) -> Result<Doc> {
    let copy = Doc::with_client_id(id);
    let whole = doc
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    copy.transact_mut()
        .apply_update(Update::decode_v2(&whole)?)?;
    Ok(copy)
}

impl ConnectedBoard {
    /// The user's right on the board.
    #[must_use]
    pub const fn right(&self) -> EditRight {
        self.right
    }

    /// The document as loaded; a change is built on it.
    #[must_use]
    pub const fn document(&self) -> &Doc {
        &self.document
    }

    /// Send `update` with `id`: inline, or uploaded first when it is large.
    ///
    /// # Errors
    /// The upload's HTTP errors, or
    /// [`crate::error::board_change::BoardChangeError::NotAccepted`] once it
    /// may be sent.
    pub async fn send(
        self,
        client: &Client,
        update: Vec<u8>,
        id: u64,
    ) -> Result<SentUpdate> {
        let Self {
            mut socket, board, ..
        } = self;
        let payload = match update.len() {
            large if large > hud::MAX_INLINE => upload(client, &board, id, update)
                .await
                .map_err(|error| upload_error(id, error))?,
            _ => hud::OutgoingUpdate::Inline(update),
        };
        socket
            .send(hud::update_frame(&board, id, &payload))
            .await
            .map_err(|cause| unconfirmed(id, cause))?;
        Ok(SentUpdate { socket, board, id })
    }
}

pub(crate) fn upload_error(id: u64, error: Error) -> Error {
    match error {
        Error::Http(HttpError::NoResponse { .. }) => unconfirmed(id, error),
        Error::Http(_)
        | Error::Login(_)
        | Error::Input(_)
        | Error::LocalData(_)
        | Error::Lookup(_)
        | Error::BoardSocket(_)
        | Error::BoardChange(_)
        | Error::Picture(_)
        | Error::StoredFile(_) => error,
    }
}

fn unconfirmed(id: u64, cause: Error) -> Error {
    Error::BoardChange(BoardChangeError::NotAccepted {
        update: id,
        objects: Vec::new(),
        cause: Box::new(cause),
    })
}

fn with_objects(error: Error, objects: &[ChangedObject]) -> Error {
    match error {
        Error::BoardChange(BoardChangeError::NotAccepted { update, cause, .. }) => {
            Error::BoardChange(BoardChangeError::NotAccepted {
                update,
                objects: objects.iter().map(|o| o.id.clone()).collect(),
                cause,
            })
        }
        Error::Login(_)
        | Error::Http(_)
        | Error::Input(_)
        | Error::LocalData(_)
        | Error::Lookup(_)
        | Error::BoardSocket(_)
        | Error::BoardChange(_)
        | Error::Picture(_)
        | Error::StoredFile(_) => error,
    }
}

async fn upload(
    client: &Client,
    board: &str,
    id: u64,
    update: Vec<u8>,
) -> Result<hud::OutgoingUpdate> {
    let size = u64::try_from(update.len()).unwrap_or(u64::MAX);
    crate::remote::api::hud::upload(client, board, id, update).await?;
    Ok(hud::OutgoingUpdate::Uploaded {
        size,
        link: hud::upload_link(board, id),
    })
}

impl SentUpdate {
    /// Wait for the server's ack of this update.
    ///
    /// # Errors
    /// [`crate::error::board_change::BoardChangeError::NotAccepted`] around a
    /// timeout, a close, a refusal or a bad frame.
    pub async fn accepted(mut self) -> Result<()> {
        tokio::time::timeout(
            ACCEPT_TIMEOUT,
            ack(&mut *self.socket, &self.board, self.id),
        )
        .await
        .map_err(|_| {
            Error::BoardChange(BoardChangeError::AcceptTimeout {
                seconds: ACCEPT_TIMEOUT.as_secs(),
            })
        })
        .and_then(|accepted| accepted)
        .map_err(|cause| unconfirmed(self.id, cause))?;
        Ok(())
    }
}

async fn ack(socket: &mut dyn BoardSocket, board: &str, id: u64) -> Result<()> {
    // imperative: frames come until the server acks our update
    let mut accepted = Accepted::Waiting;
    // imperative: same loop
    while accepted == Accepted::Waiting {
        let message = socket
            .receive()
            .await?
            .ok_or(Error::BoardChange(BoardChangeError::ClosedBeforeAccept))?;
        accepted = hud::frames(&message)?
            .into_iter()
            .try_fold(accepted, |seen, frame| acked_by(seen, frame, board, id))?;
    }
    Ok(())
}

/// Whether the server has acked our update yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Accepted {
    Waiting,
    Received,
}

fn acked_by(
    seen: Accepted,
    frame: SocketFrame,
    board: &str,
    id: u64,
) -> Result<Accepted> {
    let ours = frame.document == board || frame.document == hud::ANY_DOCUMENT;
    match frame.message {
        _ if !ours => {
            tracing::debug!(
                document = frame.document,
                "skipped a frame of another document"
            );
            Ok(seen)
        }
        hud::SocketMessage::Ack(got) => Ok(match (seen, got == id) {
            (Accepted::Received, _) | (Accepted::Waiting, true) => Accepted::Received,
            (Accepted::Waiting, false) => Accepted::Waiting,
        }),
        hud::SocketMessage::Response(DocumentResponse::NotSubscribed {
            code: refusal,
        }) => Err(Error::BoardSocket(BoardSocketError::NotSubscribed {
            code: refusal,
        })),
        hud::SocketMessage::Unknown { .. } => {
            Err(Error::BoardSocket(BoardSocketError::MalformedFrame {
                what: "an unknown message type",
            }))
        }
        hud::SocketMessage::Response(DocumentResponse::Subscribed { .. })
        | hud::SocketMessage::Update(_) => Ok(seen),
    }
}

/// Carries a plan over the board socket: sends it, or only reports it.
#[derive(Clone, Copy)]
pub struct WriteOver<'a> {
    connector: &'a dyn SocketConnector,
    mode: WriteMode,
}

impl<'a> WriteOver<'a> {
    /// Sends a released plan over `connector`.
    #[must_use]
    pub const fn sender(connector: &'a dyn SocketConnector) -> Self {
        Self {
            connector,
            mode: WriteMode::Send,
        }
    }

    /// Reports the plan over `connector` and sends nothing.
    #[must_use]
    pub const fn reporter(connector: &'a dyn SocketConnector) -> Self {
        Self {
            connector,
            mode: WriteMode::DryRun,
        }
    }
}

impl std::fmt::Debug for WriteOver<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WriteOver")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// The connector over the real network: the board socket of the Holst server.
#[must_use]
pub fn connector() -> &'static dyn SocketConnector {
    &TungsteniteConnector
}

async fn opened(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    seed: Doc,
) -> Result<OpenedBoard<ConnectedBoard>> {
    let (connected, fetched) = open_seeded(client, connector, board, seed).await?;
    Ok(OpenedBoard {
        document: connected.document().clone(),
        right: connected.right(),
        source: fetched,
        connection: connected,
    })
}

impl WriteInterpreter for WriteOver<'_> {
    type Connection = ConnectedBoard;

    fn open<'a>(
        &'a self,
        client: &'a Client,
        board: &'a str,
        seed: Doc,
    ) -> BoxFuture<'a, Result<OpenedBoard<ConnectedBoard>>> {
        Box::pin(opened(client, self.connector, board, seed))
    }

    fn carry<'a>(
        &'a self,
        client: &'a Client,
        connection: ConnectedBoard,
        plan: &'a WritePlan,
    ) -> BoxFuture<'a, Result<Delivery>> {
        let sending = match self.mode {
            WriteMode::Send => matches!(plan.release, Release::Released).then(|| {
                sent(
                    connection,
                    client,
                    plan.change.update.clone(),
                    plan.update_id,
                )
            }),
            WriteMode::DryRun => None,
        };
        Box::pin(async move {
            future::OptionFuture::from(sending)
                .await
                .transpose()
                .map(|sent| sent.map_or(Delivery::Built, |()| Delivery::Accepted))
                .map_err(|error| with_objects(error, &plan.change.objects))
        })
    }
}

/// Send `update` over the synced socket and wait for its ack.
///
/// # Errors
/// As [`ConnectedBoard::send`] and [`SentUpdate::accepted`].
pub async fn sent(
    board: ConnectedBoard,
    client: &Client,
    update: Vec<u8>,
    id: u64,
) -> Result<()> {
    board.send(client, update, id).await?.accepted().await
}
