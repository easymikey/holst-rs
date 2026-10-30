//! The server: login, cache, aliases; store access on the blocking pool.

pub(crate) mod table;
#[cfg(test)] mod tests;

use std::{
    cell::Cell,
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use futures_util::future::OptionFuture;
use holst_core::{
    command::{board::show::account_cache, board_load::CacheUse, table::CommandName},
    domain::session::Session,
    effect::{
        cache::CachePort,
        clock::Clock,
        settings::{CacheSettings, Settings},
    },
    error::{hint::Surface, input::InputError},
    remote::client::Client,
    storage::{cache::Cache, session_store::SessionStoreKind},
};
use rmcp::{
    ErrorData,
    RoleServer,
    ServerHandler,
    model::{
        CallToolRequestParams,
        CallToolResponse,
        CallToolResult,
        ContentBlock,
        Implementation,
        ListToolsResult,
        PaginatedRequestParams,
        ServerCapabilities,
        ServerConfig,
        Tool,
    },
    service::RequestContext,
};
use serde_json::{Map, Value};

use crate::{
    error::Error,
    login::{
        HandedClient,
        LoginPlan,
        LoginState,
        ReadyLogin,
        SharedLogin,
        hand_out,
        machine::{LoginStep, Milliseconds},
        store::{Backend, SystemBackend},
    },
    memory::{McpMemory, board_memory::DownloadTrust, invalidation::FullInvalidation},
    reply::{self},
    server::table::TableTool,
    tools::{admin::RawWrite, files::SecretPaths},
};

tokio::task_local! {
    /// Whether the reply of the running call came from a 401 or 403.
    static UNAUTHORIZED: Cell<bool>;
}

const INSTRUCTIONS: &str = "\
Holst (app.holst.so), unofficial, with the user's rights. Tools are holst CLI \
groups, action = its command; board_write and object_write hold the changes. \
Replies show ids as @N; a reference takes @N, an id, a link or part of a name; \
several matches are listed, not guessed. A frame or object link selects \
that part of a board; so do frame, object and rectangle \"x,y,width,height\" in \
board units. Rights: none, viewer, commentator, editor, full. confirm: true \
only after the user agreed; a risky action without it replies with what it \
would do. dry_run: true sends nothing. output: json (board, object only) gives \
full JSON. No login: run `holst-mcp setup`.";

/// The tools as `tools/list` shows them.
fn listed(table: &[TableTool], raw_write: RawWrite) -> Arc<[Tool]> {
    table.iter().map(|tool| tool.tool(raw_write)).collect()
}

/// Refuses an argument the command does not take on MCP, such as `offline` or
/// the file a picture is written to.
fn only_on_mcp(
    called: CommandName,
    arguments: &Map<String, Value>,
) -> Result<(), holst_core::Error> {
    let command = called.command();
    let taken: Vec<&'static str> = command
        .arguments_on(Surface::Mcp)
        .map(|argument| argument.name)
        .chain(crate::tools::output::on(called).then_some("output"))
        .collect();
    arguments
        .iter()
        .filter(|(name, value)| !value.is_null() && !taken.contains(&name.as_str()))
        .try_for_each(|(name, _)| {
            Err(InputError::NotArgument {
                command: command.path(),
                name: name.clone(),
                taken: taken.clone(),
            }
            .into())
        })
}

#[derive(Clone)]
pub(crate) struct McpServer {
    table: Arc<[TableTool]>,
    tools: Arc<[Tool]>,
    login: Arc<SharedLogin>,
    pub(crate) state: Arc<McpMemory>,
    pub(crate) raw_write: RawWrite,
    pub(crate) secrets: SecretPaths,
    /// Where `object get` puts a file when the call names no place.
    pub(crate) downloads: Option<PathBuf>,
    pub(crate) cache: Arc<CacheSettings>,
}

/// A clock counting milliseconds since the server started.
#[derive(Debug, Clone, Copy)]
struct MonotonicClock(Instant);

impl MonotonicClock {
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "this is the server's clock: the one place that reads the monotonic time"
    )]
    fn start() -> Self {
        Self(Instant::now())
    }
}

