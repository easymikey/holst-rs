#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests use real temp files and real time"
)]

use std::{
    future::Future as _,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll, Waker},
    time::Duration,
};

use holst_core::{
    Error as E,
    domain::session::Session,
    effect::{
        cache::{AccountCache as _, Forget},
        clock::Clock,
        settings::CacheSettings,
    },
    error,
    storage::{cache::account::AccountDisk, session_store::SessionStoreKind},
};
use rmcp::ServerHandler;

use crate::{
    error::{Error, Result},
    login::{
        HandedClient,
        fake::{
            CannedResponse,
            Manual,
            Memory,
            Script,
            account_disk,
            session,
            session_with,
        },
        machine::{RECHECK, REVOKED_RECHECK},
        store::Backend,
    },
    server::McpServer,
};

impl McpServer {
    /// A server whose network is the given replies, in order.
    pub(crate) fn scripted(
        replies: impl IntoIterator<Item = CannedResponse>,
    ) -> (Self, Arc<Script>) {
        let network = Script::new(replies);
        let memory = Memory::new(Some(session("a")), Arc::clone(&network));
        let holst = Self::with(Arc::new(memory), Arc::new(Manual::default()));
        (holst, network)
    }

    /// As `scripted`, with the clients keeping the account's list and trees.
    pub(crate) fn scripted_kept(
        replies: impl IntoIterator<Item = CannedResponse>,
    ) -> holst_core::Result<(Self, Arc<Script>)> {
        let network = Script::new(replies);
        let memory = Memory::new(Some(session("a")), Arc::clone(&network))
            .with_account_cache(account_disk()?);
        let holst = Self::with(Arc::new(memory), Arc::new(Manual::default()));
        Ok((holst, network))
    }
}

fn server(memory: Memory) -> (McpServer, Arc<Memory>, Arc<Manual>) {
    let memory = Arc::new(memory);
    let clock = Arc::new(Manual::default());
    let backend: Arc<dyn Backend> = Arc::<Memory>::clone(&memory);
    let ticks: Arc<dyn Clock> = Arc::<Manual>::clone(&clock);
    (McpServer::with(backend, ticks), memory, clock)
}

fn with_a(
    replies: impl IntoIterator<Item = CannedResponse>,
) -> (McpServer, Arc<Memory>, Arc<Manual>) {
    server(Memory::new(Some(session("a")), Script::new(replies)))
}

fn cookie(value: &str) -> Session {
    session_with("t", &format!("sessionToken={value}"))
}

fn rotating(
    memory: impl FnOnce(Memory) -> Memory,
) -> (McpServer, Arc<Memory>, Arc<Manual>) {
    let network =
        Script::new([CannedResponse::ok_setting("{}", "sessionToken=new; Path=/")]);
    server(memory(Memory::new(Some(cookie("old")), network)))
}

async fn reads(holst: &McpServer) -> bool {
    let call = async |client: HandedClient| {
        holst_core::remote::api::user::current(&client).await?;
        Ok(String::new())
    };
    holst.run(call).await.is_error == Some(false)
}

async fn refusal(holst: &McpServer) -> Option<E> {
    match holst.client().await {
        Err(Error::Holst(err)) => Some(err),
        Ok(_) | Err(_) => None,
    }
}

async fn install(holst: &McpServer) -> Result<(u64, u64)> {
    holst
        .client()
        .await
        .map(|client| (holst.state.generation(), client.epoch))
}

fn spawn_read(holst: &McpServer) -> tokio::task::JoinHandle<bool> {
    let holst = holst.clone();
    tokio::spawn(async move { reads(&holst).await })
}

fn cache(test: &str) -> CacheSettings {
    let dir = std::env::temp_dir()
        .join(format!("holst-mcp-cache-{test}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).unwrap_or_default();
    CacheSettings {
        root: Some(dir.clone()),
        roots: vec![dir],
        host: "app.holst.so".into(),
        ..CacheSettings::default()
    }
}

const HOUR: Duration = Duration::from_hours(1);
const ALICE: &str = "aaaaaaaa-0000-4000-8000-000000000001";

#[tokio::test]
async fn the_disk_cache_is_the_logins_account() -> Result<()> {
    let settings = cache("account");
    let network = Script::new([CannedResponse::ok(
        r#"{"id":"aaaaaaaa-0000-4000-8000-000000000001"}"#,
    )]);
    let (holst, ..) = server(Memory::new(Some(session("a")), Arc::clone(&network)));
    let holst = holst.with_cache(settings.clone());
    let client = holst.client().await?;
    let first = holst.cache(&client).await;
    let second = holst.cache(&client).await;
    let expected = holst_core::storage::cache::Cache::from_settings(
        &settings,
        ALICE,
        holst_core::system::clock::SystemClock,
    )
    .map(|cache| cache.dir().to_owned());
    assert!(first.is_some() && second.is_some());
    assert_eq!(network.seen().len(), 1, "the account was asked twice");
    assert!(expected.is_some_and(|dir| dir.ends_with(ALICE)));
    let (named, ..) = server(Memory::new(
        Some(session("b").with_account_id(ALICE)),
        Script::new([]),
    ));
    let named = named.with_cache(settings.clone());
    let named_conn = named.client().await?;
    assert!(named.cache(&named_conn).await.is_some());
    let (off, ..) = server(Memory::new(Some(session("c")), Arc::clone(&network)));
    let off_conn = off.client().await?;
    assert!(off.cache(&off_conn).await.is_none());
    assert_eq!(
        network.seen().len(),
        1,
        "a cache that is off asked for the account"
    );
    settings
        .roots
        .iter()
        .for_each(|dir| drop(std::fs::remove_dir_all(dir)));
    Ok(())
}

#[tokio::test]
async fn kept_indexes_are_shared_not_copied() -> Result<()> {
    use holst_core::domain::board_text::index::BoardIndex;

    use crate::memory::board_memory::KeptBoard;

    const BOARD: &str = "11111111-2222-4333-8444-555555555555";
    let holst = McpServer::offline();
    let client = HandedClient::offline()?;
    let index = Arc::new(BoardIndex::from_objects(&[], BOARD, Vec::new()));
    let kept = KeptBoard {
        index: Arc::clone(&index),
        document: Some(Arc::from(vec![1, 2, 3])),
        downloaded_at: std::time::Instant::now(),
    };
    holst
        .state
        .store_board(BOARD, kept, holst.state.generation());
    let shared = holst.kept_index(&client, BOARD).await?;
    assert!(Arc::ptr_eq(&shared, &index), "the index was copied");
    let other = "21111111-2222-4333-8444-555555555555";
    let alone = Arc::new(BoardIndex::from_objects(&[], other, Vec::new()));
    holst
        .state
        .store_index(other, Arc::clone(&alone), holst.state.generation());
    assert!(Arc::ptr_eq(
        &holst.kept_index(&client, other).await?,
        &alone
    ));
    assert!(
        holst.kept_board(&client, other).await.is_err(),
        "an index alone was served as a board"
    );
    Ok(())
}

/// The login the fake server accepts.
fn fake_session() -> Session {
    use holst_core::fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN};
    use secrecy::SecretString;

    Session::with_cookies(
        SecretString::from(ACCEPTED_TOKEN),
        ACCEPTED_COOKIES.parse().unwrap_or_default(),
    )
}

/// A login store on the fake; its clients keep the account's trees in `disk`.
struct OnFake {
    base_url: String,
    disk: Arc<AccountDisk>,
    clock: Arc<Manual>,
    stored: Mutex<Session>,
}

impl OnFake {
    fn log_in(&self, session: Session) {
        *self.stored.lock().unwrap_or_else(PoisonError::into_inner) = session;
    }
}

impl Backend for OnFake {
    fn load(&self) -> holst_core::Result<Option<(Session, SessionStoreKind)>> {
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Some((stored.clone(), SessionStoreKind::File)))
    }

    fn save(
        &self,
        session: &Session,
        from: SessionStoreKind,
    ) -> holst_core::Result<holst_core::storage::session_store::LoginSaveReport> {
        self.log_in(session.clone());
        Ok(holst_core::storage::session_store::LoginSaveReport {
            store: from,
            stale: None,
        })
    }

    fn connect(
        &self,
        session: Session,
    ) -> holst_core::Result<holst_core::remote::client::Client> {
        Ok(holst_core::command::login::connect(
            Some(session),
            self.base_url.parse()?,
            Arc::clone(&self.clock),
            holst_core::system::id::RandomIds,
        )?
        .with_account_cache(Arc::<AccountDisk>::clone(&self.disk)))
    }
}

