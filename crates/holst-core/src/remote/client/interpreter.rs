//! Planned requests carried out: sent, or only reported by a dry run.

use futures_util::{FutureExt as _, future};

use crate::{
    effect::{
        http::BoxFuture,
        interpreter::{CarriedRequest, RequestInterpreter},
        request::ApiRequest,
        server::HolstServer,
        socket::SocketSession,
    },
    error::Result,
    remote::client::Client,
};

impl HolstServer for Client {
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<serde_json::Value>> {
        Box::pin(Client::send_json(self, request))
    }

    fn download_link<'a>(&'a self, link: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(Client::download_trusted_link(self, link))
    }

    fn socket_session(&self) -> Result<SocketSession> {
        Client::session_for_socket(self)
    }
}

/// Sends the request and returns the reply.
#[derive(Debug, Clone, Copy)]
pub struct RequestSender;

/// Reports the request and sends nothing: a dry run.
#[derive(Debug, Clone, Copy)]
pub struct RequestReporter;

impl RequestInterpreter for RequestSender {
    fn carry<'a>(
        &'a self,
        server: &'a dyn HolstServer,
        request: ApiRequest,
    ) -> BoxFuture<'a, Result<CarriedRequest>> {
        Box::pin(
            server
                .send(request)
                .map(|reply| reply.map(CarriedRequest::Sent)),
        )
    }
}

impl RequestInterpreter for RequestReporter {
    fn carry<'a>(
        &'a self,
        _server: &'a dyn HolstServer,
        request: ApiRequest,
    ) -> BoxFuture<'a, Result<CarriedRequest>> {
        Box::pin(future::ready(Ok(CarriedRequest::Reported(request))))
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;

    use crate::{
        effect::interpreter::{CarriedRequest, RequestInterpreter},
        error::Result,
        remote::client::{
            fake::{on_fake, reply, stored},
            interpreter::{RequestReporter, RequestSender},
        },
    };

    #[tokio::test]
    async fn the_reporter_sends_nothing_and_the_sender_sends_once() -> Result<()> {
        let (fake, client) =
            on_fake([reply(200, &[], "{\"ok\":true}")], Some(stored("t", "")))?;
        let request = crate::remote::api::channel::rename_request("b1", "Plans")?;
        let reported = RequestReporter.carry(&client, request.clone()).await?;
        assert_eq!(reported, CarriedRequest::Reported(request.clone()));
        assert!(fake.requests().is_empty());
        let sent = RequestSender.carry(&client, request).await?;
        assert_eq!(sent, CarriedRequest::Sent(json!({ "ok": true })));
        assert_eq!(fake.requests().len(), 1);
        Ok(())
    }
}
