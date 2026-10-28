//! The HTTP client: addresses, login, retries, redirects, reply status.

pub(crate) mod exchange;
#[cfg(test)] pub(crate) mod fake;
pub mod interpreter;
pub mod sign;
#[cfg(test)] mod tests;

use std::{
    fmt,
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard},
    time::Duration,
};

use futures_util::FutureExt as _;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::Instrument as _;

#[cfg(test)] pub(crate) use self::exchange::retry_wait;
use self::exchange::{ExchangeRequest, json_or_text};
use crate::{
    domain::{
        id_source::IdSource,
        node::CreationStamp,
        session::{Session, SessionState, SessionStatus},
        url::BaseUrl,
    },
    effect::{
        clock::Clock,
        http::{BoxFuture, HttpMethod, HttpResponse, Transport},
        request::{ApiRequest, Authorization},
    },
    error::Result,
    remote::http::ReqwestTransport,
};

pub(crate) const MAX_ATTEMPTS: u32 = 3;
pub(crate) const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
pub(crate) const ERROR_BODY_LIMIT: usize = 2048;
const MAX_REDIRECTS: u32 = 5;
const MIN_SCRUBBED_COOKIE: usize = 8;

/// The context of requests: base address, transport, shared login state, clock
/// and id source.
///
/// ```
/// use holst_core::{
///     domain::{id_source::SequentialIds, session::SessionStatus},
///     effect::clock::FixedClock,
///     remote::client::Client,
/// };
///
/// let (clock, ids) = (FixedClock { unix_millis: 0 }, SequentialIds::new("id"));
/// let client = Client::new(None, "https://app.holst.so".parse()?, clock, ids)?;
/// assert_eq!(client.status(), SessionStatus::Missing);
/// assert_eq!(client.board_link("b1"), "https://app.holst.so/board/b1");
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Clone)]
pub struct Client {
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdSource>,
    base: BaseUrl,
    state: Arc<RwLock<SessionState>>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base.as_str())
            .field("status", &self.status())
            .field("cookies", &self.cookie_count())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for a login (or none) on `base`, over the real network, telling
    /// the time by `clock` and numbering new objects by `ids`.
    ///
    /// # Errors
    /// [`crate::error::http::HttpError::Transport`] if the HTTP client cannot
    /// be built.
    pub fn new(
        session: Option<Session>,
        base: BaseUrl,
        clock: impl Clock + 'static,
        ids: impl IdSource + 'static,
    ) -> Result<Self> {
        Ok(Self {
            transport: Arc::new(ReqwestTransport::new(&sign::user_agent())?),
            clock: Arc::new(clock),
            ids: Arc::new(ids),
            base,
            state: Arc::new(RwLock::new(SessionState::from(session))),
        })
    }

    /// The same client on another transport.
    #[must_use]
    pub fn with_transport(self, transport: impl Transport + 'static) -> Self {
        Self {
            transport: Arc::new(transport),
            ..self
        }
    }

    /// Returns the same client over the same transport, reading at most
    /// `limit` bytes of a reply within `timeout`: a larger reply stops early.
    ///
    /// # Errors
    /// [`crate::error::http::HttpError::Transport`] if the HTTP client cannot
    /// be built.
    pub fn with_response_limit(self, timeout: Duration, limit: usize) -> Result<Self> {
        let transport = Arc::clone(&self.transport).limited(timeout, limit)?;
        Ok(Self { transport, ..self })
    }

    /// The same client with another clock.
    #[cfg(test)]
    #[must_use]
    pub fn with_clock(self, clock: impl Clock + 'static) -> Self {
        Self {
            clock: Arc::new(clock),
            ..self
        }
    }

    /// The same client with another id source.
    #[cfg(test)]
    #[must_use]
    pub fn with_ids(self, ids: impl IdSource + 'static) -> Self {
        Self {
            ids: Arc::new(ids),
            ..self
        }
    }

    /// A stamp for a new object: a fresh id and the current time.
    #[must_use]
    pub fn stamp(&self) -> CreationStamp {
        CreationStamp::new(&*self.clock, &*self.ids)
    }

    /// A fresh id.
    #[must_use]
    pub fn new_id(&self) -> String {
        self.ids.new_id()
    }

    /// The source of fresh ids.
    #[must_use]
    pub fn ids(&self) -> &dyn IdSource {
        &*self.ids
    }

    /// The clock the client tells the time by.
    #[must_use]
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock)
    }

    /// The client clock in Unix milliseconds.
    #[must_use]
    pub fn unix_millis(&self) -> u64 {
        self.clock.unix_millis()
    }

    /// The current time in the web client's format.
    #[must_use]
    pub fn now_iso8601(&self) -> String {
        crate::domain::time::Timestamp::from_millis(self.clock.unix_millis()).iso8601()
    }

    /// The client after a login: the token joins the cookies, on every clone.
    #[must_use]
    pub fn with_token(self, token: SecretString) -> Self {
        self.transition(|state| state.log_in(token));
        self
    }

    /// Drop the login and its cookies on every clone.
    pub fn forget(&self) {
        self.transition(SessionState::forget);
    }

    /// The active login; `None` before a login and after a revocation.
    #[must_use]
    pub fn session(&self) -> Option<Session> {
        self.state().session().cloned()
    }

    /// The account of the active login, when it was saved with it.
    #[must_use]
    pub fn account_id(&self) -> Option<String> {
        self.state()
            .session()
            .and_then(Session::account_id)
            .map(str::to_owned)
    }

    /// The active login's account became known; a new one is saved with it.
    pub fn learn_account_id(&self, user: &str) {
        self.transition(|state| state.learn_account_id(user));
    }

    /// Where the login stands.
    #[must_use]
    pub fn status(&self) -> SessionStatus {
        self.state().status()
    }

    /// The session to store if its cookies changed; marked until confirmed.
    #[must_use]
    pub fn rotated(&self) -> Option<Session> {
        self.state().rotated().cloned()
    }

    /// `stored` was saved (or skipped on purpose): clear the mark if current.
    pub fn confirm_saved(&self, stored: &Session) {
        self.transition(|state| state.saved(stored));
    }

    /// How many cookies the client holds.
    #[must_use]
    pub fn cookie_count(&self) -> usize {
        self.state().cookies().len()
    }

    /// The checked base address.
    #[must_use]
    pub fn base_url(&self) -> &BaseUrl {
        &self.base
    }

    /// A link to a board in the web client at the API's address.
    #[must_use]
    pub fn board_link(&self, id: &str) -> String {
        self.base.board_link(id)
    }

    /// A link to a folder in the web client at the API's address.
    #[must_use]
    pub fn folder_link(&self, workspace: &str, id: &str) -> String {
        self.base.folder_link(workspace, id)
    }
    /// A snapshot link from the board server, fetched without the login.
    ///
    /// # Errors
    /// A link off the base host, a transport failure or a non-2xx status.
    pub(crate) async fn download_trusted_link(&self, link: &str) -> Result<Vec<u8>> {
        let leg = ExchangeRequest {
            method: HttpMethod::Get,
            url: self.base.trusted_update_link(link)?,
            body: None,
        };
        self.deliver(leg, Authorization::Unsigned)
            .await
            .map(|response| response.body)
    }

    fn state(&self) -> RwLockReadGuard<'_, SessionState> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn transition(&self, step: impl FnOnce(SessionState) -> SessionState) {
        let state = &mut *self.state.write().unwrap_or_else(PoisonError::into_inner);
        *state = step(std::mem::take(state));
    }

    /// Send a described request; the reply as JSON, empty as `null`.
    ///
    /// # Errors
    /// No or revoked login, bad address, non-2xx, foreign redirect, HTML, long
    /// 429 or network failure
    /// ([`crate::error::http::HttpError::NoResponse`] for a write).
    pub fn send_json(
        &self,
        request: ApiRequest,
    ) -> impl Future<Output = Result<Value>> + Send + '_ {
        self.respond(request)
            .map(|reply| reply.and_then(|response| json_or_text(&response)))
    }

    /// [`Client::send_json`] with the reply read as a `T`.
    ///
    /// # Errors
    /// As [`Client::send_json`], plus [`crate::error::http::HttpError::Json`]
    /// when the reply is not a `T`.
    pub async fn send_typed<T: DeserializeOwned>(
        &self,
        request: ApiRequest,
    ) -> Result<T> {
        Ok(serde_json::from_value(self.send_json(request).await?)?)
    }

    /// [`Client::send_json`] with the reply body as text.
    ///
    /// # Errors
    /// As [`Client::send_json`].
    pub fn send_text(
        &self,
        request: ApiRequest,
    ) -> impl Future<Output = Result<String>> + Send + '_ {
        self.respond(request)
            .map(|reply| reply.map(|response| response.text()))
    }

    /// Download bytes (a board snapshot), the address checked as in `send`.
    ///
    /// # Errors
    /// As [`Client::send_json`], plus
    /// [`crate::error::http::HttpError::ResponseTooLarge`].
    pub async fn download(&self, path: &str) -> Result<Vec<u8>> {
        let request = ApiRequest {
            method: HttpMethod::Get,
            path: path.to_owned(),
            query: Vec::new(),
            body: None,
            authorization: Authorization::Session,
        };
        self.respond(request).await.map(|response| response.body)
    }

    fn respond(&self, request: ApiRequest) -> BoxFuture<'_, Result<HttpResponse>> {
        let span = tracing::debug_span!(
            "request",
            method = %request.method,
            path = %request.path.split(['?', '#']).next().unwrap_or_default()
        );
        let authorization = request.authorization;
        let leg = request.url(&self.base).map(|url| ExchangeRequest {
            method: request.method,
            url,
            body: request.body,
        });
        Box::pin(
            async move { self.deliver(leg?, authorization).await }.instrument(span),
        )
    }
}