/// A fake server with the board `Plans` of one sticker, and a client on it.
async fn plans() -> holst_core::Result<(holst_core::fake::Fake, HandedClient, String)> {
    plans_of(vec![holst_core::fake::FixtureObject::Sticker(
        "Roadmap".into(),
    )])
    .await
}

/// A fake server with the board `Plans` of `objects`, and a client on it.
async fn plans_of(
    objects: Vec<holst_core::fake::FixtureObject>,
) -> holst_core::Result<(holst_core::fake::Fake, HandedClient, String)> {
    use holst_core::{
        command::{login::connect, lookup},
        system::{clock::SystemClock, id::RandomIds},
    };

    let seed = holst_core::fake::Fixture::default()
        .workspace("Team")
        .board_with_objects("Plans", "Team", objects);
    let fake = holst_core::fake::Fake::start(seed).await?;
    let client = connect(
        Some(fake_session()),
        fake.base_url().parse()?,
        SystemClock,
        RandomIds,
    )?
    .with_account_cache(account_disk()?);
    let board = lookup::board_id(&client, "Plans", None).await?;
    let client = HandedClient { client, epoch: 0 };
    Ok((fake, client, board))
}

/// Adds a sticker to `board` on the fake, as another user of the board would.
async fn add_sticker(
    client: &HandedClient,
    board: &str,
    text: &str,
) -> holst_core::Result<()> {
    use holst_core::{
        command::board_write::{WriteRequest, write_cached},
        domain::{
            change::value::{BoardChange, NewEntry, NewObject},
            geometry::Point,
        },
        effect::confirmation::ALWAYS,
        remote::socket::write::{WriteOver, connector},
    };

    let operation = BoardChange::AddAll(vec![NewEntry {
        new: NewObject::Sticker {
            text: text.into(),
            at: Point::new(10.0, 20.0)?,
            size: None,
            color: None,
        },
        frame: None,
        format: holst_core::domain::rich_text::TextFormat::Plain,
    }]);
    let user = holst_core::command::account::account_id(&client.client).await?;
    write_cached(
        &client.client,
        &WriteOver::sender(connector()),
        WriteRequest {
            board,
            change: &operation,
            send_if: ALWAYS,
            account_id: &user,
            kept: None,
        },
        None,
    )
    .await
    .map(|_| ())
    .map_err(E::from)
}

/// Makes the kept copy of `board` one the server has not confirmed lately.
fn unconfirm(holst: &McpServer, board: &str) {
    holst.state.take_board(board).into_iter().for_each(|kept| {
        holst
            .state
            .store_unconfirmed_board(board, kept, holst.state.generation());
    });
}

fn texts(kept: &crate::memory::board_memory::KeptBoard) -> Vec<String> {
    let mut shown: Vec<String> = (0..kept.index.len())
        .map(|at| kept.index.text(at).to_owned())
        .collect();
    shown.sort();
    shown
}

#[tokio::test]
async fn a_board_is_loaded_without_a_cache_and_kept() -> holst_core::Result<()> {
    let (_fake, client, board) = plans().await?;
    let holst = McpServer::offline();
    assert!(holst.state.board(&board).is_none());
    let index = holst.kept_index(&client, &board).await?;
    assert_eq!(index.len(), 1);
    assert!(
        holst.state.board(&board).is_some(),
        "the board was not kept"
    );
    Ok(())
}

#[tokio::test]
async fn an_unchanged_board_reuses_its_kept_copy() -> holst_core::Result<()> {
    let (_fake, client, board) = plans().await?;
    let holst = McpServer::offline();
    let first = holst.kept_board(&client, &board).await?;
    unconfirm(&holst, &board);
    let again = holst.kept_board(&client, &board).await?;
    assert_eq!(texts(&again), ["Roadmap"]);
    assert!(
        first
            .document
            .as_ref()
            .zip(again.document.as_ref())
            .is_some_and(|(before, after)| Arc::ptr_eq(before, after)),
        "the kept copy was packed again"
    );
    Ok(())
}

#[tokio::test]
async fn a_changed_board_is_read_again_into_its_new_object() -> holst_core::Result<()> {
    let (_fake, client, board) = plans().await?;
    let holst = McpServer::offline();
    let first = holst.kept_board(&client, &board).await?;
    add_sticker(&client, &board, "Buy milk").await?;
    assert_eq!(
        texts(&holst.kept_board(&client, &board).await?),
        ["Roadmap"],
        "a copy the server confirmed lately was read again"
    );
    unconfirm(&holst, &board);
    let again = holst.kept_board(&client, &board).await?;
    assert_eq!(texts(&again), ["Buy milk", "Roadmap"]);
    assert!(
        first
            .document
            .as_ref()
            .zip(again.document.as_ref())
            .is_some_and(|(before, after)| !Arc::ptr_eq(before, after)),
        "the old copy was served"
    );
    Ok(())
}

#[test]
fn tools_list_snapshot() {
    let holst = McpServer::offline();
    insta::assert_json_snapshot!("tools_list", *holst.tools);
}

#[tokio::test]
async fn locked_store_is_reported_and_retried() {
    let (holst, memory, _) =
        server(Memory::new(Some(session("a")), Script::new([])).locked());
    assert!(matches!(
        refusal(&holst).await,
        Some(E::LocalData(
            error::local_data::LocalDataError::KeychainLocked
        ))
    ));
    assert!(refusal(&holst).await.is_some());
    assert_eq!(memory.loads(), 2, "a failed read was cached");
}

