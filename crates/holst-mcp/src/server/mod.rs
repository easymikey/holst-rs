//! The server: login, cache, aliases; store access on the blocking pool.

pub(crate) mod table;
#[cfg(test)] mod tests;

use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use arc_swap::ArcSwapOption;
use holst_core::{
    command::{lookup, lookup::IndexedNodes, table::CommandName},
    domain::{
        account::Workspace,
        lookup::choice::TreeReads,
        session::Session,
        tree::name_index::build::NameIndex,
    },
    effect::{
        cache::CachePort,
        clock::Clock,
        settings::{CacheSettings, Settings},
    },
    error::{hint::Surface, input::InputError},
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
        LoginStep,
        ReadyLogin,
        SharedLogin,
        hand_out,
        machine::Milliseconds,
        store::{Backend, SystemBackend},
        when,
    },
    memory::{McpMemory, board_memory::DownloadTrust, invalidation::FullInvalidation},
    reply::{self},
    server::table::TableTool,
    tools::{admin::RawWrite, files::SecretPaths},
};

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
    pub(crate) login: Arc<SharedLogin>,
    pub(crate) state: Arc<McpMemory>,
    pub(crate) raw_write: RawWrite,
    pub(crate) secrets: SecretPaths,
    /// Where `object get` puts a file when the call names no place.
    pub(crate) downloads: Option<PathBuf>,
    pub(crate) cache: Arc<CacheSettings>,
    account: Arc<ArcSwapOption<KnownAccount>>,
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

/// Remembers the account name per connection epoch.
struct KnownAccount {
    epoch: u64,
    account_id: String,
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
            account: Arc::new(ArcSwapOption::empty()),
        }
    }

    #[cfg(test)]
    fn with_cache(self, cache: CacheSettings) -> Self {
        Self {
            cache: Arc::new(cache),
            ..self
        }
    }

    pub(crate) async fn cache(
        &self,
        client: &HandedClient,
    ) -> Option<Arc<dyn CachePort>> {
        self.cache.root.as_ref()?;
        let known = self.account.load();
        let known = known
            .as_ref()
            .filter(|known| known.epoch == client.epoch)
            .map(|known| known.account_id.clone());
        let user = match known {
            Some(user) => user,
            None => holst_core::command::account::known_account_id(client).await?,
        };
        self.account.store(Some(Arc::new(KnownAccount {
            epoch: client.epoch,
            account_id: user.clone(),
        })));
        holst_core::command::board_load::account_cache(
            &self.cache,
            &user,
            client.clock(),
        )
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
        match tool.pick(&mut arguments) {
            Ok(command) => match only_on_mcp(command, &arguments) {
                Ok(()) => self.command(command, arguments).await,
                Err(err) => reply::error(err.to_string()),
            },
            Err(err) => reply::error(err.to_string()),
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
        // Generation before link: a login change stores the client first.
        let generation = self.state.generation();
        let seen = self.login.state.load_full();
        match seen.next(self.login.now()) {
            LoginStep::Use(ready) => Ok(HandedClient::new(ready, generation)),
            LoginStep::Refuse => Err(holst_core::Error::Login(
                holst_core::error::login::LoginError::SessionRevoked,
            )
            .into()),
            LoginStep::Reload => self.reload(&seen).await,
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
            hand_out(&current, self.state.generation())
        }
    }

    async fn read_store(
        &self,
        login: &LoginState,
    ) -> crate::error::Result<HandedClient> {
        let generation = self.state.generation();
        let loaded = self.login.load().await?;
        let checked_at = self.login.now();
        match login.plan(loaded, checked_at) {
            LoginPlan::Keep(ready) => self.adopt(LoginState::Ready(ready), generation),
            LoginPlan::Refuse(revoked) => {
                self.adopt(LoginState::Revoked(revoked), generation)
            }
            LoginPlan::Forget => self.adopt(LoginState::Missing, generation),
            LoginPlan::Install(new) => self.install(new.session, new.store, checked_at),
        }
    }

    fn adopt(
        &self,
        login: LoginState,
        generation: u64,
    ) -> crate::error::Result<HandedClient> {
        let login = Arc::new(login);
        self.login.state.store(Arc::clone(&login));
        hand_out(&login, generation)
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
        // Cache dropped after the client is stored — see `client`.
        let client = self.adopt(LoginState::Ready(ready), 0);
        self.state.invalidate_all();
        client.map(|client| HandedClient {
            generation: self.state.generation(),
            ..client
        })
    }

    async fn keep_rotated(&self, client: &HandedClient) {
        let rotated = client.rotated().and_then(|session| {
            self.login
                .gate
                .try_lock()
                .ok()
                .map(|gate| self.store_rotated(gate, client, session))
        });
        when(rotated).await;
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
        when(saving).await.unwrap_or_else(|| {
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
        when(newer).await;
    }

    pub(crate) async fn run<F>(&self, call: F) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<String>,
    {
        self.run_blocks(async |client| {
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
        async {
            let client = self.client().await?;
            let outcome = call(client.clone()).await;
            self.keep_rotated(&client).await;
            outcome
        }
        .await
        .map_or_else(|err| self.fail(&err), CallToolResult::success)
    }

    pub(crate) fn fail(&self, err: &Error) -> CallToolResult {
        reply::error(self.state.alias_text(&reply::explain(err)))
    }

    pub(crate) async fn write<F>(&self, call: F) -> CallToolResult
    where
        F: AsyncFnOnce(HandedClient) -> crate::error::Result<String>,
    {
        let _invalidate = FullInvalidation(&self.state);
        self.run(call).await
    }

    pub(crate) async fn workspaces(
        &self,
        client: &HandedClient,
    ) -> holst_core::Result<Arc<[Workspace]>> {
        match self.state.workspaces() {
            Some(all) => Ok(all),
            None => Ok(self.state.store_workspaces(
                lookup::workspaces(client).await?,
                client.generation,
            )),
        }
    }

    pub(crate) async fn trees(
        &self,
        client: &HandedClient,
        targets: Vec<Workspace>,
    ) -> holst_core::Result<TreeReads<Arc<NameIndex>>> {
        let (ready, missing): (Vec<_>, Vec<_>) = targets
            .into_iter()
            .map(|workspace| (self.state.tree(&workspace.id), workspace))
            .partition(|(index, _)| index.is_some());
        let ready = ready
            .into_iter()
            .filter_map(|(index, workspace)| index.map(|index| (workspace, index)));
        let missing = missing
            .into_iter()
            .map(|(_, workspace)| workspace)
            .collect();
        let (fresh, unread) = holst_core::domain::lookup::choice::partition_reads(
            lookup::trees(client, missing).await,
        )?;
        let stored = fresh.into_iter().map(|(workspace, index)| {
            let index = self
                .state
                .store_tree(&workspace.id, index, client.generation);
            (workspace, index)
        });
        Ok((ready.chain(stored).collect(), unread))
    }

    pub(crate) async fn trash_trees(
        &self,
        client: &HandedClient,
        targets: Vec<Workspace>,
    ) -> holst_core::Result<TreeReads<Arc<NameIndex>>> {
        let read = lookup::indexes(client, targets, IndexedNodes::Trash).await;
        let (found, unread) =
            holst_core::domain::lookup::choice::partition_reads(read)?;
        let found = found
            .into_iter()
            .map(|(workspace, index)| (workspace, Arc::new(index)));
        Ok((found.collect(), unread))
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("holst-rs", env!("CARGO_PKG_VERSION"))
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
