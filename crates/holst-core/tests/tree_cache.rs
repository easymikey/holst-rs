//! Name lookups read trees through the account cache.
#![expect(
    clippy::disallowed_methods,
    reason = "tests build and remove real files in temp directories"
)]

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use holst_core::{
    Error,
    command::{
        board::{show::show, tree_change},
        board_load::guarded_loss,
        login::{connect, finish},
        lookup::{self, CachedNode, Fresh, IndexedNodes},
        search::search,
        table::{self, CommandArgs},
        workspace,
    },
    domain::{account::Workspace, session::Session},
    effect::{
        cache::{AccountCache, CachedTree, Forget},
        clock::FixedClock,
        settings::CacheSettings,
    },
    error::{
        board_socket::{BoardSocketError, SocketErrorCode},
        lookup::{LookupError, LookupMiss},
    },
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture},
    remote::client::Client,
    storage::cache::account::AccountDisk,
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;
use serde_json::{Value, json};

const FAKE_USER: &str = "00000000-0000-4000-8000-000000000000";
const HOUR: u64 = 3_600_000;
const START: u64 = 10 * HOUR;

/// An account cache in a temporary folder, removed with its last holder.
struct Scratch {
    disk: AccountDisk,
    dir: PathBuf,
}

impl Scratch {
    fn new() -> holst_core::Result<Arc<Self>> {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "holst-tree-cache-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let settings = CacheSettings {
            root: Some(dir.clone()),
            roots: vec![dir.clone()],
            host: "app.holst.so".into(),
            ..CacheSettings::default()
        };
        let disk = AccountDisk::from_settings(&settings).ok_or_else(|| {
            Error::LocalData(holst_core::error::local_data::LocalDataError::NoHome)
        })?;
        Ok(Arc::new(Self { disk, dir }))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.dir));
    }
}

impl AccountCache for Scratch {
    fn workspaces(&self, account: &str) -> Option<Vec<Workspace>> {
        self.disk.workspaces(account)
    }

    fn keep_workspaces(
        &self,
        account: &str,
        workspaces: &[Workspace],
    ) -> holst_core::Result<()> {
        self.disk.keep_workspaces(account, workspaces)
    }

    fn tree(&self, account: &str, workspace: &str) -> Option<CachedTree> {
        self.disk.tree(account, workspace)
    }

    fn keep_tree(
        &self,
        account: &str,
        workspace: &str,
        text: &str,
        read_at: u64,
    ) -> holst_core::Result<()> {
        self.disk.keep_tree(account, workspace, text, read_at)
    }

    fn forget(
        &self,
        account: &str,
        what: Forget<'_>,
        at: u64,
    ) -> holst_core::Result<()> {
        self.disk.forget(account, what, at)
    }
}

fn saved_login() -> Session {
    Session::with_cookies(
        SecretString::from(ACCEPTED_TOKEN),
        ACCEPTED_COOKIES.parse().unwrap_or_default(),
    )
    .with_account_id(FAKE_USER)
}

fn bare(fake: &Fake) -> holst_core::Result<Client> {
    connect(
        Some(saved_login()),
        fake.base_url().parse()?,
        SystemClock,
        RandomIds,
    )
}

/// A client on `cache` whose clock stands at `now`.
fn at(fake: &Fake, cache: &Arc<Scratch>, now: u64) -> holst_core::Result<Client> {
    Ok(connect(
        Some(saved_login()),
        fake.base_url().parse()?,
        FixedClock { unix_millis: now },
        RandomIds,
    )?
    .with_account_cache(Arc::<Scratch>::clone(cache)))
}

fn plans() -> Fixture {
    Fixture::default()
        .workspace("Team")
        .workspace("Other")
        .board("Plans", "Team", None)
}

fn single() -> Fixture {
    Fixture::default()
        .workspace("Team")
        .board("Plans", "Team", None)
}

fn args<A: CommandArgs>(arguments: &Value) -> holst_core::Result<A> {
    table::parse(arguments.as_object().cloned().unwrap_or_default())
}

/// The tree requests the fake got after the log was `before` long.
fn trees_since(fake: &Fake, before: usize) -> usize {
    fake.requests()
        .split_off(before)
        .iter()
        .filter(|request| request.ends_with("/tree"))
        .count()
}

async fn show_plans(client: &Client, refresh: bool) -> holst_core::Result<()> {
    show(
        client,
        &CacheSettings::default(),
        &args(&json!({ "board": "Plans", "refresh": refresh }))?,
    )
    .await
    .map(drop)
}