#[tokio::test]
async fn working_login_survives_a_keychain_that_locks_later() {
    let (holst, memory, clock) = with_a([]);
    assert!(reads(&holst).await, "the first call failed");
    memory.lock_now();
    clock.advance(RECHECK);
    assert!(
        reads(&holst).await,
        "the recheck of a locked store failed a working login"
    );
}

#[tokio::test]
async fn new_login_in_the_store_is_used_when_the_old_one_is_refused() {
    let replies = [
        CannedResponse::ok(ME),
        CannedResponse::unauthorized(),
        CannedResponse::ok(ME),
    ];
    let (holst, memory, _) =
        server(Memory::new(Some(cookie("a")), Script::new(replies)));
    let list = serde_json::json!({ "action": "ls" });
    let (first, opened) = table_call(&holst, "workspace", list.clone()).await;
    assert!(!first, "{opened}");
    memory.store(Some(cookie("b")));
    let (second, text) = table_call(&holst, "workspace", list).await;
    assert!(!second, "{text}");
}

#[tokio::test]
async fn new_login_is_used_when_a_bare_token_is_refused() {
    let replies = [
        CannedResponse::ok(ME),
        CannedResponse::unauthorized(),
        CannedResponse::ok(ME),
    ];
    let (holst, memory, _) =
        server(Memory::new(Some(session("a")), Script::new(replies)));
    let list = serde_json::json!({ "action": "ls" });
    let (first, opened) = table_call(&holst, "workspace", list.clone()).await;
    assert!(!first, "{opened}");
    memory.store(Some(session("b")));
    let (second, text) = table_call(&holst, "workspace", list).await;
    assert!(!second, "{text}");
}

#[tokio::test]
async fn call_is_not_repeated_when_an_earlier_request_landed() -> Result<()> {
    let replies = [
        CannedResponse::ok(ME),
        CannedResponse::unauthorized(),
        CannedResponse::ok(ME),
        CannedResponse::ok("{}"),
    ];
    let network = Script::new(replies);
    let (holst, memory, _) =
        server(Memory::new(Some(cookie("a")), Arc::clone(&network)));
    install(&holst).await?;
    memory.store(Some(cookie("b")));
    let rename = serde_json::json!({
        "action": "rename", "workspace": "easy", "name": "Crew", "confirm": true
    });
    let (failed, text) = table_call(&holst, "workspace", rename).await;
    assert!(failed, "{text}");
    assert_eq!(network.seen().len(), 2, "the call was done twice");
    Ok(())
}

#[tokio::test]
async fn store_is_rechecked_only_after_interval() -> Result<()> {
    let (holst, memory, clock) = with_a([]);
    let first = install(&holst).await?;
    install(&holst).await?;
    assert_eq!(memory.loads(), 1, "the store was read before RECHECK");

    clock.advance(RECHECK);
    assert_eq!(
        install(&holst).await?,
        first,
        "the same login dropped the cache"
    );
    assert_eq!(memory.loads(), 2);

    memory.store(Some(session("b")));
    clock.advance(RECHECK);
    let (generation, epoch) = install(&holst).await?;
    assert_ne!(generation, first.0, "a new login kept the cache");
    assert_ne!(epoch, first.1);
    Ok(())
}

#[tokio::test]
async fn revoked_login_is_refused_without_retries() {
    let network = Script::new([CannedResponse::unauthorized()]);
    let (holst, memory, clock) =
        server(Memory::new(Some(session("a")), Arc::clone(&network)));

    assert!(!reads(&holst).await);
    assert_eq!((network.seen().len(), memory.loads()), (1, 1));
    assert!(matches!(
        refusal(&holst).await,
        Some(E::Login(error::login::LoginError::SessionRevoked))
    ));
    assert_eq!(
        memory.loads(),
        2,
        "the store was not read again after the 401"
    );

    assert!(!reads(&holst).await);
    assert!(matches!(
        refusal(&holst).await,
        Some(E::Login(error::login::LoginError::SessionRevoked))
    ));
    let counts = (network.seen().len(), memory.loads());
    assert_eq!(counts, (1, 2), "a revoked login was retried");

    clock.advance(REVOKED_RECHECK);
    assert!(refusal(&holst).await.is_some());
    assert_eq!(
        memory.loads(),
        3,
        "the store was not read after REVOKED_RECHECK"
    );
}

