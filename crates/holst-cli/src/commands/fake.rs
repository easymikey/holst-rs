//! A server in memory for command tests: replies, statuses, cookies.

use std::sync::{Arc, Mutex, PoisonError};

use holst_core::effect::http::{BoxFuture, HttpRequest, HttpResponse, Transport};
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct CannedResponse {
    path: &'static str,
    status: u16,
    body: String,
}

#[derive(Debug, Default)]
pub(crate) struct FakeServer {
    replies: Mutex<Vec<CannedResponse>>,
    seen: Mutex<Vec<String>>,
    cookies: Mutex<Vec<String>>,
}

impl FakeServer {
    pub(crate) fn responding(replies: &[(&'static str, u16, Value)]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|&(path, status, ref body)| CannedResponse {
                        path,
                        status,
                        body: body.to_string(),
                    })
                    .collect(),
            ),
            seen: Mutex::default(),
            cookies: Mutex::default(),
        })
    }

    pub(crate) fn cookies(&self) -> Vec<String> {
        self.cookies
            .lock()
            .map(|cookies| cookies.clone())
            .unwrap_or_default()
    }

    fn take(&self, path: &str) -> HttpResponse {
        let reply = Some(self.replies.lock().unwrap_or_else(PoisonError::into_inner))
            .and_then(|mut replies| {
                let found = replies.iter().position(|reply| reply.path == path);
                found.map(|at| replies.remove(at))
            });
        let (status, body) =
            reply.map_or((404, String::new()), |reply| (reply.status, reply.body));
        HttpResponse {
            status,
            body: body.into_bytes(),
            ..HttpResponse::default()
        }
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl Transport for FakeServer {
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, holst_core::Result<HttpResponse>> {
        let path = request.url.path().to_owned();
        let cookie = request
            .headers
            .get("cookie")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        self.cookies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(cookie);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{} {path}", request.method.as_str()));
        let reply = self.take(&path);
        Box::pin(async move { Ok(reply) })
    }
}