#[tokio::test]
async fn the_second_show_by_name_reads_no_tree() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    show_plans(&client, false).await?;
    let before = fake.requests().len();
    show_plans(&client, false).await?;
    assert_eq!(trees_since(&fake, before), 0);
    Ok(())
}

#[tokio::test]
async fn a_second_client_on_the_same_cache_reads_no_tree() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let cache = Scratch::new()?;
    show_plans(&at(&fake, &cache, START)?, false).await?;
    let before = fake.requests().len();
    show_plans(&at(&fake, &cache, START)?, false).await?;
    assert_eq!(trees_since(&fake, before), 0);
    Ok(())
}

#[tokio::test]
async fn a_renamed_board_keeps_its_old_name_until_the_new_one_misses()
-> holst_core::Result<()> {
    let fake = Fake::start(single()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    let id = lookup::board_id(&client, "Plans", None).await?;
    tree_change::rename(
        &bare(&fake)?,
        &args(&json!({ "board": "Plans", "name": "Roadmap" }))?,
    )
    .await?;
    let before = fake.requests().len();
    assert_eq!(lookup::board_id(&client, "Plans", None).await?, id);
    assert_eq!(trees_since(&fake, before), 0);
    assert_eq!(lookup::board_id(&client, "Roadmap", None).await?, id);
    assert_eq!(trees_since(&fake, before), 1);
    Ok(())
}

#[tokio::test]
async fn refresh_asks_the_trees() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    show_plans(&client, false).await?;
    let before = fake.requests().len();
    show_plans(&client, true).await?;
    assert_eq!(trees_since(&fake, before), 2);
    Ok(())
}

/// The kept trees are `age` old for a client that runs a change.
async fn change_by_name(
    age: u64,
    other: impl AsyncFnOnce(&Client) -> holst_core::Result<()>,
) -> holst_core::Result<(Fake, holst_core::Result<String>, usize)> {
    let fake = Fake::start(plans()).await?;
    let cache = Scratch::new()?;
    lookup::board_id(&at(&fake, &cache, START)?, "Plans", None).await?;
    other(&bare(&fake)?).await?;
    let before = fake.requests().len();
    let writer = at(&fake, &cache, START + age)?.for_change();
    let found = lookup::board_id(&writer, "Plans", None).await;
    let asked = trees_since(&fake, before);
    Ok((fake, found, asked))
}

#[tokio::test]
async fn a_change_by_name_asks_a_tree_older_than_an_hour_once() -> holst_core::Result<()>
{
    let (fake, found, asked) = change_by_name(2 * HOUR, async |_| Ok(())).await?;
    assert_eq!(found?, fake.id_of("Plans").unwrap_or_default());
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_change_by_name_refuses_a_name_made_ambiguous_meanwhile()
-> holst_core::Result<()> {
    let (_, found, asked) = change_by_name(2 * HOUR, async |other| {
        tree_change::create(
            other,
            &args(&json!({ "name": "Plans", "workspace": "Team" }))?,
        )
        .await
        .map(drop)
    })
    .await?;
    assert!(
        matches!(
            found,
            Err(Error::Lookup(LookupError {
                miss: LookupMiss::Ambiguous { .. },
                ..
            }))
        ),
        "{found:?}"
    );
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_change_by_name_with_a_young_tree_asks_none() -> holst_core::Result<()> {
    let (_, found, asked) = change_by_name(HOUR / 6, async |_| Ok(())).await?;
    assert!(found.is_ok(), "{found:?}");
    assert_eq!(asked, 0);
    Ok(())
}

#[tokio::test]
async fn an_id_or_a_link_asks_no_tree() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    let id = fake.id_of("Plans").unwrap_or_default();
    lookup::board_id(&client, &id, None).await?;
    lookup::board_id(&client, &client.board_link(&id), None).await?;
    assert!(fake.requests().is_empty(), "{:?}", fake.requests());
    Ok(())
}

#[tokio::test]
async fn a_refused_tree_is_dropped_from_the_cache() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let cache = Scratch::new()?;
    let client = at(&fake, &cache, START)?;
    lookup::board_id(&client, "Plans", None).await?;
    let team = lookup::workspace(&client, Some("Team")).await?;
    let other = lookup::workspace(&client, Some("Other")).await?;
    fake.fail(&format!("workspace/{}/tree", team.id), 403);
    let read = lookup::indexes(
        &client,
        vec![team.clone()],
        IndexedNodes::Live,
        Fresh::Asked,
    )
    .await;
    assert!(read.iter().all(|(_, tree)| tree.is_err()));
    assert!(cache.tree(FAKE_USER, &team.id).is_none());
    assert!(cache.tree(FAKE_USER, &other.id).is_some());
    Ok(())
}