#[tokio::test]
async fn new_login_after_revocation_is_picked_up() -> Result<()> {
    let (holst, memory, clock) = with_a([CannedResponse::unauthorized()]);
    let before = install(&holst).await?.0;
    assert!(!reads(&holst).await);

    memory.store(Some(session("b")));
    clock.advance(REVOKED_RECHECK);
    let after = install(&holst).await?.0;
    assert_ne!(after, before, "the cache of the revoked login survived");
    assert!(reads(&holst).await);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_calls_share_one_store_read() {
    let (holst, memory, _) = server(
        Memory::new(Some(session("a")), Script::new([]))
            .slow(Duration::from_millis(50)),
    );
    let epochs: Vec<Option<u64>> = (0..8)
        .map(|_| {
            let holst = holst.clone();
            async move { install(&holst).await.ok().map(|(_, epoch)| epoch) }
        })
        .collect::<tokio::task::JoinSet<_>>()
        .join_all()
        .await;
    assert_eq!(memory.loads(), 1, "the store was read by several calls");
    assert!(epochs.iter().all(|epoch| *epoch == Some(1)), "{epochs:?}");
}

#[tokio::test]
async fn rotated_cookies_are_saved_once_and_keep_the_cache() -> Result<()> {
    let (holst, memory, clock) = rotating(|memory| memory);
    let first = install(&holst).await?;

    assert!(reads(&holst).await && reads(&holst).await);
    assert_eq!(memory.saved(), [cookie("new")]);

    clock.advance(RECHECK);
    assert_eq!(install(&holst).await?, first, "rotation dropped the cache");
    Ok(())
}

#[tokio::test]
async fn rotation_is_saved_where_the_login_was_read() -> Result<()> {
    let (holst, memory, _) = rotating(Memory::in_file);
    install(&holst).await?;
    assert!(reads(&holst).await);
    assert_eq!(memory.saved_to(), [SessionStoreKind::File]);
    Ok(())
}

#[tokio::test]
async fn failed_save_keeps_the_client() -> Result<()> {
    let (holst, memory, clock) = rotating(Memory::read_only);
    let first = install(&holst).await?;

    assert!(reads(&holst).await);
    assert_eq!(
        memory.saved_to().len(),
        1,
        "the rotation was not offered to the store"
    );
    assert_eq!(memory.saved(), Vec::<Session>::new());

    clock.advance(RECHECK);
    assert_eq!(
        install(&holst).await?,
        first,
        "a failed save looked like a new login"
    );
    assert_eq!(holst.client().await?.session(), Some(cookie("new")));
    Ok(())
}

#[tokio::test]
async fn failed_save_is_retried_on_the_next_call() {
    let (holst, memory, _) = rotating(Memory::read_only);

    assert!(reads(&holst).await);
    assert_eq!(memory.saved_to().len(), 1);
    assert_eq!(memory.saved(), Vec::<Session>::new());

    memory.make_writable();
    assert!(reads(&holst).await);
    assert_eq!(
        memory.saved_to().len(),
        2,
        "the refused rotation was not offered again"
    );
    assert_eq!(
        memory.saved(),
        [cookie("new")],
        "not stored on the second try"
    );

    assert!(reads(&holst).await);
    assert_eq!(
        memory.saved_to().len(),
        2,
        "a stored rotation was offered again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rotation_during_a_save_is_saved_too() -> Result<()> {
    let network = Script::new([
        CannedResponse::ok_setting("{}", "sessionToken=s1; Path=/"),
        CannedResponse::ok_setting("{}", "sessionToken=s2; Path=/"),
    ]);
    let (holst, memory, _) = server(
        Memory::new(Some(cookie("s0")), network).slow_save(Duration::from_millis(300)),
    );
    install(&holst).await?;
    let first = spawn_read(&holst);
    memory.saving.notified().await;

    assert!(reads(&holst).await);
    assert_eq!(first.await.ok(), Some(true));
    let last = memory.saved().last().cloned();
    assert_eq!(
        last,
        Some(cookie("s2")),
        "the newer rotation stayed in memory"
    );
    Ok(())
}

#[tokio::test]
async fn clock_set_back_does_not_prolong_a_refusal() {
    let (holst, memory, clock) = with_a([CannedResponse::unauthorized()]);
    clock.advance(Duration::from_hours(2));
    assert!(!reads(&holst).await);
    assert!(matches!(
        refusal(&holst).await,
        Some(E::Login(error::login::LoginError::SessionRevoked))
    ));

    memory.store(Some(session("b")));
    clock.rewind(Duration::from_hours(1));
    let loads = memory.loads();
    assert!(reads(&holst).await);
    assert_eq!(memory.loads(), loads + 1, "the store was not read again");
}

#[tokio::test(flavor = "current_thread")]
async fn hanging_save_does_not_block_other_calls() -> Result<()> {
    let (holst, memory, _) =
        rotating(|memory| memory.slow_save(Duration::from_millis(300)));
    install(&holst).await?;
    let saving = spawn_read(&holst);
    memory.saving.notified().await;

    assert!(reads(&holst).await);
    assert!(!saving.is_finished(), "the other call waited for the save");
    assert_eq!(saving.await.ok(), Some(true));
    assert_eq!(memory.saved(), [cookie("new")]);
    Ok(())
}

#[tokio::test]
async fn cancelled_write_still_drops_the_cache() -> Result<()> {
    let (holst, ..) = with_a([CannedResponse::Hang]);
    let before = install(&holst).await?.0;
    // Dropped at statement end: cancels the write.
    let poll = std::pin::pin!(holst.write(async |client| {
        client
            .send_json(holst_core::remote::api::channel::rename_request(
                "b1",
                &holst_core::remote::api::channel::Rename::new("x"),
            )?)
            .await?;
        Ok("renamed".to_owned())
    }))
    .as_mut()
    .poll(&mut Context::from_waker(Waker::noop()));
    assert!(matches!(poll, Poll::Pending), "the request did not hang");

    assert_ne!(
        holst.state.generation(),
        before,
        "a cancelled write kept the cache"
    );
    assert!(reads(&holst).await);
    Ok(())
}

const ME: &str = r#"{"id":"00000003-0000-4000-8000-000000000001","name":"Ann",
    "workspaces":[{"id":"00000002-0000-4000-8000-000000000001","name":"easy"}]}"#;
const TREE: &str = r#"{"channels":[
    {"id":"00000001-0000-4000-8000-000000000001","type":"folder","name":"Plans"},
    {"id":"00000001-0000-4000-8000-000000000002","type":"board","name":"Roadmap"},
    {"id":"00000001-0000-4000-8000-000000000003","type":"board","name":"Old","in_trash":true}]}"#;

/// What a call to a table tool replies, and whether it is a tool error.
pub(super) async fn table_call(
    holst: &McpServer,
    tool: &str,
    arguments: serde_json::Value,
) -> (bool, String) {
    match holst.table_tool(tool) {
        Some(found) => {
            let result = holst
                .call_table(found, arguments.as_object().cloned())
                .await;
            let text = result
                .content
                .iter()
                .filter_map(|block| block.as_text())
                .map(|block| block.text.as_str())
                .collect();
            (result.is_error == Some(true), text)
        }
        None => (true, format!("the table has no tool {tool}")),
    }
}

#[tokio::test]
async fn an_argument_the_command_does_not_take_is_a_tool_error_in_our_words() {
    let (holst, network) = McpServer::scripted([]);
    let (failed, text) = table_call(
        &holst,
        "auth",
        serde_json::json!({ "action": "status", "cover": "x.png" }),
    )
    .await;
    assert!(failed);
    assert_eq!(
        text,
        "\"cover\" is not an argument of `auth status`: it takes with"
    );
    assert!(network.seen().is_empty(), "the call went to the network");
}

#[tokio::test]
async fn auth_has_no_login_action_on_mcp() {
    let (holst, network) = McpServer::scripted([]);
    let (failed, text) =
        table_call(&holst, "auth", serde_json::json!({ "action": "login" })).await;
    assert!(failed);
    assert!(
        text.contains("`auth` has no action \"login\"; it has: status"),
        "{text}"
    );
    assert!(network.seen().is_empty(), "the call went to the network");
}

#[tokio::test]
async fn auth_without_a_login_names_the_setup_command() {
    let (holst, ..) = server(Memory::new(None, Script::new([])));
    let (failed, text) =
        table_call(&holst, "auth", serde_json::json!({ "action": "status" })).await;
    assert!(!failed, "{text}");
    assert_eq!(
        text,
        "no login: ask the user to run `holst-mcp setup` in a terminal, then call auth again"
    );
}

#[tokio::test]
async fn a_login_saved_by_another_process_is_taken_on_the_next_call() {
    let network = Script::new([CannedResponse::ok(ME), CannedResponse::ok(TREE)]);
    let (holst, memory, _) = server(Memory::new(None, Arc::clone(&network)));
    let (failed, text) =
        table_call(&holst, "workspace", serde_json::json!({ "action": "ls" })).await;
    assert!(failed, "{text}");
    assert!(text.contains("holst-mcp setup"), "{text}");

    memory.store(Some(session("a")));
    let (failed_after, status) =
        table_call(&holst, "auth", serde_json::json!({ "action": "status" })).await;
    assert!(!failed_after, "{status}");
    assert!(status.starts_with("logged in: Ann @"), "{status}");
}

#[test]
fn auth_is_read_only_in_the_tool_list() {
    let holst = McpServer::offline();
    let hints = holst
        .listed_tools()
        .iter()
        .find(|tool| tool.name == "auth")
        .and_then(|tool| tool.annotations.as_ref())
        .map(|hints| (hints.read_only_hint, hints.destructive_hint));
    assert_eq!(hints, Some((Some(true), None)));
}

