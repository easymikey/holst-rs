// The hud WebSocket: serves a filled board.

use std::{io, sync::PoisonError};

use futures_util::{StreamExt as _, future};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use yrs::{
    Doc,
    ReadTxn as _,
    StateVector,
    Transact as _,
    Update,
    updates::{decoder::Decode as _, encoder::Encode as _},
};

use crate::fake::{Shared, State, login::Sessions};

const REQUEST_LINE: &str = "GET /hud/ws";
const END_OF_HEAD: &str = "\r\n\r\n";
const HEAD_LIMIT: usize = 8192;
const ENCODED_V2: &str = "encV2";
const UPDATE: u64 = 0;
const REQUEST_DOCUMENT: u64 = 1;
const ACK: u64 = 2;
const RESPONSE: u64 = 3;
const INLINE: u64 = 0;
const LINK: u64 = 1;
const COMMENTS_ROOM: &str = "$comments";

/// A document the socket serves: a board, or the comments room of one.
struct Room {
    name: String,
    document: Doc,
    read_only: bool,
}

/// The room called `name`, if the board it belongs to exists. Whoever may
/// only comment can write in the comments room of a board and not in the board.
fn room_of(name: &str, state: &mut State) -> Option<Room> {
    match name.strip_suffix(COMMENTS_ROOM) {
        Some(id) => {
            let board = state.tree.board(id)?;
            let commentator = state.access.role_of(id, state.comments.user())
                == Some("commentator".to_owned());
            Some(Room {
                name: name.to_owned(),
                document: state.comments.room(id),
                read_only: board.read_only && !commentator,
            })
        }
        None => state.tree.board(name).map(|board| Room {
            name: board.id,
            document: board.document,
            read_only: board.read_only,
        }),
    }
}

/// Tells whether the request waiting on the stream is a socket handshake that
/// carries the cookie of a live session; any other request goes to the HTTP
/// routes.
pub(crate) async fn is_handshake(stream: &TcpStream, state: &Shared) -> bool {
    let head = peek_head(stream).await;
    head.starts_with(REQUEST_LINE)
        && head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .any(|(name, value)| {
                name.eq_ignore_ascii_case("cookie")
                    && state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .sessions
                        .accepts(value)
            })
}

/// Looks at the request without consuming it, until its whole head arrived.
async fn peek_head(stream: &TcpStream) -> String {
    let mut buffer = [0_u8; HEAD_LIMIT];
    // imperative: a head may arrive in pieces, and peeking consumes nothing
    loop {
        let read = stream.peek(&mut buffer).await.unwrap_or_default();
        let head = String::from_utf8_lossy(buffer.get(..read).unwrap_or_default())
            .into_owned();
        if head.contains(END_OF_HEAD) || read == 0 || read == HEAD_LIMIT {
            break head;
        }
        tokio::task::yield_now().await;
    }
}

/// Answers each `RequestDocument` with the response and the whole document,
/// and each inline `Update` with an ack once it is in the document.
pub(crate) async fn serve(stream: TcpStream, state: Shared) -> io::Result<()> {
    let socket = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(io::Error::other)?;
    let (outgoing, incoming) = socket.split();
    incoming
        .filter_map(|message| {
            future::ready(
                message
                    .ok()
                    .and_then(|message| reply(&message.into_data(), &state)),
            )
        })
        .map(|bytes| Ok(Message::binary(bytes)))
        .forward(outgoing)
        .await
        .map_err(io::Error::other)
}

fn reply(message: &[u8], state: &Shared) -> Option<Vec<u8>> {
    let (name, rest) = read_string(message)?;
    let (kind, rest) = read_uint(rest)?;
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    match kind {
        REQUEST_DOCUMENT => {
            let room = room_of(name, &mut state);
            respond_to_request(name, rest, room.as_ref(), &state.sessions)
        }
        UPDATE => accept_update(name, rest, &mut state),
        _ => None,
    }
}

