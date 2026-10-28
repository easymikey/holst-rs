//! A board loaded over the socket: frames read until the document is whole.

#[cfg(test)] mod tests;

use std::time::Duration;

use futures_util::future;
use secrecy::ExposeSecret as _;
use yrs::{
    Doc,
    ReadTxn as _,
    StateVector,
    Transact as _,
    Update,
    updates::decoder::Decode as _,
};

use crate::{
    domain::board::{
        EditRight,
        LoadedBoard,
        state_vector::{ahead, holds},
    },
    effect::{
        board_write::LoadSource,
        server::HolstServer,
        socket::{BoardSocket, SocketConnector, SocketSession},
    },
    error::{Error, Result, board_socket::BoardSocketError},
    protocol::{
        hud,
        hud::{DocumentResponse, SocketFrame, UpdatePayload},
    },
    remote::{client::Client, socket::stream::TungsteniteConnector},
};

/// How long the server may stay silent while a board loads.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest a whole load may take, however steadily the bytes come.
pub const TOTAL_CAP: Duration = Duration::from_mins(10);

/// `wait`, unless the server stays silent for [`IDLE_TIMEOUT`].
async fn idle<T>(wait: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(IDLE_TIMEOUT, wait)
        .await
        .unwrap_or_else(|_| {
            Err(Error::BoardSocket(BoardSocketError::Timeout {
                seconds: IDLE_TIMEOUT.as_secs(),
            }))
        })
}

/// `load`, unless it takes longer than [`TOTAL_CAP`].
async fn capped<T>(load: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(TOTAL_CAP, load)
        .await
        .unwrap_or_else(|_| {
            Err(Error::BoardSocket(BoardSocketError::TotalTimeout {
                seconds: TOTAL_CAP.as_secs(),
            }))
        })
}

#[derive(Debug, Clone)]
struct UnappliedUpdate {
    payload: UpdatePayload,
    announced: StateVector,
}

#[derive(Debug, Clone, Default)]
struct LoadProgress {
    right: Option<EditRight>,
    unapplied: Vec<UnappliedUpdate>,
    announced: StateVector,
    last: bool,
}

/// The board's document over the real socket, with the client's login.
///
/// # Errors
/// No login, a refused board, a closed or slow connection, a bad frame.
pub async fn load(client: &Client, board: &str) -> Result<LoadedBoard> {
    load_with(client, &TungsteniteConnector, board).await
}

/// The board's document over `connector`; only reads, never sends an update.
///
/// # Errors
/// As [`load`], and
/// [`crate::error::board_socket::BoardSocketError::Incomplete`] for a document
/// with gaps.
pub async fn load_with(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
) -> Result<LoadedBoard> {
    load_seeded(client, connector, board, Doc::new())
        .await
        .map(|(loaded, _)| loaded)
}

/// The board loaded into `seed`; a link whose announced state `seed` already
/// holds is not downloaded.
///
/// # Errors
/// As [`load_with`].
pub async fn load_seeded(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
    seed: Doc,
) -> Result<(LoadedBoard, LoadSource)> {
    connected(client, connector, board, seed)
        .await
        .map(|(_, loaded, source)| (loaded, source))
}

/// The user's right and the server's state vector, from the frames of the
/// initial load; `None` when an update came without one. Nothing is downloaded
/// or applied.
///
/// # Errors
/// As [`load_with`].
pub async fn announced(
    client: &Client,
    connector: &dyn SocketConnector,
    board: &str,
) -> Result<(EditRight, Option<StateVector>)> {
    let login = client.socket_session()?;
    capped(announce(connector, &login, board)).await
}

