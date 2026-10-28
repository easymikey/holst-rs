//! The comment threads of a board: its room `<board>$comments` on the socket.

use crate::{
    domain::{
        board::{
            EditRight,
            comment::{CommentChange, Thread, build, threads},
        },
        change::build::update_id,
    },
    error::Result,
    protocol::document::COMMENTS_ROOM,
    remote::{
        client::Client,
        socket::{
            stream::TungsteniteConnector,
            write::{ConnectedBoard, open_with, sent},
        },
    },
};

/// The name of the room that holds the threads of `board`.
#[must_use]
pub fn room(board: &str) -> String {
    format!("{board}{COMMENTS_ROOM}")
}

/// The comments of a board as loaded, over a socket open for one change.
#[derive(Debug)]
pub struct CommentRoom {
    connected: ConnectedBoard,
}

/// Load the threads of `board` over the real socket and keep it open.
///
/// # Errors
/// As [`open_with`]: the whole room is read, every chunk of the initial state
/// applied.
pub async fn open(client: &Client, board: &str) -> Result<CommentRoom> {
    open_with(client, &TungsteniteConnector, &room(board))
        .await
        .map(|connected| CommentRoom { connected })
}

impl CommentRoom {
    /// What the user may do in the room.
    #[must_use]
    pub const fn right(&self) -> EditRight {
        self.connected.right()
    }

    /// The threads of the board, oldest first.
    #[must_use]
    pub fn threads(&self) -> Vec<Thread> {
        threads(self.connected.document())
    }

    /// Writes `change` and waits for the server's ack.
    ///
    /// # Errors
    /// A thread the room does not hold, and as
    /// [`ConnectedBoard::send`] and
    /// [`SentUpdate::accepted`](crate::remote::socket::write::SentUpdate::accepted).
    pub async fn send(self, client: &Client, change: &CommentChange) -> Result<()> {
        let update = build(self.connected.document(), change, client.ids())?;
        sent(self.connected, client, update, update_id(&client.new_id()))
            .await
            .map(|_| ())
    }
}