#[tokio::test]
async fn workspace_rename_without_confirm_sends_nothing() {
    let network = Script::new([CannedResponse::ok(ME)]);
    let (holst, ..) = server(Memory::new(Some(session("a")), Arc::clone(&network)));
    let (failed, text) = table_call(
        &holst,
        "workspace",
        serde_json::json!({ "action": "rename", "workspace": "easy", "name": "Crew" }),
    )
    .await;
    assert!(failed, "{text}");
    assert!(text.contains("; confirm with confirm: true"), "{text}");
    assert_eq!(
        network.seen().len(),
        1,
        "only the lookup: {:?}",
        network.seen()
    );
}

#[tokio::test]
async fn workspace_rename_with_confirm_sends_the_new_name() {
    let network = Script::new([CannedResponse::ok(ME), CannedResponse::ok("{}")]);
    let (holst, ..) = server(Memory::new(Some(session("a")), Arc::clone(&network)));
    let (failed, text) = table_call(
        &holst,
        "workspace",
        serde_json::json!({
            "action": "rename", "workspace": "easy", "name": "Crew", "confirm": true
        }),
    )
    .await;
    assert!(!failed, "{text}");
    assert!(text.starts_with("renamed easy to Crew @"), "{text}");
    assert_eq!(network.seen().len(), 2, "{:?}", network.seen());
}

#[test]
fn instructions_explain_the_tools_and_the_login() {
    let instructions = McpServer::offline()
        .get_info()
        .instructions
        .unwrap_or_default();
    let wanted = [
        "object_write",
        "x,y,width,height",
        "holst-mcp setup",
        "@N",
        "output: json (board, object only)",
    ];
    assert!(
        wanted.iter().all(|word| instructions.contains(word)),
        "{instructions}"
    );
    assert!(!instructions.contains("raw: true"), "{instructions}");
}

#[test]
fn instructions_say_objects_and_threads_have_no_name_lookup() {
    let instructions = McpServer::offline()
        .get_info()
        .instructions
        .unwrap_or_default();
    assert!(
        instructions.contains("object or thread needs @N or id"),
        "objects and threads are found by @N, id or link only: {instructions}"
    );
    assert!(
        !instructions.contains("a reference takes"),
        "one rule for every reference is wrong: {instructions}"
    );
}

#[rstest::rstest]
#[case::board_show("board", serde_json::json!({ "action": "show", "board": "X", "offline": true }))]
#[case::board_snap("board", serde_json::json!({ "action": "snap", "board": "X", "offline": true }))]
#[case::object("object", serde_json::json!({ "board": "X", "offline": true }))]
#[tokio::test]
async fn offline_reads_only_the_cache(
    #[case] tool: &str,
    #[case] call: serde_json::Value,
) {
    let (holst, network) = McpServer::scripted([]);
    let (failed, text) = table_call(&holst, tool, call).await;
    assert!(failed, "{text}");
    assert!(!text.contains("is not an argument"), "{text}");
    assert!(network.seen().is_empty(), "the call went to the network");
}

#[tokio::test]
async fn embed_gives_the_picture_inline() -> holst_core::Result<()> {
    let (_fake, holst, _) = on_fake().await?;
    let call = serde_json::json!({
        "action": "snap", "board": "Plans", "picture_format": "svg", "embed": true
    });
    let (failed, text) = table_call(&holst, "board", call).await;
    assert!(!failed, "{text}");
    assert!(text.contains("<svg"), "{text}");
    Ok(())
}

/// What the command line prints with `-o json`.
fn printed(reply: &impl serde::Serialize) -> holst_core::Result<String> {
    Ok(serde_json::to_string(&serde_json::to_value(reply)?)?)
}

fn given(arguments: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_value(arguments).unwrap_or_default()
}

/// A server on the fake of `plans`, and a client of the command line's on it.
async fn on_fake()
-> holst_core::Result<(holst_core::fake::Fake, McpServer, HandedClient)> {
    on_fake_of(vec![holst_core::fake::FixtureObject::Sticker(
        "Roadmap".into(),
    )])
    .await
}

async fn on_fake_of(
    objects: Vec<holst_core::fake::FixtureObject>,
) -> holst_core::Result<(holst_core::fake::Fake, McpServer, HandedClient)> {
    let (fake, client, _) = plans_of(objects).await?;
    let (holst, ..) = served(&fake)?;
    Ok((fake, holst, client))
}

/// A server on `fake`, its login store, and the clock of the server's login.
fn served(
    fake: &holst_core::fake::Fake,
) -> holst_core::Result<(McpServer, Arc<OnFake>, Arc<Manual>)> {
    let clock = Arc::new(Manual::default());
    clock.advance(HOUR * 10);
    let login = Arc::new(OnFake {
        base_url: fake.base_url().to_owned(),
        disk: account_disk()?,
        clock,
        stored: Mutex::new(fake_session()),
    });
    let ticks = Arc::new(Manual::default());
    let backend: Arc<dyn Backend> = Arc::<OnFake>::clone(&login);
    let holst = McpServer::with(backend, Arc::<Manual>::clone(&ticks));
    Ok((holst, login, ticks))
}

#[tokio::test]
async fn object_json_is_what_the_command_line_prints() -> holst_core::Result<()> {
    use holst_core::command::{object, table::parse};

    let (_fake, holst, client) = on_fake().await?;
    let args = parse::<object::LsArgs>(given(
        serde_json::json!({ "board": "Plans", "limit": 25 }),
    ))?;
    let listing = object::list::ls(&client, &CacheSettings::default(), &args).await?;
    let call = serde_json::json!({ "board": "Plans", "output": "json" });
    let (failed, text) = table_call(&holst, "object", call).await;
    assert!(!failed, "{text}");
    assert_eq!(text, printed(&listing)?);
    Ok(())
}

#[tokio::test]
async fn board_show_json_is_what_the_command_line_prints() -> holst_core::Result<()> {
    use holst_core::command::{board, table::parse};

    let (_fake, holst, client) = on_fake().await?;
    let args =
        parse::<board::ShowArgs>(given(serde_json::json!({ "board": "Plans" })))?;
    let shown = board::show::show(&client, &CacheSettings::default(), &args).await?;
    let call =
        serde_json::json!({ "action": "show", "board": "Plans", "output": "json" });
    let (failed, text) = table_call(&holst, "board", call).await;
    assert!(!failed, "{text}");
    assert_eq!(text, printed(&shown)?);
    Ok(())
}

#[tokio::test]
async fn board_ls_json_is_what_the_command_line_prints() -> holst_core::Result<()> {
    use holst_core::command::{board, table::parse};

    let (_fake, holst, client) = on_fake().await?;
    let args = parse::<board::LsArgs>(given(serde_json::json!({ "limit": 300 })))?;
    let tree = board::tree::ls(&client, &args).await?;
    let call = serde_json::json!({ "action": "ls", "output": "json" });
    let (failed, text) = table_call(&holst, "board", call).await;
    assert!(!failed, "{text}");
    assert_eq!(text, printed(&tree)?);
    Ok(())
}

