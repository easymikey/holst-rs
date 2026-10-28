//! A scripted fake transport and client helpers for tests.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use secrecy::{ExposeSecret as _, SecretString};

use crate::{
    domain::{
        id_source::SequentialIds,
        session::Session,
        url::{BaseUrl, DEFAULT_BASE},
    },
    effect::{
        clock::FixedClock,
        http::{BoxFuture, HttpRequest, HttpResponse, Transport},
    },
    error::{Error, Result, http::HttpError},
    remote::client::Client,
};

#[derive(Debug, Default)]
pub(crate) struct Fake {
    replies: Mutex<VecDeque<Result<HttpResponse>>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Fake {
    fn replying(replies: impl IntoIterator<Item = HttpResponse>) -> Self {
        Self::scripted(replies.into_iter().map(Ok))
    }

    pub(crate) fn scripted(
        replies: impl IntoIterator<Item = Result<HttpResponse>>,
    ) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            seen: Mutex::default(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<HttpRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Fake {
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, Result<HttpResponse>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Ok(HttpResponse {
                    status: 599,
                    ..HttpResponse::default()
                })
            });
        Box::pin(std::future::ready(reply))
    }

    fn limited(
        self: Arc<Self>,
        _timeout: Duration,
        limit: usize,
    ) -> Result<Arc<dyn Transport>> {
        Ok(Arc::new(Limited { inner: self, limit }))
    }
}

/// A [`Fake`] that refuses a body over `limit`, as the network transport does.
struct Limited {
    inner: Arc<Fake>,
    limit: usize,
}

impl Transport for Limited {
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, Result<HttpResponse>> {
        Box::pin(async move {
            let response = self.inner.send(request).await?;
            (response.body.len() <= self.limit)
                .then_some(response)
                .ok_or(Error::Http(HttpError::ResponseTooLarge {
                    limit: self.limit,
                }))
        })
    }
}

pub(crate) fn new_client(session: Option<Session>, base: BaseUrl) -> Result<Client> {
    Client::new(
        session,
        base,
        FixedClock { unix_millis: 0 },
        SequentialIds::new("id"),
    )
}

pub(crate) fn connect(
    fake: Fake,
    session: Option<Session>,
) -> Result<(Arc<Fake>, Client)> {
    let fake = Arc::new(fake);
    let client =
        new_client(session, DEFAULT_BASE.parse()?)?.with_transport(Arc::clone(&fake));
    Ok((fake, client))
}

pub(crate) fn on_fake(
    replies: impl IntoIterator<Item = HttpResponse>,
    session: Option<Session>,
) -> Result<(Arc<Fake>, Client)> {
    connect(Fake::replying(replies), session)
}

pub(crate) fn stored(token: &str, cookies: &str) -> Session {
    Session::with_cookies(
        SecretString::from(token),
        cookies.parse().unwrap_or_default(),
    )
}

pub(crate) fn cookie_line(session: &Session) -> String {
    session.cookies().header().expose_secret().to_owned()
}

pub(crate) fn reply(
    status: u16,
    headers: &[(&'static str, &str)],
    body: &str,
) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers
            .iter()
            .filter_map(|&(name, value)| {
                reqwest::header::HeaderValue::from_str(value)
                    .ok()
                    .map(|value| {
                        (reqwest::header::HeaderName::from_static(name), value)
                    })
            })
            .collect(),
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn lost(path: &str) -> Result<HttpResponse> {
    Err(Error::Http(HttpError::NoResponse {
        method: "POST",
        path: path.to_owned(),
    }))
}