#[tokio::test]
async fn a_cached_board_gives_its_workspace_and_folder_with_no_request()
-> holst_core::Result<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .folder("Docs", "Team")
            .board("Plans", "Team", Some("Docs")),
    )
    .await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    let id = lookup::board_id(&client, "Plans", None).await?;
    let before = fake.requests().len();
    let node = lookup::cached_node(&client, &id);
    assert_eq!(fake.requests().len(), before);
    let docs = fake.id_of("Docs").unwrap_or_default();
    assert_eq!(
        node,
        Some(CachedNode {
            workspace: lookup::workspace(&client, Some("Team")).await?,
            name: "Plans".into(),
            folder: Some((docs, "Docs".into())),
        })
    );
    Ok(())
}

#[tokio::test]
async fn forgetting_trees_drops_one_or_all_and_keeps_the_list() -> holst_core::Result<()>
{
    let fake = Fake::start(plans()).await?;
    let cache = Scratch::new()?;
    let client = at(&fake, &cache, START)?;
    lookup::board_id(&client, "Plans", None).await?;
    let team = lookup::workspace(&client, Some("Team")).await?;
    let other = lookup::workspace(&client, Some("Other")).await?;
    lookup::forget_trees(&client, Some(&team.id));
    assert!(cache.tree(FAKE_USER, &team.id).is_none());
    assert!(cache.tree(FAKE_USER, &other.id).is_some());
    lookup::forget_trees(&client, None);
    assert!(cache.tree(FAKE_USER, &other.id).is_none());
    assert!(cache.workspaces(FAKE_USER).is_some());
    Ok(())
}

#[tokio::test]
async fn forgetting_the_trees_holding_an_id_leaves_the_others() -> holst_core::Result<()>
{
    let fake = Fake::start(plans()).await?;
    let cache = Scratch::new()?;
    let client = at(&fake, &cache, START)?;
    let id = lookup::board_id(&client, "Plans", None).await?;
    let team = lookup::workspace(&client, Some("Team")).await?;
    let other = lookup::workspace(&client, Some("Other")).await?;
    lookup::forget_trees_holding(&client, &id);
    assert!(cache.tree(FAKE_USER, &team.id).is_none());
    assert!(cache.tree(FAKE_USER, &other.id).is_some());
    Ok(())
}

fn names(extra: &Value) -> holst_core::Result<holst_core::command::search::SearchArgs> {
    let mut given = json!({ "query": "plans", "mode": "names" });
    given
        .as_object_mut()
        .into_iter()
        .for_each(|map| map.extend(extra.as_object().cloned().unwrap_or_default()));
    args(&given)
}

#[tokio::test]
async fn search_in_a_workspace_made_elsewhere_finds_it() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    lookup::board_id(&client, "Plans", None).await?;
    workspace::create(&bare(&fake)?, &args(&json!({ "name": "Studio" }))?).await?;
    let found = search(
        &client,
        &CacheSettings::default(),
        &names(&json!({ "workspace": "Studio" }))?,
    )
    .await;
    assert!(found.is_ok(), "{found:?}");
    Ok(())
}

#[tokio::test]
async fn search_reads_the_kept_trees_and_refresh_asks_them() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    let cache = CacheSettings::default();
    search(&client, &cache, &names(&json!({}))?).await?;
    let before = fake.requests().len();
    search(&client, &cache, &names(&json!({}))?).await?;
    assert_eq!(trees_since(&fake, before), 0);
    search(&client, &cache, &names(&json!({ "refresh": true }))?).await?;
    assert_eq!(trees_since(&fake, before), 2);
    Ok(())
}

#[tokio::test]
async fn a_new_login_forgets_the_trees_and_keeps_the_list() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    lookup::board_id(&client, "Plans", None).await?;
    assert!(client.known_workspaces().is_some());
    let done = finish(client.clone(), None, async |_| Ok::<_, Error>(())).await;
    assert!(done.is_ok());
    assert!(client.known_workspaces().is_some());
    let team = lookup::workspace(&client, Some("Team")).await?;
    assert!(client.known_tree(&team.id).is_none());
    Ok(())
}