#[tokio::test]
async fn output_text_is_the_default_reply() -> holst_core::Result<()> {
    let (_fake, holst, _) = on_fake().await?;
    let plain = serde_json::json!({ "board": "Plans" });
    let text = serde_json::json!({ "board": "Plans", "output": "text" });
    let (failed, listed) = table_call(&holst, "object", plain).await;
    assert!(!failed && listed.contains("Roadmap"), "{listed}");
    assert_eq!(table_call(&holst, "object", text).await, (false, listed));
    Ok(())
}

#[rstest::rstest]
#[case::object(
    "object",
    serde_json::json!({ "board": "X", "output": "yaml" }),
    "output of `object ls` must be one of text, json"
)]
#[case::board_show(
    "board",
    serde_json::json!({ "action": "show", "board": "X", "output": "yaml" }),
    "output of `board show` must be one of text, json"
)]
#[case::board_snap(
    "board",
    serde_json::json!({ "action": "snap", "board": "X", "output": "json" }),
    "\"output\" is not an argument of `board snap`"
)]
#[tokio::test]
async fn an_output_but_text_or_json_is_refused_before_any_request(
    #[case] tool: &str,
    #[case] call: serde_json::Value,
    #[case] words: &str,
) {
    let (holst, network) = McpServer::scripted([]);
    let (failed, text) = table_call(&holst, tool, call).await;
    assert!(failed && text.starts_with(words), "{text}");
    assert!(network.seen().is_empty(), "the call went to the network");
}

#[tokio::test]
async fn a_file_for_a_picture_is_the_command_lines() {
    let (holst, ..) = McpServer::scripted([]);
    let refused = async |extra: (&str, serde_json::Value)| {
        let given =
            serde_json::json!({ "action": "snap", "board": "b", extra.0: extra.1 });
        table_call(&holst, "board", given).await.0
    };
    assert!(refused(("file", "x.png".into())).await);
}

/// Stickers with a sentence each inside frames, arrows between them, shapes and
/// free text: the richest kinds, `count` of them.
fn rich_objects(count: u32) -> Vec<holst_core::fake::FixtureObject> {
    use holst_core::fake::FixtureObject;

    (1..=count)
        .map(|n| match n % 5 {
            0 => FixtureObject::Frame(format!("Quarter {n} roadmap")),
            1 | 2 => FixtureObject::Sticker(format!(
                "Retro note {n}: ship the sync fix before the quarter closes and tell the team"
            ))
            .inside((n / 5 + 1) * 5),
            3 => FixtureObject::Arrow(n - 2, n - 1),
            _ => FixtureObject::Shape(format!("Decision {n}")).inside((n / 5 + 1) * 5),
        })
        .collect()
}

#[tokio::test]
async fn a_default_json_page_of_the_richest_kinds_fits_the_limit()
-> holst_core::Result<()> {
    let (_fake, holst, _) = on_fake_of(rich_objects(300)).await?;
    let default = serde_json::json!({ "board": "Plans", "output": "json" });
    let (failed, page) = table_call(&holst, "object", default).await;
    // 100 of these objects are 39 111 bytes of JSON, 25 are about a quarter.
    assert!(!failed && page.len() < 16_000, "{failed} {}", page.len());
    let hundred =
        serde_json::json!({ "board": "Plans", "output": "json", "limit": 100 });
    let (refused, refusal) = table_call(&holst, "object", hundred).await;
    assert!(
        refused && refusal.contains("pass a lower `limit`"),
        "{refusal}"
    );
    Ok(())
}

const FAKE_USER: &str = "00000000-0000-4000-8000-000000000000";

async fn plans_fake() -> holst_core::Result<holst_core::fake::Fake> {
    let seed = holst_core::fake::Fixture::default()
        .workspace("Team")
        .board("Plans", "Team", None);
    Ok(holst_core::fake::Fake::start(seed).await?)
}

/// What a call replies; the call must not fail.
async fn replied(
    holst: &McpServer,
    tool: &str,
    arguments: serde_json::Value,
) -> String {
    let (failed, text) = table_call(holst, tool, arguments).await;
    assert!(!failed, "{text}");
    text
}

fn show_plans() -> serde_json::Value {
    serde_json::json!({ "action": "show", "board": "Plans" })
}

fn add_note() -> serde_json::Value {
    serde_json::json!({ "action": "add", "board": "Plans", "text": "note" })
}

/// The requests for `last` the fake got after the log was `since` long.
fn asked(fake: &holst_core::fake::Fake, since: usize, last: &str) -> usize {
    fake.requests()
        .split_off(since)
        .iter()
        .filter(|request| request.ends_with(last))
        .count()
}

#[tokio::test]
async fn a_second_board_show_by_name_asks_no_account_and_no_tree()
-> holst_core::Result<()> {
    let fake = plans_fake().await?;
    let (holst, ..) = served(&fake)?;
    replied(&holst, "board", show_plans()).await;
    let before = fake.requests().len();
    replied(&holst, "board", show_plans()).await;
    assert_eq!(asked(&fake, before, "/current_user"), 0);
    assert_eq!(asked(&fake, before, "/tree"), 0);
    Ok(())
}

#[tokio::test]
async fn a_board_show_by_name_after_a_search_reads_no_tree() -> holst_core::Result<()> {
    let fake = plans_fake().await?;
    let (holst, ..) = served(&fake)?;
    let search = serde_json::json!({ "query": "plans", "mode": "names" });
    assert!(replied(&holst, "search", search).await.contains("Plans"));
    let before = fake.requests().len();
    replied(&holst, "board", show_plans()).await;
    assert_eq!(asked(&fake, before, "/tree"), 0);
    Ok(())
}

#[tokio::test]
async fn an_object_write_by_name_asks_a_tree_older_than_an_hour_once()
-> holst_core::Result<()> {
    let fake = plans_fake().await?;
    let (holst, login, _) = served(&fake)?;
    replied(&holst, "board", show_plans()).await;
    let before = fake.requests().len();
    replied(&holst, "object_write", add_note()).await;
    assert_eq!(asked(&fake, before, "/tree"), 0, "a young tree was asked");
    login.clock.advance(HOUR * 2);
    let later = fake.requests().len();
    replied(&holst, "object_write", add_note()).await;
    assert_eq!(asked(&fake, later, "/tree"), 1);
    Ok(())
}

#[tokio::test]
async fn a_new_login_reads_its_account_and_list_from_the_store_and_asks_the_trees()
-> Result<()> {
    let fake = plans_fake().await?;
    let (holst, login, ticks) = served(&fake)?;
    replied(&holst, "board", show_plans()).await;
    let first = holst.client().await?.epoch;
    // What `holst-mcp setup` leaves: the list fresh, no trees.
    let list = login.disk.workspaces(FAKE_USER).unwrap_or_default();
    login.disk.forget(FAKE_USER, Forget::Trees, 0)?;
    login.disk.keep_workspaces(FAKE_USER, &list)?;
    login.log_in(fake_session().with_account_id(FAKE_USER));
    ticks.advance(RECHECK);
    let before = fake.requests().len();
    replied(&holst, "board", show_plans()).await;
    assert_ne!(
        holst.client().await?.epoch,
        first,
        "the login was not renewed"
    );
    assert_eq!(asked(&fake, before, "/current_user"), 0);
    assert_eq!(asked(&fake, before, "/tree"), 1);
    Ok(())
}