impl Clock for MonotonicClock {
    fn unix_millis(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

impl McpServer {
    pub(crate) fn new(
        settings: Settings,
        raw_write: RawWrite,
        secrets: SecretPaths,
    ) -> Self {
        let trust = DownloadTrust(Duration::from_millis(settings.cache.trust_millis));
        let cache = Arc::new(settings.cache.clone());
        let backend = Arc::new(SystemBackend::new(settings));
        Self {
            secrets,
            cache,
            ..Self::with(backend, Arc::new(MonotonicClock::start()))
                .with_raw_write(raw_write)
                .with_download_trust(trust)
        }
    }

    fn with_download_trust(self, trust: DownloadTrust) -> Self {
        Self {
            state: Arc::new(McpMemory::with_download_trust(trust)),
            ..self
        }
    }

    /// Sets the raw-write switch; the `api` tool's hints follow it.
    pub(crate) fn with_raw_write(self, raw_write: RawWrite) -> Self {
        Self {
            tools: listed(&self.table, raw_write),
            raw_write,
            ..self
        }
    }

    #[cfg(test)]
    pub(crate) fn offline() -> Self {
        use crate::login::fake::{Memory, Script};
        Self::with(
            Arc::new(Memory::new(None, Script::new([]))),
            Arc::new(MonotonicClock::start()),
        )
    }

    pub(crate) fn with(backend: Arc<dyn Backend>, clock: Arc<dyn Clock>) -> Self {
        let table: Arc<[TableTool]> = table::tools().into();
        let tools = listed(&table, RawWrite::Denied);
        Self {
            table,
            tools,
            login: Arc::new(SharedLogin::new(backend, clock)),
            state: Arc::new(McpMemory::default()),
            raw_write: RawWrite::Denied,
            secrets: SecretPaths::default(),
            downloads: None,
            cache: Arc::new(CacheSettings::default()),
        }
    }

    #[cfg(test)]
    fn with_cache(self, cache: CacheSettings) -> Self {
        Self {
            cache: Arc::new(cache),
            ..self
        }
    }

    /// The board cache of the login's account; the client keeps the account.
    pub(crate) async fn cache(
        &self,
        client: &HandedClient,
    ) -> Option<Arc<dyn CachePort>> {
        account_cache(client, &self.cache, CacheUse::Use)
            .await
            .ok()
            .flatten()
            .map(Cache::into_shared)
    }

    fn table_tool(&self, name: &str) -> Option<&TableTool> {
        self.table.iter().find(|tool| tool.name() == name)
    }

    /// Runs a call of a table tool: names its command, then the command's
    /// arguments are read against the table.
    async fn call_table(
        &self,
        tool: &TableTool,
        arguments: Option<Map<String, Value>>,
    ) -> CallToolResult {
        let mut arguments = arguments.unwrap_or_default();
        let refused = |err: &dyn std::fmt::Display| reply::error(err.to_string());
        let allowed = tool
            .pick(&mut arguments)
            .map_err(|err| refused(&err))
            .and_then(|command| {
                only_on_mcp(command, &arguments)
                    .map(|()| command)
                    .map_err(|err| refused(&err))
            });
        match allowed {
            Ok(command) => self.command_with_new_login(command, arguments).await,
            Err(reply) => reply,
        }
    }

    /// A 401 on the first request applied nothing: if the store holds another
    /// login by now, the call is run again with it.
    async fn command_with_new_login(
        &self,
        command: CommandName,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        let (before, answered) = {
            let started = self.login.state.load();
            (started.epoch(), started.answered())
        };
        let (first, refused) = UNAUTHORIZED
            .scope(Cell::new(false), async {
                let reply = Box::pin(self.command(command, arguments.clone())).await;
                (reply, UNAUTHORIZED.with(Cell::get))
            })
            .await;
        let revoked = refused && first.is_error == Some(true);
        let seen = self.login.state.load_full();
        // A new client starts at zero; an old one must not have answered more.
        let carried = (seen.epoch() == before).then_some(answered);
        let nothing_done = seen.answered() == carried.unwrap_or_default();
        let renewed =
            OptionFuture::from((revoked && nothing_done).then(|| self.reload(&seen)))
                .await
                .and_then(Result::ok)
                .filter(|fresh| Some(fresh.epoch) != before);
        match renewed {
            Some(_) => Box::pin(self.command(command, arguments)).await,
            None => first,
        }
    }

    pub(crate) fn tool_count(&self) -> usize {
        self.tools.len()
    }

    #[cfg(test)]
    pub(crate) fn listed_tools(&self) -> &[Tool] {
        &self.tools
    }

    pub(crate) async fn store_kind(
        &self,
    ) -> crate::error::Result<Option<SessionStoreKind>> {
        Ok(self.login.load().await?.map(|(_, kind)| kind))
    }

    async fn client(&self) -> crate::error::Result<HandedClient> {
        let seen = self.login.state.load_full();
        match seen.next(self.login.now()) {
            LoginStep::Reload => self.reload(&seen).await,
            LoginStep::Use | LoginStep::Refuse => hand_out(&seen),
        }
    }

    async fn reload(
        &self,
        seen: &Arc<LoginState>,
    ) -> crate::error::Result<HandedClient> {
        let _one_at_a_time = self.login.gate.lock().await;
        let current = self.login.state.load_full();
        if Arc::ptr_eq(seen, &current) {
            self.read_store(&current).await
        } else {
            hand_out(&current)
        }
    }

    async fn read_store(
        &self,
        login: &LoginState,
    ) -> crate::error::Result<HandedClient> {
        let loaded = self.login.load().await;
        let checked_at = self.login.now();
        match loaded {
            Ok(loaded) => match login.plan(loaded, checked_at) {
                LoginPlan::Keep(ready) => self.adopt(LoginState::Ready(ready)),
                LoginPlan::Refuse(revoked) => self.adopt(LoginState::Revoked(revoked)),
                LoginPlan::Forget => self.adopt(LoginState::Missing),
                LoginPlan::Install(new) => {
                    self.install(new.session, new.store, checked_at)
                }
            },
            Err(err) => login
                .kept_unread(checked_at)
                .map_or(Err(err), |ready| self.adopt(LoginState::Ready(ready))),
        }
    }

    fn adopt(&self, login: LoginState) -> crate::error::Result<HandedClient> {
        let login = Arc::new(login);
        self.login.state.store(Arc::clone(&login));
        hand_out(&login)
    }

    fn install(
        &self,
        session: Session,
        store: SessionStoreKind,
        checked_at: Milliseconds,
    ) -> crate::error::Result<HandedClient> {
        let ready = ReadyLogin {
            client: self.login.backend.connect(session.clone())?,
            session,
            epoch: self
                .login
                .epochs
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1),
            checked_at,
            store,
        };
        // The boards are dropped after the client is stored.
        let client = self.adopt(LoginState::Ready(ready));
        self.state.invalidate_all();
        client
    }

    async fn keep_rotated(&self, client: &HandedClient) {
        let rotated = client.rotated().and_then(|session| {
            self.login
                .gate
                .try_lock()
                .ok()
                .map(|gate| self.store_rotated(gate, client, session))
        });
        OptionFuture::from(rotated).await;
    }

    async fn store_rotated(
        &self,
        _one_at_a_time: tokio::sync::MutexGuard<'_, ()>,
        client: &HandedClient,
        session: Session,
    ) {
        let login = self.login.state.load_full();
        let target = match &*login {
            LoginState::Ready(ready) if ready.epoch == client.epoch => Some(ready),
            LoginState::Missing | LoginState::Ready(_) | LoginState::Revoked(_) => None,
        };
        let saving = target
            .map(|ready| self.save_rotated(client, ready.clone(), session.clone()));
        OptionFuture::from(saving).await.unwrap_or_else(|| {
            tracing::debug!("rotated cookies stay in memory: the login was replaced");
            client.confirm_saved(&session);
        });
    }

    async fn save_rotated(
        &self,
        client: &HandedClient,
        ready: ReadyLogin,
        session: Session,
    ) {
        let saved = self
            .login
            .save(session.clone(), ready.store)
            .await
            .inspect_err(|err| tracing::warn!(%err, "could not save rotated cookies"))
            .ok()
            .map(|saved| {
                tracing::debug!(store = saved.store.as_str(), "saved rotated cookies");
                saved.stale.iter().for_each(|stale| {
                    let hint = stale.hint().map(|hint| hint.on(Surface::Mcp).to_string());
                    tracing::warn!(%stale, ?hint, "saved rotated cookies behind an older login");
                });
                client.confirm_saved(&session);
                let next = ReadyLogin {
                    store: saved.store,
                    ..ready.rotated(session.clone())
                };
                self.login.state.store(Arc::new(LoginState::Ready(next.clone())));
                next
            });
        let newer = saved.and_then(|next| {
            client
                .rotated()
                .filter(|newer| *newer != session)
                .map(|newer| Box::pin(self.save_rotated(client, next, newer)))
        });
        OptionFuture::from(newer).await;
    }

    pub(crate) async fn run<F>(&self, call: F) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<String>,
    {
        self.run_with(std::convert::identity, call).await
    }

    async fn run_with<F>(
        &self,
        prepare: fn(Client) -> Client,
        call: F,
    ) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<String>,
    {
        self.run_blocks_with(prepare, async |client| {
            call(client)
                .await
                .map(|text| vec![ContentBlock::text(text)])
        })
        .await
    }

    pub(crate) fn expand(&self, reference: &str) -> crate::error::Result<String> {
        Ok(self.state.expand(reference)?)
    }

    pub(crate) async fn run_blocks<F>(&self, call: F) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<Vec<ContentBlock>>,
    {
        self.run_blocks_with(std::convert::identity, call).await
    }

    async fn run_blocks_with<F>(
        &self,
        prepare: fn(Client) -> Client,
        call: F,
    ) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<Vec<ContentBlock>>,
    {
        async {
            let client = self.client().await?;
            let running = HandedClient {
                client: prepare(client.client.clone()),
                ..client.clone()
            };
            let outcome = call(running).await;
            self.keep_rotated(&client).await;
            outcome
        }
        .await
        .map_or_else(|err| self.fail(&err), CallToolResult::success)
    }

    pub(crate) fn fail(&self, err: &Error) -> CallToolResult {
        let unauthorized = matches!(err, Error::Holst(core) if core.is_unauthorized());
        UNAUTHORIZED
            .try_with(|refused| refused.set(unauthorized))
            .unwrap_or_default();
        reply::error(self.state.alias_text(&reply::explain(err)))
    }

    pub(crate) async fn write<F>(&self, call: F) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<String>,
    {
        let _invalidate = FullInvalidation(&self.state);
        self.run_with(Client::for_change, call).await
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(crate::setup::SERVER, env!("CARGO_PKG_VERSION"))
                    .with_title("Holst (unofficial)"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    /// A table tool runs its command; any other name is an error.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = self.table_tool(&request.name).ok_or_else(|| {
            ErrorData::invalid_params(format!("unknown tool {}", request.name), None)
        })?;
        Ok(self.call_table(tool, request.arguments).await.into())
    }

    /// Our own `tools/list`: the tools of the table.
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.to_vec()))
    }
}