#[tokio::test]
async fn a_lost_board_drops_the_trees_holding_it() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    let id = lookup::board_id(&client, "Plans", None).await?;
    let team = lookup::workspace(&client, Some("Team")).await?;
    let other = lookup::workspace(&client, Some("Other")).await?;
    let lost = Err::<(), _>(Error::BoardSocket(BoardSocketError::NotSubscribed {
        code: SocketErrorCode::NotFound,
    }));
    let (_, closed) = guarded_loss(&client, None, &id, lost).await;
    assert!(closed);
    assert!(client.known_tree(&team.id).is_none());
    assert!(client.known_tree(&other.id).is_some());
    Ok(())
}

/// The cache holds the tree of `START`; `made` is made elsewhere, `change`
/// runs half an hour later on it. Gives its result and its tree requests.
async fn change_after_web_edit(
    made: Value,
    change: impl AsyncFnOnce(&Fake, &Client) -> holst_core::Result<()>,
) -> holst_core::Result<(holst_core::Result<()>, usize)> {
    let fake = Fake::start(plans().folder("Archive", "Team")).await?;
    let cache = Scratch::new()?;
    lookup::board_id(&at(&fake, &cache, START)?, "Plans", None).await?;
    tree_change::create(&bare(&fake)?, &args(&made)?).await?;
    let before = fake.requests().len();
    let writer = at(&fake, &cache, START + HOUR / 2)?.for_change();
    let done = change(&fake, &writer).await;
    Ok((done, trees_since(&fake, before)))
}

#[tokio::test]
async fn a_board_made_in_the_web_is_moved_after_one_tree_read() -> holst_core::Result<()>
{
    let made = json!({ "name": "Q4", "workspace": "Team" });
    let (done, asked) = change_after_web_edit(made, async |fake, writer| {
        let given = json!({ "board": fake.id_of("Q4"), "folder": "Archive" });
        tree_change::mv(writer, &args(&given)?).await.map(drop)
    })
    .await?;
    assert!(done.is_ok(), "{done:?}");
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_board_made_in_the_web_is_trashed_after_one_tree_read()
-> holst_core::Result<()> {
    let made = json!({ "name": "Q4", "workspace": "Team" });
    let (done, asked) = change_after_web_edit(made, async |fake, writer| {
        let given = json!({ "board": fake.id_of("Q4") });
        tree_change::rm(writer, &args(&given)?).await.map(drop)
    })
    .await?;
    assert!(done.is_ok(), "{done:?}");
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_folder_made_in_the_web_takes_a_new_board_after_one_tree_read()
-> holst_core::Result<()> {
    let made = json!({ "name": "Fresh", "workspace": "Team", "kind": "folder" });
    let (done, asked) = change_after_web_edit(made, async |_, writer| {
        let given = json!({ "name": "Notes", "workspace": "Team", "folder": "Fresh" });
        tree_change::create(writer, &args(&given)?).await.map(drop)
    })
    .await?;
    assert!(done.is_ok(), "{done:?}");
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_folder_made_in_the_web_takes_a_moved_board_after_one_tree_read()
-> holst_core::Result<()> {
    let made = json!({ "name": "Fresh", "workspace": "Team", "kind": "folder" });
    let (done, asked) = change_after_web_edit(made, async |_, writer| {
        let given = json!({ "board": "Plans", "folder": "Fresh" });
        tree_change::mv(writer, &args(&given)?).await.map(drop)
    })
    .await?;
    assert!(done.is_ok(), "{done:?}");
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_folder_that_is_nowhere_costs_one_tree_read_before_not_found()
-> holst_core::Result<()> {
    let (done, asked) = change_after_web_edit(
        json!({ "name": "Q4", "workspace": "Team" }),
        async |_, writer| {
            let given =
                json!({ "name": "Notes", "workspace": "Team", "folder": "Nowhere" });
            tree_change::create(writer, &args(&given)?).await.map(drop)
        },
    )
    .await?;
    assert!(matches!(done, Err(Error::Lookup(_))), "{done:?}");
    assert_eq!(asked, 1);
    Ok(())
}

#[tokio::test]
async fn a_tree_asked_before_a_forget_is_not_kept_after_it() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = at(&fake, &Scratch::new()?, START)?;
    let team = lookup::workspace(&client, Some("Team")).await?;
    lookup::forget_trees(&client, Some(&team.id));
    client.learn_tree(&team.id, "{\"channels\":[]}", START - 1);
    assert!(client.known_tree(&team.id).is_none());
    client.learn_tree(&team.id, "{\"channels\":[]}", START);
    assert!(client.known_tree(&team.id).is_some());
    Ok(())
}
