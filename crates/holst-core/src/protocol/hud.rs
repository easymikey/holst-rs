//! Frames of the board document socket `/hud/ws`: lib0 encoding, no I/O.

use super::lib0::{
    read_var_bytes,
    read_var_int,
    read_var_string,
    read_var_uint,
    var_bytes,
    var_string,
    var_uint,
};
use crate::{
    domain::board::EditRight,
    error::{
        Error,
        Result,
        board_socket::{BoardSocketError, SocketErrorCode},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateEncoding {
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdatePayload {
    Inline(Vec<u8>),
    Link(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerUpdate {
    pub encoding: UpdateEncoding,
    pub server_time: u64,
    pub left: i64,
    pub id: u64,
    pub state_vector: Vec<u8>,
    pub payload: UpdatePayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DocumentResponse {
    Subscribed { right: EditRight },
    NotSubscribed { code: SocketErrorCode },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SocketMessage {
    Update(ServerUpdate),
    Ack(u64),
    Response(DocumentResponse),
    Unknown { kind: u64, dropped: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SocketFrame {
    pub document: String,
    pub message: SocketMessage,
}

pub(crate) const ANY_DOCUMENT: &str = "00000000-0000-0000-0000-000000000000";

pub(crate) const UPDATE: u64 = 0;
const REQUEST_DOCUMENT: u64 = 1;
pub(crate) const ACK: u64 = 2;
pub(crate) const RESPONSE: u64 = 3;
pub(crate) const INLINE: u64 = 0;
pub(crate) const LINK: u64 = 1;
pub(crate) const ENC_V2: &str = "encV2";

#[must_use]
pub(crate) fn request_document(document: &str, token: &str) -> Vec<u8> {
    let login = serde_json::json!({ "token": token }).to_string();
    [
        var_string(document),
        var_uint(REQUEST_DOCUMENT),
        var_string(&login),
        var_uint(0),
    ]
    .concat()
}

pub(crate) const MAX_INLINE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutgoingUpdate {
    Inline(Vec<u8>),
    Uploaded { size: u64, link: String },
}

#[must_use]
pub(crate) fn upload_link(document: &str, id: u64) -> String {
    format!("{document}/{id}")
}

#[must_use]
pub(crate) fn update_frame(
    document: &str,
    id: u64,
    payload: &OutgoingUpdate,
) -> Vec<u8> {
    let body = match payload {
        OutgoingUpdate::Inline(bytes) => [var_uint(INLINE), var_bytes(bytes)].concat(),
        OutgoingUpdate::Uploaded { size, link } => {
            [var_uint(LINK), var_uint(*size), var_string(link)].concat()
        }
    };
    [var_string(document), var_uint(UPDATE), var_uint(id), body].concat()
}

/// A client update frame, read as the server would: document, id, payload.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::MalformedFrame`] for
/// anything but one well-formed update frame.
#[cfg(test)]
pub(crate) fn read_update_frame(input: &[u8]) -> Result<(String, u64, OutgoingUpdate)> {
    let (document, rest) = read_var_string(input)?;
    let (kind, rest) = read_var_uint(rest)?;
    let (id, rest) = match kind {
        UPDATE => read_var_uint(rest)?,
        _ => Err(Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "a client frame that is not an update",
        }))?,
    };
    let (how, rest) = read_var_uint(rest)?;
    let (payload, rest) = match how {
        INLINE => read_var_bytes(rest)
            .map(|(bytes, rest)| (OutgoingUpdate::Inline(bytes.to_vec()), rest))?,
        LINK => {
            let (size, rest) = read_var_uint(rest)?;
            read_var_string(rest).map(|(link, rest)| {
                (
                    OutgoingUpdate::Uploaded {
                        size,
                        link: link.to_owned(),
                    },
                    rest,
                )
            })?
        }
        _ => Err(Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "an unknown update kind",
        }))?,
    };
    rest.is_empty()
        .then(|| (document.to_owned(), id, payload))
        .ok_or(Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "bytes after the update",
        }))
}

/// Every frame of one WebSocket message; a message may carry several.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::MalformedFrame`] for a
/// malformed frame.
pub(crate) fn frames(message: &[u8]) -> Result<Vec<SocketFrame>> {
    // Each step reads one frame off the rest; a failed step ends the walk.
    std::iter::successors(Some(Ok((None, message))), |step| match step {
        Ok((_, rest)) if !rest.is_empty() => {
            Some(frame(rest).map(|(frame, rest)| (Some(frame), rest)))
        }
        _ => None,
    })
    .filter_map(|step| step.map(|(frame, _)| frame).transpose())
    .collect()
}

fn frame(input: &[u8]) -> Result<(SocketFrame, &[u8])> {
    let (first, rest) = read_var_string(input)?;
    let (encoding, document, rest) = match first {
        ENC_V2 => read_var_string(rest)
            .map(|(name, rest)| (UpdateEncoding::V2, name, rest))?,
        name => (UpdateEncoding::V1, name, rest),
    };
    let (kind, rest) = read_var_uint(rest)?;
    let (message, rest) = match kind {
        UPDATE => update(encoding, rest)
            .map(|(update, rest)| (SocketMessage::Update(update), rest))?,
        ACK => read_var_uint(rest).map(|(id, rest)| (SocketMessage::Ack(id), rest))?,
        RESPONSE => answer(rest)
            .map(|(answer, rest)| (SocketMessage::Response(answer), rest))?,
        // Its length is unknown, so nothing after it can be read.
        kind => (
            SocketMessage::Unknown {
                kind,
                dropped: rest.len(),
            },
            &[][..],
        ),
    };
    Ok((
        SocketFrame {
            document: document.to_owned(),
            message,
        },
        rest,
    ))
}

fn update(encoding: UpdateEncoding, input: &[u8]) -> Result<(ServerUpdate, &[u8])> {
    let (server_time, rest) = read_var_uint(input)?;
    let (left, rest) = read_var_int(rest)?;
    let (id, rest) = read_var_uint(rest)?;
    let (state_vector, rest) = match encoding {
        UpdateEncoding::V2 => {
            read_var_bytes(rest).map(|(vector, rest)| (vector.to_vec(), rest))?
        }
        UpdateEncoding::V1 => (Vec::new(), rest),
    };
    let (kind, rest) = read_var_uint(rest)?;
    let (payload, rest) = match kind {
        INLINE => read_var_bytes(rest)
            .map(|(bytes, rest)| (UpdatePayload::Inline(bytes.to_vec()), rest))?,
        LINK => read_var_string(rest)
            .map(|(link, rest)| (UpdatePayload::Link(link.to_owned()), rest))?,
        _ => Err(Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "an unknown update kind",
        }))?,
    };
    Ok((
        ServerUpdate {
            encoding,
            server_time,
            left,
            id,
            state_vector,
            payload,
        },
        rest,
    ))
}

fn answer(input: &[u8]) -> Result<(DocumentResponse, &[u8])> {
    #[derive(serde::Deserialize)]
    struct Reply {
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default, rename = "readOnly")]
        read_only: Option<bool>,
    }
    let (text, rest) = read_var_string(input)?;
    let reply: Reply = serde_json::from_str(text).map_err(|_| {
        Error::BoardSocket(BoardSocketError::MalformedFrame {
            what: "an answer that is not JSON",
        })
    })?;
    let response = match (reply.error, reply.status) {
        (Some(code), _) => DocumentResponse::NotSubscribed {
            code: SocketErrorCode::from_code(code),
        },
        (None, Some(status)) if status != "ok" => DocumentResponse::NotSubscribed {
            code: SocketErrorCode::from_code(status),
        },
        (None, _) => DocumentResponse::Subscribed {
            right: match reply.read_only {
                Some(false) => EditRight::Editable,
                Some(true) | None => EditRight::ReadOnly,
            },
        },
    };
    Ok((response, rest))
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

    use rstest::rstest;

    use crate::{
        domain::board::EditRight,
        error::{
            Error,
            Result,
            board_socket::{BoardSocketError, SocketErrorCode},
        },
        protocol::{
            hud::{
                ACK,
                DocumentResponse,
                ENC_V2,
                INLINE,
                LINK,
                RESPONSE,
                ServerUpdate,
                SocketFrame,
                SocketMessage,
                UPDATE,
                UpdatePayload,
                frames,
                request_document,
            },
            lib0::{
                read_var_int,
                read_var_string,
                read_var_uint,
                var_bytes,
                var_string,
                var_uint,
            },
        },
    };

    #[rstest]
    #[case(0, &[0x00])]
    #[case(127, &[0x7f])]
    #[case(128, &[0x80, 0x01])]
    #[case(300, &[0xac, 0x02])]
    fn var_uint_matches_lib0(#[case] value: u64, #[case] bytes: &[u8]) {
        assert_eq!(var_uint(value), bytes);
        assert_eq!(read_var_uint(bytes).ok(), Some((value, &[][..])));
    }

    #[rstest]
    #[case(&[0x05], 5)]
    #[case(&[0x45], -5)]
    #[case(&[0x81, 0x01], 65)]
    #[case(&[0xc1, 0x01], -65)]
    fn signed_varint_follows_lib0(#[case] bytes: &[u8], #[case] value: i64) {
        assert_eq!(read_var_int(bytes).ok(), Some((value, &[][..])));
    }

    #[test]
    fn request_carries_name_type_token_and_zero() {
        let frame = request_document("b1", "tok");
        let (name, rest) = read_var_string(&frame).unwrap_or(("", &[]));
        let (kind, rest) = read_var_uint(rest).unwrap_or((9, &[]));
        let (login, rest) = read_var_string(rest).unwrap_or(("", &[]));
        assert_eq!(
            (name, kind, login, rest),
            ("b1", 1, r#"{"token":"tok"}"#, &[0_u8][..])
        );
    }

    #[test]
    fn several_frames_in_one_message_are_read_in_order() -> Result<()> {
        let answer = [
            var_string("b1"),
            var_uint(RESPONSE),
            var_string(r#"{"readOnly":false,"status":"ok"}"#),
        ]
        .concat();
        let update = [
            var_string(ENC_V2),
            var_string("b1"),
            var_uint(UPDATE),
            var_uint(42),
            vec![0x00],
            var_uint(7),
            var_bytes(&[1, 2]),
            var_uint(INLINE),
            var_bytes(&[9, 9, 9]),
        ]
        .concat();
        let link = [
            var_string(ENC_V2),
            var_string("b1"),
            var_uint(UPDATE),
            var_uint(43),
            vec![0x01],
            var_uint(8),
            var_bytes(&[]),
            var_uint(LINK),
            var_string("https://cdn/x"),
        ]
        .concat();
        let read: Vec<SocketMessage> = frames(&[answer, update, link].concat())?
            .into_iter()
            .map(|frame| frame.message)
            .collect();
        assert!(matches!(
            read.as_slice(),
            [
                SocketMessage::Response(DocumentResponse::Subscribed {
                    right: EditRight::Editable
                }),
                SocketMessage::Update(ServerUpdate {
                    left: 0,
                    id: 7,
                    payload: UpdatePayload::Inline(_),
                    ..
                }),
                SocketMessage::Update(ServerUpdate {
                    left: 1,
                    id: 8,
                    payload: UpdatePayload::Link(_),
                    ..
                }),
            ]
        ));
        Ok(())
    }

    #[test]
    fn refusal_names_the_reason() -> Result<()> {
        let message = [
            var_string("b1"),
            var_uint(RESPONSE),
            var_string(r#"{"error":"forbidden"}"#),
        ]
        .concat();
        let read: Vec<SocketMessage> = frames(&message)?
            .into_iter()
            .map(|frame| frame.message)
            .collect();
        assert_eq!(
            read,
            [SocketMessage::Response(DocumentResponse::NotSubscribed {
                code: SocketErrorCode::Forbidden
            })]
        );
        Ok(())
    }

    #[rstest]
    #[case::cut_varint(&[0x80])]
    #[case::long_string(&[0x05, b'a'])]
    fn malformed_frames_are_errors(#[case] message: &[u8]) {
        assert!(matches!(
            frames(message),
            Err(Error::BoardSocket(BoardSocketError::MalformedFrame { .. }))
        ));
    }

    #[test]
    fn an_unknown_type_ends_the_message() -> Result<()> {
        let message = [
            var_string("other"),
            var_uint(9),
            vec![1, 2, 3],
            var_string("b1"),
            var_uint(ACK),
            var_uint(5),
        ]
        .concat();
        let read = frames(&message)?;
        assert_eq!(
            read,
            [SocketFrame {
                document: "other".into(),
                message: SocketMessage::Unknown {
                    kind: 9,
                    dropped: message.len() - 7
                },
            }]
        );
        Ok(())
    }

    #[rstest]
    #[case::editable(r#"{"status":"ok","readOnly":false}"#, Some(EditRight::Editable))]
    #[case::read_only(r#"{"status":"ok","readOnly":true}"#, Some(EditRight::ReadOnly))]
    #[case::no_right_named(r#"{"status":"ok"}"#, Some(EditRight::ReadOnly))]
    #[case::no_status(r#"{"readOnly":false}"#, Some(EditRight::Editable))]
    #[case::other_status(r#"{"status":"busy","readOnly":false}"#, None)]
    fn only_an_ok_answer_subscribes(
        #[case] json: &str,
        #[case] right: Option<EditRight>,
    ) -> Result<()> {
        let message = [var_string("b1"), var_uint(RESPONSE), var_string(json)].concat();
        let read: Vec<SocketMessage> = frames(&message)?
            .into_iter()
            .map(|frame| frame.message)
            .collect();
        let expected = right.map_or_else(
            || DocumentResponse::NotSubscribed {
                code: SocketErrorCode::Other("busy".into()),
            },
            |right| DocumentResponse::Subscribed { right },
        );
        assert_eq!(read, [SocketMessage::Response(expected)]);
        Ok(())
    }
}
