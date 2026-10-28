#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::{Message, http::header};

use super::TungsteniteConnector;
use crate::{
    effect::socket::{SocketConnector, SocketSession},
    error::{Error, Result, board_socket::BoardSocketError},
};

#[tokio::test]
async fn real_socket_sends_login_headers_and_skips_control_frames() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base: crate::domain::url::BaseUrl =
        format!("http://{}", listener.local_addr()?).parse()?;
    let (cookie_tx, cookie_rx) = std::sync::mpsc::channel();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await?;
        // imperative: sending and reading take the socket by `&mut`
        let mut ws = tokio_tungstenite::accept_hdr_async(tcp, KeepCookie(cookie_tx))
            .await
            .map_err(Box::new)
            .map_err(BoardSocketError::from)?;
        let first = ws
            .next()
            .await
            .transpose()
            .map_err(Box::new)
            .map_err(BoardSocketError::from)?;
        let replies = [
            Message::Ping(vec![1].into()),
            Message::text("hi"),
            Message::binary(vec![7, 8]),
        ];
        ws.send_all(&mut futures_util::stream::iter(replies.map(Ok)))
            .await
            .map_err(Box::new)
            .map_err(BoardSocketError::from)?;
        ws.close(None)
            .await
            .map_err(Box::new)
            .map_err(BoardSocketError::from)?;
        Ok::<_, Error>(first)
    });
    let login = SocketSession {
        url: base.board_socket()?,
        token: "tok".into(),
        cookies: "s=1".into(),
        user_agent: "test".into(),
    };
    // imperative: sending and receiving take the socket by `&mut`
    let mut socket = TungsteniteConnector.open(&login).await?;
    socket.send(vec![1, 2, 3]).await?;
    assert_eq!(socket.receive().await?, Some(vec![7, 8]));
    assert_eq!(socket.receive().await?, None);
    let first = server.await.map_err(std::io::Error::other)??;
    assert_eq!(first, Some(Message::binary(vec![1, 2, 3])));
    assert_eq!(cookie_rx.try_recv().ok().flatten().as_deref(), Some("s=1"));
    Ok(())
}

struct KeepCookie(std::sync::mpsc::Sender<Option<String>>);

impl tungstenite::handshake::server::Callback for KeepCookie {
    fn on_request(
        self,
        request: &tungstenite::handshake::server::Request,
        response: tungstenite::handshake::server::Response,
    ) -> std::result::Result<
        tungstenite::handshake::server::Response,
        tungstenite::handshake::server::ErrorResponse,
    > {
        let cookie = request.headers().get(header::COOKIE);
        drop(
            self.0
                .send(cookie.and_then(|v| v.to_str().ok()).map(str::to_owned)),
        );
        Ok(response)
    }
}