async fn announce(
    connector: &dyn SocketConnector,
    login: &SocketSession,
    board: &str,
) -> Result<(EditRight, Option<StateVector>)> {
    // imperative: reading takes the socket by `&mut`
    let mut socket = requested(connector, login, board).await?;
    // imperative: the load state follows the frames
    let mut progress = LoadProgress::default();
    // imperative: set by any update that has no state vector
    let mut blind = false;
    // imperative: frames come until a board update says none follow
    while !progress.last {
        let message = received(&mut *socket).await?;
        progress = hud::frames(&message)?
            .into_iter()
            .try_fold(progress, |progress, frame| advance(progress, frame, board))?;
        blind = blind
            || progress
                .unapplied
                .iter()
                .any(|update| update.announced.is_empty());
        progress.unapplied.clear();
    }
    let access = progress.right.unwrap_or(EditRight::ReadOnly);
    Ok((access, (!blind).then_some(progress.announced)))
}

async fn requested(
    connector: &dyn SocketConnector,
    login: &SocketSession,
    board: &str,
) -> Result<Box<dyn BoardSocket>> {
    // imperative: sending takes the socket by `&mut`
    let mut socket = idle(connector.open(login)).await?;
    idle(socket.send(hud::request_document(board, login.token.expose_secret())))
        .await?;
    Ok(socket)
}

async fn received(socket: &mut dyn BoardSocket) -> Result<Vec<u8>> {
    idle(socket.receive())
        .await?
        .ok_or(Error::BoardSocket(BoardSocketError::Closed))
}

struct LoadRequest<'a> {
    server: &'a dyn HolstServer,
    connector: &'a dyn SocketConnector,
    session: &'a SocketSession,
    board: &'a str,
    document: &'a Doc,
    seed: &'a StateVector,
}

pub(crate) async fn connected(
    client: &dyn HolstServer,
    connector: &dyn SocketConnector,
    board: &str,
    doc: Doc,
) -> Result<(Box<dyn BoardSocket>, LoadedBoard, LoadSource)> {
    let login = client.socket_session()?;
    let seed = doc.transact().state_vector();
    let request = LoadRequest {
        server: client,
        connector,
        session: &login,
        board,
        document: &doc,
        seed: &seed,
    };
    let (socket, access, source) = capped(load_frames(&request)).await?;
    Ok((
        socket,
        LoadedBoard {
            right: access,
            document: doc,
        },
        source,
    ))
}

async fn load_frames(
    request: &LoadRequest<'_>,
) -> Result<(Box<dyn BoardSocket>, EditRight, LoadSource)> {
    let LoadRequest {
        server: client,
        connector,
        session: login,
        board,
        document: doc,
        seed,
    } = *request;
    // imperative: the socket is read by `&mut` until the load is whole
    let mut socket = requested(connector, login, board).await?;
    // imperative: the state of the load follows the frames as they arrive
    let mut progress = LoadProgress::default();
    // imperative: the state the applied blocks reach grows with each frame
    let mut reached = StateVector::default();
    // imperative: the source of the bytes follows each download
    let mut source = LoadSource::Skipped;
    // imperative: set by any update that has no state vector
    let mut blind = false;
    // imperative: frames come until the document holds the whole initial load
    while !(progress.last && covers(doc, &progress.announced)) {
        let message = received(&mut *socket).await?;
        progress = hud::frames(&message)?
            .into_iter()
            .try_fold(progress, |progress, frame| advance(progress, frame, board))?;
        let waiting = std::mem::take(&mut progress.unapplied);
        blind = blind || waiting.iter().any(|update| update.announced.is_empty());
        (if !blind && ahead(seed, &progress.announced) && progress.last {
            Err(Error::BoardSocket(BoardSocketError::CopyAhead))
        } else {
            Ok(())
        })?;
        let (updates, downloaded) = download_links(client, doc, waiting).await?;
        source = source.plus(downloaded);
        reached.merge(integrate(doc, &updates)?);
    }
    complete(doc, &reached)?;
    let access = progress.right.unwrap_or(EditRight::ReadOnly);
    Ok((socket, access, source))
}

