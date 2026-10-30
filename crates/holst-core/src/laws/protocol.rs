use std::time::Duration;

use proptest::{
    collection::vec,
    option,
    prelude::{Strategy, any, prop_assert, prop_assert_eq, prop_oneof, proptest},
    sample::{Index, select},
};
use strum::IntoEnumIterator as _;

use crate::{
    domain::board::EditRight,
    effect::http::HttpMethod,
    error::board_socket::SocketErrorCode,
    laws::{config, encoded, uuid, var_int},
    protocol::{
        hud,
        hud::{
            DocumentResponse,
            ServerUpdate,
            SocketFrame,
            SocketMessage,
            UpdateEncoding,
            UpdatePayload,
        },
        lib0,
    },
    remote::client::{
        ERROR_BODY_LIMIT,
        MAX_ATTEMPTS,
        MAX_RETRY_WAIT,
        exchange::scrub_secrets,
        retry_wait,
    },
};

proptest! {
    #![proptest_config(config())]

    /// Pause ≤ `MAX_RETRY_WAIT`, only 429 and GET 502–504, ≤ `MAX_ATTEMPTS`.
    #[test]
    fn retries_are_bounded(
        method in select(HttpMethod::iter().collect::<Vec<_>>()),
        status in prop_oneof![100_u16..600, select(&[429_u16, 502, 503, 504][..])],
        retry_after in option::of((0_u64..120).prop_map(Duration::from_secs)),
        attempt in prop_oneof![0_u32..8, any::<u32>()],
    ) {
        let wait = retry_wait(method, status, retry_after, attempt);
        let retryable = status == 429 || (method == HttpMethod::Get && (502..=504).contains(&status));
        prop_assert!(wait.is_none_or(|wait| wait <= MAX_RETRY_WAIT));
        prop_assert!(wait.is_none() || (retryable && attempt.saturating_add(1) < MAX_ATTEMPTS));
        let attempts = std::iter::successors(Some(0_u32), |&previous| {
            retry_wait(method, status, retry_after, previous).map(|_| previous + 1)
        })
        .count();
        prop_assert!(attempts <= usize::try_from(MAX_ATTEMPTS).unwrap_or(usize::MAX));
    }

    /// No token in the error text, and the length is bounded.
    #[test]
    fn scrubbed_body_hides_token_and_is_bounded(
        token in "[A-Za-z0-9_-]{1,40}",
        pieces in vec(prop_oneof![".{0,40}", "\\PC{0,1500}"], 0..4),
    ) {
        let body = pieces.join(&token);
        let body = format!("{token}{body}{token}");
        let out = scrub_secrets(&body, Some(token.as_str()));
        prop_assert!(!out.contains(&token));
        prop_assert!(out.len() <= ERROR_BODY_LIMIT + '…'.len_utf8());
        let replaced = body.replace(&token, "***");
        prop_assert!(replaced.len() > ERROR_BODY_LIMIT || out == replaced);
    }
}

fn js_int() -> impl Strategy<Value = i64> {
    -(1_i64 << 53)..(1_i64 << 53)
}

fn hud_frame() -> impl Strategy<Value = SocketFrame> {
    let payload = prop_oneof![
        vec(any::<u8>(), 0..64).prop_map(UpdatePayload::Inline),
        "[a-z0-9/-]{1,40}".prop_map(UpdatePayload::Link),
    ];
    let update = (
        select(vec![UpdateEncoding::V1, UpdateEncoding::V2]),
        any::<u64>(),
        js_int(),
        any::<u64>(),
        vec(any::<u8>(), 0..16),
        payload,
    )
        .prop_map(|(encoding, server_time, left, id, vector, payload)| {
            ServerUpdate {
                encoding,
                server_time,
                left,
                id,
                state_vector: match encoding {
                    UpdateEncoding::V2 => vector,
                    UpdateEncoding::V1 => Vec::new(),
                },
                payload,
            }
        });
    let message = prop_oneof![
        update.prop_map(SocketMessage::Update),
        any::<u64>().prop_map(SocketMessage::Ack),
        select(vec![EditRight::ReadOnly, EditRight::Editable]).prop_map(|right| {
            SocketMessage::Response(DocumentResponse::Subscribed { right })
        }),
        "\\PC{0,20}".prop_map(|code| SocketMessage::Response(
            DocumentResponse::NotSubscribed {
                code: SocketErrorCode::from_code(code)
            }
        )),
    ];
    (uuid(), message).prop_map(|(document, message)| SocketFrame { document, message })
}

proptest! {
    #![proptest_config(config())]

    /// A varint reads back to its value and leaves the tail untouched.
    #[test]
    fn var_uint_round_trips(value in any::<u64>(), tail in vec(any::<u8>(), 0..8)) {
        let bytes = [lib0::var_uint(value), tail.clone()].concat();
        prop_assert_eq!(lib0::read_var_uint(&bytes).ok(), Some((value, &tail[..])));
    }

    /// LEB128 is minimal: 7 value bits per byte, at least one byte.
    #[test]
    fn var_uint_is_minimal(value in any::<u64>()) {
        let bits = value.bit_width();
        let expected = usize::try_from(bits.div_ceil(7).max(1)).unwrap_or(usize::MAX);
        prop_assert_eq!(lib0::var_uint(value).len(), expected);
    }

    /// A signed varint as lib0 writes it reads back, sign included.
    #[test]
    fn var_int_round_trips(value in js_int(), tail in vec(any::<u8>(), 0..8)) {
        let bytes = [var_int(value), tail.clone()].concat();
        prop_assert_eq!(lib0::read_var_int(&bytes).ok(), Some((value, &tail[..])));
    }

    /// Frames packed into one message come back as they were, in order.
    #[test]
    fn frames_round_trip(sent in vec(hud_frame(), 0..6)) {
        let message: Vec<u8> = sent.iter().flat_map(encoded).collect();
        prop_assert_eq!(hud::frames(&message).ok(), Some(sent));
    }

    /// A frame cut anywhere is an error, not a panic or a shorter frame.
    #[test]
    fn cut_frame_is_refused(frame in hud_frame(), cut in any::<Index>()) {
        let bytes = encoded(&frame);
        let short = bytes.get(..cut.index(bytes.len())).unwrap_or_default();
        prop_assert!(short.is_empty() || hud::frames(short).is_err());
    }

    /// Any bytes are read without a panic.
    #[test]
    fn frames_never_panic(bytes in vec(any::<u8>(), 0..256)) {
        drop(hud::frames(&bytes));
    }
}