#[test]
fn the_system_backends_clients_keep_the_account_list_on_disk() -> holst_core::Result<()>
{
    use holst_core::{domain::account::Workspace, effect::settings::Settings};

    let home =
        std::env::temp_dir().join(format!("holst-mcp-system-{}", std::process::id()));
    std::fs::remove_dir_all(&home).unwrap_or_default();
    let variables = [
        ("HOLST_HOME", home.clone().into_os_string()),
        ("HOLST_CACHE_DIR", home.join("cache").into_os_string()),
    ];
    let settings = Settings::from_variables(|name| {
        variables
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    })?;
    let client = crate::login::store::SystemBackend::new(settings)
        .connect(session("a").with_account_id(ALICE))?;
    client.learn_workspaces(&[Workspace {
        id: "bbbbbbbb-0000-4000-8000-000000000001".into(),
        name: "Team".into(),
    }]);
    let kept = client.known_workspaces();
    std::fs::remove_dir_all(&home).unwrap_or_default();
    assert_eq!(kept.map(|list| list.len()), Some(1));
    Ok(())
}

const TEAM: &str = "00000000-0000-4000-8000-000000000001";
const OTHER: &str = "00000000-0000-4000-8000-000000000002";

/// The requests one tool call sent, by kind; `other` holds the rest, sorted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Sent {
    current_user: usize,
    tree: usize,
    trash: usize,
    channel: usize,
    socket: usize,
    document: usize,
    update: usize,
    storage: usize,
    other: Vec<String>,
}

fn tallied(requests: &[String], named: impl Fn(&str) -> String) -> Sent {
    requests.iter().fold(Sent::default(), |sent, request| {
        let counted = |count: usize| count + 1;
        match request.as_str() {
            "GET /gapi/current_user" => Sent {
                current_user: counted(sent.current_user),
                ..sent
            },
            "WS open" => Sent {
                socket: counted(sent.socket),
                ..sent
            },
            _ if request.starts_with("WS RequestDocument") => Sent {
                document: counted(sent.document),
                ..sent
            },
            _ if request.starts_with("WS Update") => Sent {
                update: counted(sent.update),
                ..sent
            },
            _ if request.starts_with("GET /storage/") => Sent {
                storage: counted(sent.storage),
                ..sent
            },
            _ if request.ends_with("/tree") => Sent {
                tree: counted(sent.tree),
                ..sent
            },
            _ if request.ends_with("/trash") => Sent {
                trash: counted(sent.trash),
                ..sent
            },
            _ if request
                .strip_prefix("GET /gapi/channel/")
                .is_some_and(|rest| !rest.contains('/')) =>
            {
                Sent {
                    channel: counted(sent.channel),
                    ..sent
                }
            }
            _ => Sent {
                other: sent
                    .other
                    .into_iter()
                    .chain([named(request)])
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                ..sent
            },
        }
    })
}

/// Two workspaces, a folder, the boards `Plans` (with a comment thread) and
/// `Ideas`.
async fn tool_world() -> holst_core::Result<holst_core::fake::Fake> {
    use holst_core::fake::{Fixture, FixtureObject, FixtureThread};

    let objects = vec![
        FixtureObject::Sticker("Roadmap".into()),
        FixtureObject::Sticker("Second".into()),
    ];
    let seed = Fixture::default()
        .workspace("Team")
        .workspace("Other")
        .folder("Archive", "Team")
        .board_with_objects("Plans", "Team", objects)
        .board("Ideas", "Other", None)
        .thread("Plans", FixtureThread::new("t1", 1, "Is this final?"));
    Ok(holst_core::fake::Fake::start(seed).await?)
}

/// A server on `fake` as after `login::finish`: the account and the workspaces
/// are known, no tree is kept.
async fn finished(fake: &holst_core::fake::Fake) -> holst_core::Result<McpServer> {
    use holst_core::command::login::{Stopped, finish};

    let (holst, login, _) = served(fake)?;
    let stored = Arc::clone(&login);
    finish(login.connect(fake_session())?, None, async |session| {
        stored.log_in(session);
        Ok::<(), std::convert::Infallible>(())
    })
    .await
    .map_err(|stopped| match stopped {
        Stopped::Check(error) => error,
        Stopped::Save(never) => match never {},
    })?;
    Ok(holst)
}

/// The call with `<Plans.1>` as the id of that object and `<@Plans>` as the
/// `@N` the server gave the board.
fn tool_call(
    fake: &holst_core::fake::Fake,
    holst: &McpServer,
    arguments: &serde_json::Value,
) -> serde_json::Value {
    let board = holst.state.alias(&fake.id_of("Plans").unwrap_or_default());
    let text = arguments
        .to_string()
        .replace("<@Plans>", &board)
        .replace("<Plans.1>", &fake.object_id("Plans", 1).unwrap_or_default());
    serde_json::from_str(&text).unwrap_or_default()
}

/// What `call` sent to the fake.
async fn tool_sent(
    fake: &holst_core::fake::Fake,
    holst: &McpServer,
    tool: &str,
    arguments: &serde_json::Value,
) -> Sent {
    let before = fake.requests().len();
    let call = tool_call(fake, holst, arguments);
    let (failed, text) = table_call(holst, tool, call).await;
    assert!(!failed, "{text}");
    let names = [
        ("<Plans>", fake.id_of("Plans").unwrap_or_default()),
        ("<Ideas>", fake.id_of("Ideas").unwrap_or_default()),
        ("<Team>", TEAM.to_owned()),
        ("<Other>", OTHER.to_owned()),
    ];
    let sent = fake.requests().split_off(before);
    tallied(&sent, |request| {
        names.iter().fold(request.to_owned(), |named, (name, id)| {
            named.replace(id, name)
        })
    })
}

/// What fills the cache before the warm call.
#[derive(Debug, Clone, Copy)]
enum WarmUp {
    /// The call itself, once; it can be repeated.
    Again,
    /// A `board ls`; the call changes what a second one would find.
    Listed,
}

/// Pins the requests of a tool call cold, then warm on another server.
async fn tool_pins(
    tool: &str,
    arguments: serde_json::Value,
    warm_up: WarmUp,
    cold: Sent,
    warm: Sent,
) -> holst_core::Result<()> {
    let cold_fake = tool_world().await?;
    let cold_holst = finished(&cold_fake).await?;
    let found_cold = tool_sent(&cold_fake, &cold_holst, tool, &arguments).await;
    let fake = tool_world().await?;
    let holst = finished(&fake).await?;
    let (first_tool, first) = match warm_up {
        WarmUp::Again => (tool, arguments.clone()),
        WarmUp::Listed => ("board", serde_json::json!({ "action": "ls" })),
    };
    tool_sent(&fake, &holst, first_tool, &first).await;
    let found_warm = tool_sent(&fake, &holst, tool, &arguments).await;
    assert_eq!(found_cold, cold, "cold");
    assert_eq!(found_warm, warm, "warm");
    Ok(())
}

