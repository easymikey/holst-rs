//! The HTTP client: addresses, login, retries, redirects, reply status.

pub(crate) mod exchange;
#[cfg(test)] pub(crate) mod fake;
pub mod sign;
#[cfg(test)] mod tests;

use std::{
    fmt,
    sync::{
        Arc,
        OnceLock,
        PoisonError,
        RwLock,
        RwLockReadGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::FutureExt as _;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::OnceCell;
use tracing::Instrument as _;

#[cfg(test)] pub(crate) use self::exchange::retry_wait;
use self::exchange::{ExchangeRequest, json_or_text};
use crate::{
    domain::{
        account::{Account, Workspace},
        id_source::IdSource,
        kinds::icon::Manifest,
        node::CreationStamp,
        session::{Session, SessionState, SessionStatus},
        url::BaseUrl,
    },
    effect::{
        cache::{AccountCache, CachedTree, Forget},
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
    shared: Arc<Shared>,
    change: bool,
}

/// What a client and its clones count and keep together.
#[derive(Default)]
struct Shared {
    answered: AtomicU64,
    account_cache: OnceLock<Arc<dyn AccountCache>>,
    icons: OnceCell<Manifest>,
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
            shared: Arc::default(),
            change: false,
        })
    }

    /// How many requests of this client (and its clones) got a success reply.
    #[must_use]
    pub fn answered(&self) -> u64 {
        self.shared.answered.load(Ordering::Relaxed)
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

    /// The icon manifest this client and its clones read, once.
    pub(crate) fn icons(&self) -> &OnceCell<Manifest> {
        &self.shared.icons
    }

    /// The same client, on every clone, keeping the workspace list of its
    /// account in `cache`; the first cache stays.
    #[must_use]
    pub fn with_account_cache(self, cache: Arc<dyn AccountCache>) -> Self {
        drop(self.shared.account_cache.set(cache));
        self
    }

    /// The same client, marked as running a change: a name that picks a board
    /// to write is looked up in trees read within the last hour.
    #[must_use]
    pub fn for_change(self) -> Self {
        Self {
            change: true,
            ..self
        }
    }

    /// The client runs a change (see [`Client::for_change`]), not a read.
    #[must_use]
    pub fn runs_change(&self) -> bool {
        self.change
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
        self.transition(|state| state.log_in(token, &self.base));
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

    /// The active login's account and name became known; a change is saved.
    pub fn learn_account(&self, account: &Account) {
        self.transition(|state| state.learn_account(account));
    }

    /// The workspace list kept for the login's account; `None` when none is
    /// kept, the login has no account id or there is no account cache.
    #[must_use]
    pub fn known_workspaces(&self) -> Option<Vec<Workspace>> {
        let cache = self.shared.account_cache.get()?;
        cache.workspaces(&self.account_id()?)
    }

    /// Keeps the workspace list for the login's account; a failure is logged.
    pub fn learn_workspaces(&self, workspaces: &[Workspace]) {
        self.in_account_cache(|cache, account| {
            cache.keep_workspaces(account, workspaces)
        });
    }

    /// The tree kept for a workspace of the login's account; `None` when none
    /// is kept, the login has no account id or there is no account cache.
    #[must_use]
    pub fn known_tree(&self, workspace: &str) -> Option<CachedTree> {
        let cache = self.shared.account_cache.get()?;
        cache.tree(&self.account_id()?, workspace)
    }

    /// Keeps a workspace's tree reply, asked at Unix milliseconds `started`; a
    /// failure is logged.
    pub fn learn_tree(&self, workspace: &str, text: &str, started: u64) {
        self.in_account_cache(|cache, account| {
            cache.keep_tree(account, workspace, text, started)
        });
    }

    /// Drops the kept tree of one workspace.
    pub fn forget_tree(&self, workspace: &str) {
        let now = self.unix_millis();
        self.in_account_cache(|cache, account| {
            cache.forget(account, Forget::Tree(workspace), now)
        });
    }

    /// Drops every kept tree of the login's account; the list stays.
    pub fn forget_all_trees(&self) {
        let now = self.unix_millis();
        self.in_account_cache(|cache, account| {
            cache.forget(account, Forget::Trees, now)
        });
    }

    /// Drops the kept workspace list of the login's account.
    pub fn forget_workspaces(&self) {
        let now = self.unix_millis();
        self.in_account_cache(|cache, account| {
            cache.forget(account, Forget::Workspaces, now)
        });
    }

    fn in_account_cache(
        &self,
        act: impl FnOnce(&dyn AccountCache, &str) -> Result<()>,
    ) {
        let kept = self
            .shared
            .account_cache
            .get()
            .zip(self.account_id())
            .map(|(cache, account)| act(&**cache, &account));
        kept.and_then(std::result::Result::err)
            .iter()
            .for_each(|err| tracing::debug!(%err, "account cache: not updated"));
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

    /// [`Client::send_json`] with the reply body as bytes.
    ///
    /// # Errors
    /// As [`Client::send_json`], plus
    /// [`crate::error::http::HttpError::ResponseTooLarge`].
    pub fn send_bytes(
        &self,
        request: ApiRequest,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send + '_ {
        self.respond(request)
            .map(|reply| reply.map(|response| response.body))
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
