//! A board, a client and a directory for operation tests.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use serde::Serialize;
use serde_json::{Value, json};
use yrs::{Any, Doc, Map as _, MapPrelim, ReadTxn as _, StateVector, Transact as _};

use crate::{
    Result,
    command::table::{CommandArgs, parse},
    domain::board::{EditRight, Object, object_of},
    effect::http::HttpResponse,
    protocol::hud::{DocumentResponse, SocketFrame, UpdatePayload},
    remote::{
        client::{
            Client,
            fake::{Fake, on_fake, reply, stored},
        },
        socket::fake::{BOARD, Scripted, acking, answer, update},
    },
};

/// A board nothing else in the tests names.
pub(in crate::command) const OTHER: &str = "00000000-0000-4000-8000-0000000000a7";

pub(in crate::command) const FRAME: &str = "f1";
pub(in crate::command) const STICKER: &str = "s1";
pub(in crate::command) const TEXT: &str = "t1";
pub(in crate::command) const ARROW: &str = "a1";
pub(in crate::command) const IMAGE: &str = "i1";

/// A frame with a sticker in it, a text beside it, an arrow from one to the
/// other and an image below.
pub(in crate::command) fn objects() -> Vec<Value> {
    let at = |x: i32, y: i32| json!({"x": x, "y": y});
    vec![
        json!({"id": FRAME, "type": "frame", "position": at(0, 0), "width": 800,
               "height": 600, "labelText": "Q3", "zIndex": -1000}),
        json!({"id": STICKER, "type": "sticker", "position": at(24, 24), "width": 192,
               "height": 192, "parentId": FRAME, "zIndex": 10}),
        json!({"id": TEXT, "type": "simple-text", "position": at(900, 0), "zIndex": 20}),
        json!({"id": ARROW, "type": "arrow", "position": at(0, 0), "zIndex": 30,
               "start": {"objectId": STICKER, "point": at(216, 120)},
               "end": {"objectId": TEXT, "point": at(900, 0)}}),
        json!({"id": IMAGE, "type": "image", "position": at(0, 700), "width": 100,
               "height": 100, "name": "u1.png", "originalName": "cat.png",
               "zIndex": 40}),
    ]
}

/// The objects of the board as the core reads them.
pub(in crate::command) fn listed() -> Vec<Object> {
    objects().into_iter().filter_map(object_of).collect()
}

/// The arguments `given` names, on the test board unless they name one or the
/// command takes none.
pub(in crate::command) fn arguments<A: CommandArgs>(given: Value) -> Result<A> {
    let mut map = match given {
        Value::Object(map) => map,
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_) => serde_json::Map::new(),
    };
    if A::COMMAND
        .arguments
        .iter()
        .any(|argument| argument.name == "board")
    {
        map.entry("board").or_insert_with(|| json!(BOARD));
    }
    parse(map)
}

/// A signed-in client over a fake server that answers with `replies`.
pub(in crate::command) fn client(
    replies: impl IntoIterator<Item = HttpResponse>,
) -> Result<(Arc<Fake>, Client)> {
    on_fake(replies, Some(stored("tok", "s=1")))
}

/// The `channel/{id}` reply every online board load asks for, of a board in
/// format v2 on hud.
pub(in crate::command) fn channel() -> HttpResponse {
    channel_with(&json!({"schema_version": 2, "server_provider_type": "hud"}))
}

/// The `channel/{id}` reply with `format`'s fields over a bare node.
pub(in crate::command) fn channel_with(format: &Value) -> HttpResponse {
    let mut node = json!({"id": BOARD, "type": "board"});
    if let (Some(node), Some(format)) = (node.as_object_mut(), format.as_object()) {
        node.extend(format.clone());
    }
    reply(
        200,
        &[("content-type", "application/json")],
        &node.to_string(),
    )
}

/// `replies` after the channel reply that a board load asks for first.
pub(in crate::command) fn after_channel(
    replies: impl IntoIterator<Item = HttpResponse>,
) -> Vec<HttpResponse> {
    std::iter::once(channel()).chain(replies).collect()
}

/// As `signed_in`, with the format check of a load first: `channel()`, then
/// the account and the board that a write asks for.
pub(in crate::command) fn signed_in_hud() -> Result<(Arc<Fake>, Client)> {
    client([
        channel(),
        reply(200, &[], r#"{"id":"me"}"#),
        reply(200, &[], r#"{"id":"b","in_trash":null}"#),
    ])
}

/// A new empty directory under the system's temporary one.
pub(in crate::command) fn directory() -> Result<PathBuf> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "holst-object-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    crate::storage::file::create_dir_all(&dir)?;
    Ok(dir)
}

/// An editable board holding `objects`, for one connection; every update sent
/// is acknowledged.
pub(in crate::command) fn board_of(objects: &[Value]) -> Scripted {
    acking([Some(frames_of(objects))])
}

fn frames_of(objects: &[Value]) -> Vec<SocketFrame> {
    let doc = Doc::with_client_id(4);
    let store = doc.get_or_insert_map("objects");
    {
        let mut transaction = doc.transact_mut();
        // imperative: yrs inserts through one `&mut` transaction
        for fields in objects {
            let id = fields.get("id").and_then(Value::as_str).unwrap_or_default();
            let pairs: Vec<(String, Any)> = fields
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        Any::from_json(&value.to_string()).unwrap_or(Any::Null),
                    )
                })
                .collect();
            store.insert(&mut transaction, id, MapPrelim::from_iter(pairs));
        }
    }
    let state = doc
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        update(0, UpdatePayload::Inline(state)),
    ]
}

/// A 200 reply carrying `body` as JSON.
pub(in crate::command) fn json_reply(body: &Value) -> HttpResponse {
    reply(
        200,
        &[("content-type", "application/json")],
        &body.to_string(),
    )
}

/// The id with `ordinal` among the ids a test names.
pub(in crate::command) fn uuid(ordinal: u32) -> String {
    format!("00000001-0000-4000-8000-{ordinal:012x}")
}

/// The `status` field of a serialized outcome; empty when it has none.
pub(in crate::command) fn status_of(outcome: &impl Serialize) -> String {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|outcome| outcome.get("status").cloned())
        .and_then(|status| status.as_str().map(str::to_owned))
        .unwrap_or_default()
}