#[rstest::rstest]
#[case::show_by_name(serde_json::json!({ "action": "show", "board": "Plans" }), WarmUp::Again, Sent { tree: 2, channel: 1, socket: 1, document: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() })]
#[case::show_by_alias(serde_json::json!({ "action": "show", "board": "<@Plans>" }), WarmUp::Again, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() })]
#[case::ls(serde_json::json!({ "action": "ls" }), WarmUp::Again, Sent { tree: 2, ..Sent::default() }, Sent { tree: 2, ..Sent::default() })]
#[case::snap(serde_json::json!({ "action": "snap", "board": "Plans", "picture_format": "svg" }), WarmUp::Again, Sent { tree: 2, channel: 1, socket: 1, document: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() })]
#[tokio::test]
async fn the_board_tool_sends(
    #[case] arguments: serde_json::Value,
    #[case] warm_up: WarmUp,
    #[case] cold: Sent,
    #[case] warm: Sent,
) -> holst_core::Result<()> {
    tool_pins("board", arguments, warm_up, cold, warm).await
}

#[rstest::rstest]
#[case::by_name(serde_json::json!({ "board": "Plans" }), WarmUp::Again, Sent { tree: 2, channel: 1, socket: 1, document: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() })]
#[case::by_alias(serde_json::json!({ "board": "<@Plans>" }), WarmUp::Again, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, ..Sent::default() })]
#[tokio::test]
async fn the_object_tool_sends(
    #[case] arguments: serde_json::Value,
    #[case] warm_up: WarmUp,
    #[case] cold: Sent,
    #[case] warm: Sent,
) -> holst_core::Result<()> {
    tool_pins("object", arguments, warm_up, cold, warm).await
}

#[rstest::rstest]
#[case::add_by_name(serde_json::json!({ "action": "add", "board": "Plans", "text": "note" }), WarmUp::Again, Sent { tree: 2, channel: 1, socket: 2, document: 2, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 2, document: 2, update: 1, ..Sent::default() })]
#[case::add_with_confirm(serde_json::json!({ "action": "add", "board": "Plans", "text": "note", "confirm": true }), WarmUp::Again, Sent { tree: 2, channel: 1, socket: 2, document: 2, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 2, document: 2, update: 1, ..Sent::default() })]
#[case::add_by_alias(serde_json::json!({ "action": "add", "board": "<@Plans>", "text": "note" }), WarmUp::Again, Sent { channel: 1, socket: 2, document: 2, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 2, document: 2, update: 1, ..Sent::default() })]
#[case::edit(serde_json::json!({ "action": "edit", "board": "Plans", "object": ["<Plans.1>"], "text": "changed", "confirm": true }), WarmUp::Listed, Sent { tree: 2, channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() })]
#[case::rm(serde_json::json!({ "action": "rm", "board": "Plans", "object": ["<Plans.1>"], "confirm": true }), WarmUp::Listed, Sent { tree: 2, channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() })]
#[case::lock(serde_json::json!({ "action": "lock", "board": "Plans", "object": ["<Plans.1>"] }), WarmUp::Listed, Sent { tree: 2, channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() })]
#[case::lock_with_confirm(serde_json::json!({ "action": "lock", "board": "Plans", "object": ["<Plans.1>"], "confirm": true }), WarmUp::Listed, Sent { tree: 2, channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() }, Sent { channel: 1, socket: 1, document: 1, update: 1, ..Sent::default() })]
#[tokio::test]
async fn the_object_write_tool_sends(
    #[case] arguments: serde_json::Value,
    #[case] warm_up: WarmUp,
    #[case] cold: Sent,
    #[case] warm: Sent,
) -> holst_core::Result<()> {
    tool_pins("object_write", arguments, warm_up, cold, warm).await
}

#[rstest::rstest]
#[case::ls(serde_json::json!({ "action": "ls", "board": "Plans" }), WarmUp::Again, Sent { tree: 2, socket: 1, document: 1, other: vec!["GET /gapi/comment/thread_list/<Plans>".into()], ..Sent::default() }, Sent { socket: 1, document: 1, other: vec!["GET /gapi/comment/thread_list/<Plans>".into()], ..Sent::default() })]
#[case::add(serde_json::json!({ "action": "add", "board": "Plans", "text": "Yes" }), WarmUp::Again, Sent { tree: 2, socket: 1, document: 1, update: 1, ..Sent::default() }, Sent { socket: 1, document: 1, update: 1, ..Sent::default() })]
#[case::reply(serde_json::json!({ "action": "reply", "board": "Plans", "thread": "t1", "text": "Yes" }), WarmUp::Again, Sent { tree: 2, socket: 1, document: 1, update: 1, ..Sent::default() }, Sent { socket: 1, document: 1, update: 1, ..Sent::default() })]
#[tokio::test]
async fn the_comment_tool_sends(
    #[case] arguments: serde_json::Value,
    #[case] warm_up: WarmUp,
    #[case] cold: Sent,
    #[case] warm: Sent,
) -> holst_core::Result<()> {
    tool_pins("comment", arguments, warm_up, cold, warm).await
}

#[rstest::rstest]
#[case::names(serde_json::json!({ "query": "plans", "mode": "names" }), WarmUp::Again, Sent { tree: 2, ..Sent::default() }, Sent::default())]
#[case::board_text(serde_json::json!({ "query": "roadmap", "mode": "board_text" }), WarmUp::Again, Sent { tree: 2, channel: 2, socket: 2, document: 2, ..Sent::default() }, Sent::default())]
#[tokio::test]
async fn the_search_tool_sends(
    #[case] arguments: serde_json::Value,
    #[case] warm_up: WarmUp,
    #[case] cold: Sent,
    #[case] warm: Sent,
) -> holst_core::Result<()> {
    tool_pins("search", arguments, warm_up, cold, warm).await
}

#[rstest::rstest]
#[case::ls(serde_json::json!({ "action": "ls" }), WarmUp::Again, Sent { current_user: 1, ..Sent::default() }, Sent { current_user: 1, ..Sent::default() })]
#[case::show(serde_json::json!({ "action": "show", "workspace": "Team" }), WarmUp::Again, Sent { other: vec!["GET /gapi/workspace/<Team>".into()], ..Sent::default() }, Sent { other: vec!["GET /gapi/workspace/<Team>".into()], ..Sent::default() })]
#[case::rename(serde_json::json!({ "action": "rename", "workspace": "Other", "name": "Crew", "confirm": true }), WarmUp::Listed, Sent { other: vec!["PATCH /gapi/workspace/<Other>".into()], ..Sent::default() }, Sent { other: vec!["PATCH /gapi/workspace/<Other>".into()], ..Sent::default() })]
#[tokio::test]
async fn the_workspace_tool_sends(
    #[case] arguments: serde_json::Value,
    #[case] warm_up: WarmUp,
    #[case] cold: Sent,
    #[case] warm: Sent,
) -> holst_core::Result<()> {
    tool_pins("workspace", arguments, warm_up, cold, warm).await
}