/// Applies an update the client sent, inline or uploaded first, then acks it;
/// a read-only or unknown board refuses it the way it refuses a request for
/// the document.
fn accept_update(name: &str, rest: &[u8], state: &mut State) -> Option<Vec<u8>> {
    let (id, rest) = read_uint(rest)?;
    let (kind, rest) = read_uint(rest)?;
    state.updates += 1;
    match (kind, room_of(name, state)) {
        (_, None) => Some(refusal(name, "internal-error")),
        (_, Some(room)) if room.read_only => Some(refusal(name, "forbidden")),
        (INLINE | LINK, Some(room)) => {
            let bytes = match kind {
                INLINE => read_bytes(rest).map(|(bytes, _)| bytes.to_vec()),
                _ => read_uint(rest)
                    .and_then(|(_, rest)| read_string(rest))
                    .and_then(|(link, _)| state.files.transfers.uploaded(link)),
            };
            Some(
                match bytes.and_then(|bytes| merge(&room.document, &bytes)) {
                    Some(()) => ack(name, id),
                    None => refusal(name, "internal-error"),
                },
            )
        }
        (_, Some(_)) => None,
    }
}

fn merge(document: &Doc, bytes: &[u8]) -> Option<()> {
    let update = Update::decode_v2(bytes).ok()?;
    document.transact_mut().apply_update(update).ok()
}

fn ack(name: &str, id: u64) -> Vec<u8> {
    [write_string(name), write_uint(ACK), write_uint(id)].concat()
}

fn refusal(name: &str, error: &str) -> Vec<u8> {
    response(name, &json!({ "error": error }))
}

fn respond_to_request(
    name: &str,
    rest: &[u8],
    room: Option<&Room>,
    sessions: &Sessions,
) -> Option<Vec<u8>> {
    let (login, _) = read_string(rest)?;
    let token = serde_json::from_str::<Value>(login).ok()?;
    let accepted = token
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|token| sessions.accepts_token(token));
    Some(match (accepted, room) {
        (true, Some(room)) => [
            response(name, &json!({ "readOnly": room.read_only, "status": "ok" })),
            whole_document(room),
        ]
        .concat(),
        (true, None) => refusal(name, "internal-error"),
        (false, _) => refusal(name, "forbidden"),
    })
}

fn response(name: &str, body: &Value) -> Vec<u8> {
    [
        write_string(name),
        write_uint(RESPONSE),
        write_string(&body.to_string()),
    ]
    .concat()
}

/// One update with `leftUpdates` 0 that carries everything the board holds.
fn whole_document(room: &Room) -> Vec<u8> {
    let (state, update) = {
        let txn = room.document.transact();
        (
            txn.state_vector().encode_v1(),
            txn.encode_state_as_update_v2(&StateVector::default()),
        )
    };
    [
        write_string(ENCODED_V2),
        write_string(&room.name),
        write_uint(UPDATE),
        write_uint(0),
        write_uint(0),
        write_uint(1),
        write_bytes(&state),
        write_uint(INLINE),
        write_bytes(&update),
    ]
    .concat()
}

fn write_uint(value: u64) -> Vec<u8> {
    let low = u8::try_from(value & 0x7f).unwrap_or_default();
    match value >> 7 {
        0 => vec![low],
        rest => [vec![low | 0x80], write_uint(rest)].concat(),
    }
}

fn write_bytes(bytes: &[u8]) -> Vec<u8> {
    [
        write_uint(u64::try_from(bytes.len()).unwrap_or_default()),
        bytes.to_vec(),
    ]
    .concat()
}

fn write_string(text: &str) -> Vec<u8> {
    write_bytes(text.as_bytes())
}

fn read_uint(input: &[u8]) -> Option<(u64, &[u8])> {
    let (&byte, rest) = input.split_first()?;
    let low = u64::from(byte & 0x7f);
    match byte & 0x80 {
        0 => Some((low, rest)),
        _ => read_uint(rest).map(|(high, rest)| (low | high << 7, rest)),
    }
}

fn read_bytes(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, rest) = read_uint(input)?;
    let length = usize::try_from(length).ok()?;
    Some((rest.get(..length)?, rest.get(length..)?))
}

fn read_string(input: &[u8]) -> Option<(&str, &[u8])> {
    let (bytes, rest) = read_bytes(input)?;
    Some((std::str::from_utf8(bytes).ok()?, rest))
}
