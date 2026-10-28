use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
};

use futures_util::future;
use itertools::Itertools as _;
use serde_json::json;
use yrs::{
    Any,
    Doc,
    Map as _,
    ReadTxn as _,
    StateVector,
    Transact as _,
    updates::encoder::Encode as _,
};

use crate::{
    domain::{
        board::{EditRight, LoadedBoard},
        change::value::BoardChange,
    },
    effect::{
        http::BoxFuture,
        socket::{BoardSocket, SocketConnector, SocketSession},
    },
    error::{Error, Result, board_socket::BoardSocketError},
    laws::encoded,
    protocol::{
        hud,
        hud::{
            DocumentResponse,
            ServerUpdate as HudUpdate,
            SocketFrame,
            SocketMessage as HudMessage,
            UpdateEncoding,
            UpdatePayload,
        },
    },
    remote::client::{Client, fake},
};

pub(crate) const BOARD: &str = "00000000-0000-4000-8000-0000000000a6";

#[derive(Debug, Default)]
pub(crate) struct Script {
    pub(crate) incoming: Mutex<VecDeque<Option<Vec<u8>>>>,
    pub(crate) sent: Mutex<Vec<Vec<u8>>>,
    pub(crate) acks: bool,
    pub(crate) resets: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Scripted(pub(crate) Arc<Script>);

impl BoardSocket for Scripted {
    fn send(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        self.0
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(bytes.clone());
        let ack = hud::read_update_frame(&bytes)
            .ok()
            .filter(|_| self.0.acks)
            .map(|(_, id, _)| encoded(&on_board(HudMessage::Ack(id))));
        self.0
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(ack.map(Some));
        Box::pin(future::ready(Ok(())))
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        let next = self
            .0
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let reset = || {
            Error::BoardSocket(BoardSocketError::Connection(Box::new(
                tungstenite::Error::ConnectionClosed,
            )))
        };
        match (next, self.0.resets) {
            (Some(message), _) => Box::pin(future::ready(Ok(message))),
            (None, true) => Box::pin(future::ready(Err(reset()))),
            (None, false) => Box::pin(future::pending()),
        }
    }
}

impl SocketConnector for Scripted {
    fn open<'a>(
        &'a self,
        _: &'a SocketSession,
    ) -> BoxFuture<'a, Result<Box<dyn BoardSocket>>> {
        let socket: Box<dyn BoardSocket> = Box::new(self.clone());
        Box::pin(future::ready(Ok(socket)))
    }
}

pub(crate) fn script(
    messages: impl IntoIterator<Item = Option<Vec<SocketFrame>>>,
) -> Scripted {
    scripted(messages, Acknowledgement::Without)
}

pub(crate) fn acking(
    messages: impl IntoIterator<Item = Option<Vec<SocketFrame>>>,
) -> Scripted {
    scripted(messages, Acknowledgement::With)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Acknowledgement {
    Without,
    With,
}

fn scripted(
    messages: impl IntoIterator<Item = Option<Vec<SocketFrame>>>,
    acks: Acknowledgement,
) -> Scripted {
    let incoming = messages
        .into_iter()
        .map(|frames| frames.map(|frames| frames.iter().flat_map(encoded).collect()))
        .collect();
    Scripted(Arc::new(Script {
        incoming: Mutex::new(incoming),
        sent: Mutex::default(),
        acks: acks == Acknowledgement::With,
        resets: false,
    }))
}

pub(crate) fn sticker_by(client: u64, id: &str) -> (Vec<u8>, StateVector) {
    let doc = Doc::with_client_id(client);
    let fields = json!({"id": id, "type": "sticker", "position": {"x": 1, "y": 2}});
    let sticker = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
    doc.get_or_insert_map("objects")
        .insert(&mut doc.transact_mut(), id, sticker);
    let txn = doc.transact();
    (
        txn.encode_state_as_update_v2(&StateVector::default()),
        txn.state_vector(),
    )
}

pub(crate) fn ids_of(loaded: &LoadedBoard) -> Vec<String> {
    crate::domain::board::objects(&loaded.document)
        .into_iter()
        .map(|object| object.id)
        .sorted()
        .collect()
}

pub(crate) fn on_board(message: HudMessage) -> SocketFrame {
    SocketFrame {
        document: BOARD.to_owned(),
        message,
    }
}

pub(crate) fn update(left: i64, payload: UpdatePayload) -> SocketFrame {
    on_board(HudMessage::Update(HudUpdate {
        encoding: UpdateEncoding::V2,
        server_time: 1,
        left,
        id: 7,
        state_vector: Vec::new(),
        payload,
    }))
}

pub(crate) fn answer(answer: DocumentResponse) -> SocketFrame {
    on_board(HudMessage::Response(answer))
}

pub(crate) fn sent(socket: &Scripted) -> Vec<Vec<u8>> {
    socket
        .0
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(crate) fn empty_board(right: EditRight) -> Vec<SocketFrame> {
    vec![
        answer(DocumentResponse::Subscribed { right }),
        update(0, UpdatePayload::Inline(Vec::new())),
    ]
}

pub(crate) fn signed_in(
    more: impl IntoIterator<Item = crate::effect::http::HttpResponse>,
) -> Result<(Arc<fake::Fake>, Client)> {
    on_board_reply(r#"{"id":"b","in_trash":null}"#, more)
}

pub(crate) fn on_board_reply(
    channel: &str,
    more: impl IntoIterator<Item = crate::effect::http::HttpResponse>,
) -> Result<(Arc<fake::Fake>, Client)> {
    let me = fake::reply(200, &[], r#"{"id":"me"}"#);
    let channel = fake::reply(200, &[], channel);
    fake::on_fake(
        [me, channel].into_iter().chain(more),
        Some(fake::stored("tok", "s=1")),
    )
}

pub(crate) fn note(text: &str) -> BoardChange {
    BoardChange::AddAll(vec![crate::domain::change::value::NewEntry {
        new: crate::domain::change::value::NewObject::Sticker {
            text: text.into(),
            at: crate::domain::geometry::Point::whole(1, 2),
            size: None,
            color: None,
        },
        frame: None,
    }])
}

pub(crate) fn link_update(server: &StateVector) -> SocketFrame {
    on_board(HudMessage::Update(HudUpdate {
        encoding: UpdateEncoding::V2,
        server_time: 1,
        left: 0,
        id: 9,
        state_vector: server.encode_v1(),
        payload: UpdatePayload::Link(
            "https://cdns-edge.app.holst.so/snapshots/9".into(),
        ),
    }))
}

pub(crate) fn linked_board(server: &StateVector) -> Scripted {
    script([Some(vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        link_update(server),
    ])])
}