fn advance(
    progress: LoadProgress,
    frame: SocketFrame,
    board: &str,
) -> Result<LoadProgress> {
    let ours = frame.document == board;
    let shared = frame.document == hud::ANY_DOCUMENT;
    match frame.message {
        _ if !ours && !shared => Ok(skipped(progress, &frame.document, "a frame")),
        hud::SocketMessage::Response(DocumentResponse::NotSubscribed {
            code: refusal,
        }) => Err(Error::BoardSocket(BoardSocketError::NotSubscribed {
            code: refusal,
        })),
        hud::SocketMessage::Response(DocumentResponse::Subscribed {
            right: access,
        }) => Ok(LoadProgress {
            right: Some(access),
            ..progress
        }),
        hud::SocketMessage::Update(_) if !ours => {
            Ok(skipped(progress, &frame.document, "an update"))
        }
        hud::SocketMessage::Update(update) => {
            let announced = decoded(&update.state_vector)?;
            // imperative: merging takes the vector by `&mut`
            let mut server = progress.announced;
            server.merge(announced.clone());
            Ok(LoadProgress {
                last: progress.last || update.left == 0,
                announced: server,
                unapplied: progress
                    .unapplied
                    .into_iter()
                    .chain([UnappliedUpdate {
                        payload: update.payload,
                        announced,
                    }])
                    .collect(),
                ..progress
            })
        }
        hud::SocketMessage::Ack(_) => Ok(progress),
        hud::SocketMessage::Unknown { .. } => {
            Err(Error::BoardSocket(BoardSocketError::MalformedFrame {
                what: "an unknown message type",
            }))
        }
    }
}

fn skipped(progress: LoadProgress, document: &str, what: &str) -> LoadProgress {
    tracing::debug!(document, "skipped {what} of another document");
    progress
}

fn decoded(announced: &[u8]) -> Result<StateVector> {
    match announced {
        [] => Ok(StateVector::default()),
        bytes => Ok(StateVector::decode_v1(bytes)?),
    }
}

fn covers(doc: &Doc, server: &StateVector) -> bool {
    holds(&doc.transact().state_vector(), server)
}

async fn download_links(
    client: &dyn HolstServer,
    doc: &Doc,
    updates: Vec<UnappliedUpdate>,
) -> Result<(Vec<Vec<u8>>, u64)> {
    let source = future::try_join_all(updates.into_iter().map(|waiting| {
        let held = !waiting.announced.is_empty() && covers(doc, &waiting.announced);
        async move {
            match (waiting.payload, held) {
                (UpdatePayload::Inline(bytes), _) => Ok(Some((bytes, 0))),
                (UpdatePayload::Link(_), true) => {
                    tracing::debug!(
                        "skipped a board link: the cached copy holds what it announces"
                    );
                    Ok(None)
                }
                (UpdatePayload::Link(link), false) => {
                    client.download_link(&link).await.map(|bytes| {
                        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                        Some((bytes, size))
                    })
                }
            }
        }
    }))
    .await?;
    let downloaded = source.iter().flatten().map(|(_, size)| size).sum();
    let loaded = source
        .into_iter()
        .flatten()
        .map(|(bytes, _)| bytes)
        .collect();
    Ok((loaded, downloaded))
}

fn integrate(doc: &Doc, updates: &[Vec<u8>]) -> Result<StateVector> {
    updates.iter().filter(|bytes| !bytes.is_empty()).try_fold(
        StateVector::default(),
        |mut reached, bytes| {
            let update = Update::decode_v2(bytes)?;
            update.insertions(true).iter().for_each(|(client, ranges)| {
                let end = ranges
                    .iter()
                    .map(|range| range.end)
                    .max()
                    .unwrap_or_default();
                reached.set_max(*client, end);
            });
            doc.transact_mut().apply_update(update)?;
            Ok(reached)
        },
    )
}

fn complete(doc: &Doc, reached: &StateVector) -> Result<()> {
    let holes = !covers(doc, reached);
    let missing = holes || doc.transact().has_missing_updates();
    if missing {
        Err(Error::BoardSocket(BoardSocketError::Incomplete))
    } else {
        Ok(())
    }
}
