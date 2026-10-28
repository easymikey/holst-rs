//! The board socket: binary messages over `tokio-tungstenite`.

#[cfg(test)] mod tests;

use futures_util::{SinkExt as _, StreamExt as _, future};
use secrecy::ExposeSecret;
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest as _,
    http::{HeaderValue, header},
};

use crate::{
    effect::{
        http::BoxFuture,
        socket::{BoardSocket, SocketConnector, SocketSession},
    },
    error::{Error, Result},
    remote::socket::{proxy, proxy::ProxyRoute},
};

/// The real socket over `tokio-tungstenite` with rustls.
#[derive(Debug, Clone, Copy, Default)]
pub struct TungsteniteConnector;

struct TungsteniteSocket(
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
);

impl SocketConnector for TungsteniteConnector {
    fn open<'a>(
        &'a self,
        login: &'a SocketSession,
    ) -> BoxFuture<'a, Result<Box<dyn BoardSocket>>> {
        Box::pin(async move {
            // imperative: the headers are added through `&mut`
            let mut request = login
                .url
                .as_str()
                .into_client_request()
                .map_err(Box::new)
                .map_err(crate::error::board_socket::BoardSocketError::from)?;
            let headers = request.headers_mut();
            [
                (header::COOKIE, login.cookies.expose_secret()),
                (header::USER_AGENT, login.user_agent.as_str()),
            ]
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .filter_map(|(name, value)| {
                HeaderValue::from_str(value).ok().map(|mut value| {
                    value.set_sensitive(true);
                    (name, value)
                })
            })
            .for_each(|(name, value)| {
                headers.insert(name, value);
            });
            // The proxy of REST requests too: else no board loads behind one.
            let matcher = hyper_util::client::proxy::matcher::Matcher::from_system();
            let (stream, _) = match proxy::route(&matcher, &login.url)? {
                ProxyRoute::Direct => tokio_tungstenite::connect_async(request).await,
                ProxyRoute::Tunnel {
                    proxy,
                    target,
                    authorization,
                } => {
                    let tunnel = proxy::tunnel(
                        &proxy,
                        &target,
                        authorization.as_ref().map(ExposeSecret::expose_secret),
                    )
                    .await?;
                    tokio_tungstenite::client_async_tls(request, tunnel).await
                }
            }
            .map_err(Box::new)
            .map_err(crate::error::board_socket::BoardSocketError::from)?;
            let socket: Box<dyn BoardSocket> = Box::new(TungsteniteSocket(stream));
            Ok(socket)
        })
    }
}

impl BoardSocket for TungsteniteSocket {
    fn send(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            Ok(self
                .0
                .send(Message::binary(bytes))
                .await
                .map_err(Box::new)
                .map_err(crate::error::board_socket::BoardSocketError::from)?)
        })
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(async move {
            // Ping, pong and text are skipped up to a binary message or the close.
            (&mut self.0)
                .filter_map(|message| future::ready(binary_of(message)))
                .next()
                .await
                .transpose()
                .map(Option::flatten)
        })
    }
}

fn binary_of(message: tungstenite::Result<Message>) -> Option<Result<Option<Vec<u8>>>> {
    match message {
        Ok(Message::Binary(bytes)) => Some(Ok(Some(bytes.to_vec()))),
        Ok(Message::Close(_)) => Some(Ok(None)),
        Ok(_) => None,
        Err(error) => Some(Err(Error::from(
            crate::error::board_socket::BoardSocketError::from(Box::new(error)),
        ))),
    }
}
