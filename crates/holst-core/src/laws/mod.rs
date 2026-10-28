//! Laws of the pure functions: properties for any input, with a fixed seed.

mod board;
mod constructors;
mod id;
mod protocol;
mod session;
mod text;

use std::collections::BTreeMap;

use proptest::{
    collection::{btree_map, vec},
    prelude::{Just, Strategy, any, prop_oneof},
    test_runner::{Config, RngSeed},
};
use serde_json::{Value, json};

use crate::{
    domain::{board::EditRight, cookie::CookieJar, id::BoardId},
    protocol::{
        hud,
        hud::{
            DocumentResponse,
            SocketFrame,
            SocketMessage,
            UpdateEncoding,
            UpdatePayload,
        },
        lib0,
    },
};

pub(crate) fn config() -> Config {
    Config {
        cases: 256,
        rng_seed: RngSeed::Fixed(20_261_022),
        failure_persistence: None,
        ..Config::default()
    }
}


fn uuid() -> impl Strategy<Value = String> {
    let lower = any::<u128>()
        .prop_map(|bits| uuid::Uuid::from_u128(bits).hyphenated().to_string());
    prop_oneof![lower.clone(), lower.prop_map(|id| id.to_ascii_uppercase())]
}

fn path_text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[./%?#a-zA-Z0-9 ~_-]{0,6}",
        Just(String::new()),
        Just(".".to_owned()),
        Just("..".to_owned()),
    ]
}

fn cookie_name() -> impl Strategy<Value = String> {
    "[A-Za-z0-9_!#$&'*+.^`|~-]{1,12}"
}

fn cookie_value() -> impl Strategy<Value = String> {
    "[A-Za-z0-9=/+._~:!#$&'()*<>?@^`{|}-]{0,16}"
}

fn jar_pairs() -> impl Strategy<Value = BTreeMap<String, String>> {
    btree_map(cookie_name(), cookie_value(), 0..6)
}

fn jar() -> impl Strategy<Value = CookieJar> {
    jar_pairs().prop_map(|pairs| pairs.into_iter().collect())
}

fn parsed_jar(stored: &str) -> CookieJar {
    let Ok(jar) = stored.parse::<CookieJar>();
    jar
}

fn token() -> impl Strategy<Value = String> {
    prop_oneof!["[A-Za-z0-9_.-]{1,48}", "\\S(.{0,30}\\S)?"]
}

const LAST_MS: u64 = 253_402_300_799_999;

fn board_name() -> impl Strategy<Value = String> {
    "[a-zA-Zа-яА-ЯёЁ0-9 :.,-]{0,12}"
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i32>().prop_map(Value::from),
        ".{0,6}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::from),
            btree_map("[a-z_]{1,6}", inner, 0..4)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}

fn found(input: &str) -> Option<String> {
    BoardId::find_in(input).map(String::from)
}

fn var_int(value: i64) -> Vec<u8> {
    let magnitude = value.unsigned_abs();
    let low = u8::try_from(magnitude & 0x3f).unwrap_or_default();
    let high = magnitude >> 6;
    let first = low | (u8::from(value < 0) << 6) | (u8::from(high > 0) << 7);
    let tail = match high {
        0 => Vec::new(),
        high => lib0::var_uint(high),
    };
    [vec![first], tail].concat()
}

pub(crate) fn encoded(frame: &SocketFrame) -> Vec<u8> {
    let name = |encoding| match encoding {
        UpdateEncoding::V2 => [
            lib0::var_string(hud::ENC_V2),
            lib0::var_string(&frame.document),
        ]
        .concat(),
        UpdateEncoding::V1 => lib0::var_string(&frame.document),
    };
    match &frame.message {
        SocketMessage::Update(update) => [
            name(update.encoding),
            lib0::var_uint(hud::UPDATE),
            lib0::var_uint(update.server_time),
            var_int(update.left),
            lib0::var_uint(update.id),
            match update.encoding {
                UpdateEncoding::V2 => lib0::var_bytes(&update.state_vector),
                UpdateEncoding::V1 => Vec::new(),
            },
            match &update.payload {
                UpdatePayload::Inline(bytes) => {
                    [lib0::var_uint(hud::INLINE), lib0::var_bytes(bytes)].concat()
                }
                UpdatePayload::Link(link) => {
                    [lib0::var_uint(hud::LINK), lib0::var_string(link)].concat()
                }
            },
        ]
        .concat(),
        SocketMessage::Ack(id) => [
            name(UpdateEncoding::V2),
            lib0::var_uint(hud::ACK),
            lib0::var_uint(*id),
        ]
        .concat(),
        SocketMessage::Response(answer) => {
            let reply = match answer {
                DocumentResponse::Subscribed { right: access } => {
                    json!({ "status": "ok", "readOnly": *access == EditRight::ReadOnly })
                }
                DocumentResponse::NotSubscribed { code: refusal } => {
                    json!({ "error": refusal.to_string() })
                }
            };
            [
                name(UpdateEncoding::V2),
                lib0::var_uint(hud::RESPONSE),
                lib0::var_string(&reply.to_string()),
            ]
            .concat()
        }
        SocketMessage::Unknown { kind, dropped } => [
            name(UpdateEncoding::V1),
            lib0::var_uint(*kind),
            vec![0; *dropped],
        ]
        .concat(),
    }
}
