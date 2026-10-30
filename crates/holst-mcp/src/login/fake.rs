//! Deterministic stand-ins for the store, the clock and the network.
#![expect(
    clippy::disallowed_methods,
    reason = "the fake store is a synchronous `Backend`: its slowness blocks the thread"
)]

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use holst_core::{
    domain::session::Session,
    effect::{
        clock::Clock,
        http::{BoxFuture, HttpRequest, HttpResponse, Transport},
    },
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
};
use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
use secrecy::SecretString;
use tokio::sync::Notify;

use crate::login::store::Backend;

pub(crate) fn session(token: &str) -> Session {
    Session::new(SecretString::from(token))
}

pub(crate) fn session_with(token: &str, cookies: &str) -> Session {
    Session::with_cookies(
        SecretString::from(token),
        cookies.parse().unwrap_or_default(),
    )
}

/// A client holding `session` whose requests go to `network`.
pub(crate) fn client(
    session: Session,
    network: &Arc<Script>,
) -> holst_core::Result<Client> {
    Ok(holst_core::command::login::connect(
        Some(session),
        "http://127.0.0.1:1".parse()?,
        holst_core::system::clock::SystemClock,
        holst_core::system::id::RandomIds,
    )?
    .with_transport(Arc::clone(network)))
}

#[derive(Clone)]
pub(crate) enum CannedResponse {
    With {
        status: u16,
        body: &'static str,
        cookies: Vec<&'static str>,
    },
    Hang,
}

impl CannedResponse {
    pub(crate) fn ok(body: &'static str) -> Self {
        CannedResponse::With {
            status: 200,
            body,
            cookies: Vec::new(),
        }
    }

    pub(crate) fn ok_setting(body: &'static str, cookie: &'static str) -> Self {
        CannedResponse::With {
            status: 200,
            body,
            cookies: vec![cookie],
        }
    }

    pub(crate) fn unauthorized() -> Self {
        CannedResponse::With {
            status: 401,
            body: r#"{"errCode":"unauthorized"}"#,
            cookies: Vec::new(),
        }
    }

    fn response(status: u16, body: &str, cookies: &[&str]) -> HttpResponse {
        let headers = cookies
            .iter()
            .filter_map(|cookie| HeaderValue::from_str(cookie).ok())
            .fold(HeaderMap::new(), |mut headers, cookie| {
                headers.append(SET_COOKIE, cookie);
                headers
            });
        HttpResponse {
            status,
            headers,
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Script {
    replies: Mutex<VecDeque<CannedResponse>>,
    seen: Mutex<Vec<String>>,
}

impl Script {
    pub(crate) fn new(replies: impl IntoIterator<Item = CannedResponse>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().collect()),
            seen: Mutex::default(),
        })
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, holst_core::Result<HttpResponse>> {
        let query = request.url.query().map(|q| format!("?{q}"));
        let request_body = request
            .body
            .as_ref()
            .map(|body| format!(" {}", String::from_utf8_lossy(&body.bytes)));
        let (method, path) = (&request.method, request.url.path());
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!(
                "{method} {path}{}{}",
                query.unwrap_or_default(),
                request_body.unwrap_or_default()
            ));
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| CannedResponse::ok("{}"));
        match next {
            CannedResponse::With {
                status,
                body,
                cookies,
            } => {
                let response = CannedResponse::response(status, body, &cookies);
                Box::pin(async move { Ok(response) })
            }
            CannedResponse::Hang => Box::pin(std::future::pending()),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Manual(AtomicU64);

impl Manual {
    pub(crate) fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.0.fetch_add(by, Ordering::SeqCst);
    }

    pub(crate) fn rewind(&self, by: Duration) {
        let by = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.0
            .store(self.unix_millis().saturating_sub(by), Ordering::SeqCst);
    }
}

impl Clock for Manual {
    fn unix_millis(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub(crate) struct Memory {
    stored: Mutex<Option<Session>>,
    loads: AtomicUsize,
    saved: Mutex<Vec<Session>>,
    saved_to: Mutex<Vec<SessionStoreKind>>,
    kind: SessionStoreKind,
    slow: Duration,
    slow_save: Duration,
    pub(crate) saving: Notify,
    locked: Mutex<Lock>,
    writable: Mutex<Lock>,
    network: Arc<Script>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lock {
    Open,
    Locked,
}

impl Memory {
    pub(crate) fn new(stored: Option<Session>, network: Arc<Script>) -> Self {
        Self {
            stored: Mutex::new(stored),
            loads: AtomicUsize::new(0),
            saved: Mutex::default(),
            saved_to: Mutex::default(),
            kind: SessionStoreKind::Keychain,
            slow: Duration::ZERO,
            slow_save: Duration::ZERO,
            saving: Notify::new(),
            locked: Mutex::new(Lock::Open),
            writable: Mutex::new(Lock::Open),
            network,
        }
    }

    pub(crate) fn read_only(self) -> Self {
        Self {
            writable: Mutex::new(Lock::Locked),
            ..self
        }
    }

    pub(crate) fn make_writable(&self) {
        *self.writable.lock().unwrap_or_else(PoisonError::into_inner) = Lock::Open;
    }

    pub(crate) fn slow_save(self, pause: Duration) -> Self {
        Self {
            slow_save: pause,
            ..self
        }
    }

    pub(crate) fn locked(self) -> Self {
        Self {
            locked: Mutex::new(Lock::Locked),
            ..self
        }
    }

    pub(crate) fn slow(self, pause: Duration) -> Self {
        Self {
            slow: pause,
            ..self
        }
    }

    pub(crate) fn lock_now(&self) {
        *self.locked.lock().unwrap_or_else(PoisonError::into_inner) = Lock::Locked;
    }

    pub(crate) fn store(&self, session: Option<Session>) {
        *self.stored.lock().unwrap_or_else(PoisonError::into_inner) = session;
    }

    pub(crate) fn loads(&self) -> usize {
        self.loads.load(Ordering::SeqCst)
    }

    pub(crate) fn in_file(self) -> Self {
        Self {
            kind: SessionStoreKind::File,
            ..self
        }
    }

    pub(crate) fn saved_to(&self) -> Vec<SessionStoreKind> {
        self.saved_to
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn saved(&self) -> Vec<Session> {
        self.saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Backend for Memory {
    fn load(&self) -> holst_core::Result<Option<(Session, SessionStoreKind)>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.slow);
        let locked = *self.locked.lock().unwrap_or_else(PoisonError::into_inner);
        match locked {
            Lock::Locked => Err(holst_core::Error::LocalData(
                holst_core::error::local_data::LocalDataError::KeychainLocked,
            )),
            Lock::Open => Ok(self
                .stored
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .map(|session| (session, self.kind))),
        }
    }

    fn save(
        &self,
        session: &Session,
        from: SessionStoreKind,
    ) -> holst_core::Result<LoginSaveReport> {
        self.saved_to
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(from);
        self.saving.notify_one();
        std::thread::sleep(self.slow_save);
        let writable = *self.writable.lock().unwrap_or_else(PoisonError::into_inner);
        match writable {
            Lock::Locked => Err(holst_core::Error::LocalData(
                holst_core::error::local_data::LocalDataError::KeychainLocked,
            )),
            Lock::Open => {
                self.saved
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(session.clone());
                self.store(Some(session.clone()));
                Ok(LoginSaveReport {
                    store: from,
                    stale: None,
                })
            }
        }
    }

    fn connect(&self, session: Session) -> holst_core::Result<Client> {
        client(session, &self.network)
    }
}
