//! How many requests each command sends, counted on the fake's log.
#![expect(
    clippy::disallowed_methods,
    reason = "tests build and remove real files in temp directories"
)]

use std::{
    convert::Infallible,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use holst_core::{
    command::{
        auth,
        auth::AuthVerb,
        backup,
        backup::BackupVerb,
        board::{
            BoardVerb,
            file,
            own,
            picture,
            rights,
            settings,
            show::show,
            tag,
            tree,
            tree_change,
        },
        comment::{self, CommentVerb},
        login::{Stopped, connect, email::EmailLogin, finish, logout},
        lookup,
        object::{ObjectVerb, add, align, change, copy, get, group, list, lock},
        search::search,
        table::{
            self,
            CommandArgs,
            CommandName::{
                self,
                Auth,
                Backup,
                Board,
                Comment,
                Object,
                Search,
                Storage,
                Template,
                Workspace,
            },
            CommandRisk,
        },
        template::{self, TemplateVerb},
        workspace::{self, WorkspaceVerb},
    },
    domain::{account::Workspace as Known, session::Session},
    effect::{
        cache::{AccountCache, CachedTree, Forget},
        clock::FixedClock,
        settings::CacheSettings,
    },
    error::board_socket::BoardSocketError,
    fake::{
        ACCEPTED_COOKIES,
        ACCEPTED_TOKEN,
        Fake,
        Fixture,
        FixtureObject,
        FixtureThread,
        LOGIN_CODE,
        WorkspaceRole,
    },
    remote::client::Client,
    storage::cache::account::AccountDisk,
    system::{clock::SystemClock, id::RandomIds},
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

const CURRENT_USER: &str = "GET /gapi/current_user";
const FAKE_USER: &str = "00000000-0000-4000-8000-000000000000";

/// An account cache in a temporary folder, removed with its last holder.
struct Scratch {
    disk: AccountDisk,
    dir: PathBuf,
}

impl Scratch {
    fn new() -> holst_core::Result<Arc<Self>> {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "holst-requests-{}-{}",
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
            holst_core::Error::LocalData(
                holst_core::error::local_data::LocalDataError::NoHome,
            )
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
    fn workspaces(&self, account: &str) -> Option<Vec<Known>> {
        self.disk.workspaces(account)
    }

    fn keep_workspaces(
        &self,
        account: &str,
        workspaces: &[Known],
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

fn on_cache(fake: &Fake, cache: &Arc<Scratch>) -> holst_core::Result<Client> {
    Ok(bare(fake)?.with_account_cache(Arc::<Scratch>::clone(cache)))
}

/// A fake and a client with a saved login (account id) and a temp cache.
async fn started(fixture: Fixture) -> holst_core::Result<(Fake, Client)> {
    let fake = Fake::start(fixture).await?;
    let client = on_cache(&fake, &Scratch::new()?)?;
    Ok((fake, client))
}

/// The requests the fake got after the log was `before` long.
fn since(fake: &Fake, before: usize) -> Vec<String> {
    fake.requests().split_off(before)
}

fn asked(requests: &[String]) -> usize {
    requests
        .iter()
        .filter(|request| *request == CURRENT_USER)
        .count()
}

fn plans() -> Fixture {
    Fixture::default()
        .workspace("Team")
        .workspace("Other")
        .board("Plans", "Team", None)
}

fn args<A: CommandArgs>(arguments: &Value) -> holst_core::Result<A> {
    table::parse(arguments.as_object().cloned().unwrap_or_default())
}

#[derive(Debug, Clone, Copy)]
enum Run {
    BoardLs,
    BoardShow,
    ObjectLs,
    SearchNames,
    CommentLs,
    WorkspaceShow,
}

async fn run(client: &Client, command: Run) -> holst_core::Result<()> {
    let cache = CacheSettings::default();
    match command {
        Run::BoardLs => tree::ls(client, &args(&json!({}))?).await.map(drop),
        Run::BoardShow => show(client, &cache, &args(&json!({ "board": "Plans" }))?)
            .await
            .map(drop),
        Run::ObjectLs => list::ls(client, &cache, &args(&json!({ "board": "Plans" }))?)
            .await
            .map(drop),
        Run::SearchNames => search(
            client,
            &cache,
            &args(&json!({ "query": "plans", "mode": "names" }))?,
        )
        .await
        .map(drop),
        Run::CommentLs => comment::ls(client, &args(&json!({ "board": "Plans" }))?)
            .await
            .map(drop),
        Run::WorkspaceShow => {
            workspace::show(client, &args(&json!({ "workspace": "Team" }))?)
                .await
                .map(drop)
        }
    }
}

#[rstest]
#[case::board_ls(Run::BoardLs)]
#[case::board_show(Run::BoardShow)]
#[case::object_ls(Run::ObjectLs)]
#[case::search(Run::SearchNames)]
#[case::comment_ls(Run::CommentLs)]
#[case::ws_show(Run::WorkspaceShow)]
#[tokio::test]
async fn the_second_run_does_not_ask_who_you_are(
    #[case] command: Run,
) -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, command).await?;
    let first = since(&fake, 0);
    run(&client, command).await?;
    let second = since(&fake, first.len());
    assert_eq!(asked(&first), 1, "{first:?}");
    assert_eq!(asked(&second), 0, "{second:?}");
    assert!(second.len() < first.len(), "{first:?} then {second:?}");
    Ok(())
}

#[tokio::test]
async fn a_second_client_on_the_same_cache_does_not_ask() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let cache = Scratch::new()?;
    run(&on_cache(&fake, &cache)?, Run::BoardLs).await?;
    let before = fake.requests().len();
    run(&on_cache(&fake, &cache)?, Run::BoardLs).await?;
    assert_eq!(asked(&since(&fake, before)), 0);
    Ok(())
}

#[tokio::test]
async fn workspace_list_asks_each_time() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    let listed = args(&json!({}))?;
    workspace::list(&client, &listed).await?;
    workspace::list(&client, &listed).await?;
    assert_eq!(asked(&fake.requests()), 2);
    Ok(())
}

#[tokio::test]
async fn workspace_list_keeps_what_it_read() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    workspace::list(&client, &args(&json!({}))?).await?;
    let before = fake.requests().len();
    run(&client, Run::BoardLs).await?;
    assert_eq!(asked(&since(&fake, before)), 0);
    Ok(())
}

#[rstest]
#[case::board_ls(Run::BoardLs)]
#[case::board_show(Run::BoardShow)]
#[case::object_ls(Run::ObjectLs)]
#[case::search(Run::SearchNames)]
#[case::comment_ls(Run::CommentLs)]
#[case::ws_show(Run::WorkspaceShow)]
#[tokio::test]
async fn with_no_account_cache_every_run_asks(
    #[case] command: Run,
) -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = bare(&fake)?;
    run(&client, command).await?;
    run(&client, command).await?;
    assert_eq!(asked(&fake.requests()), 2);
    Ok(())
}

#[tokio::test]
async fn a_workspace_made_elsewhere_is_found_by_name() -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = on_cache(&fake, &Scratch::new()?)?;
    run(&client, Run::BoardLs).await?;
    workspace::create(&bare(&fake)?, &args(&json!({ "name": "Studio" }))?).await?;
    let before = fake.requests().len();
    let found =
        workspace::show(&client, &args(&json!({ "workspace": "Studio" }))?).await?;
    assert_eq!(found.name, "Studio");
    assert_eq!(asked(&since(&fake, before)), 1);
    Ok(())
}

#[tokio::test]
async fn a_board_in_a_workspace_made_elsewhere_is_found_by_name()
-> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = on_cache(&fake, &Scratch::new()?)?;
    run(&client, Run::BoardLs).await?;
    let elsewhere = bare(&fake)?;
    workspace::create(&elsewhere, &args(&json!({ "name": "Studio" }))?).await?;
    tree_change::create(
        &elsewhere,
        &args(&json!({ "name": "Nova", "workspace": "Studio" }))?,
    )
    .await?;
    let before = fake.requests().len();
    let found = lookup::board_id(&client, "Nova", None).await?;
    assert_eq!(Some(found), fake.id_of("Nova"));
    assert_eq!(asked(&since(&fake, before)), 1);
    Ok(())
}

#[tokio::test]
async fn a_name_nobody_has_asks_once_and_says_so() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    let before = fake.requests().len();
    let missing = lookup::board_id(&client, "Nothing", None).await;
    assert!(missing.is_err(), "{missing:?}");
    assert_eq!(asked(&since(&fake, before)), 1);
    Ok(())
}

#[tokio::test]
async fn after_a_workspace_is_created_the_next_lookup_asks_once()
-> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    workspace::create(&client, &args(&json!({ "name": "Studio" }))?).await?;
    let before = fake.requests().len();
    run(&client, Run::BoardLs).await?;
    run(&client, Run::BoardLs).await?;
    assert_eq!(asked(&since(&fake, before)), 1);
    Ok(())
}

#[tokio::test]
async fn a_dry_run_create_keeps_the_list() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    let created = workspace::create(
        &client,
        &args(&json!({ "name": "Studio", "dry_run": true }))?,
    )
    .await?;
    assert!(created.id.is_none());
    let before = fake.requests().len();
    run(&client, Run::BoardLs).await?;
    assert_eq!(asked(&since(&fake, before)), 0);
    Ok(())
}

#[tokio::test]
async fn after_a_workspace_is_renamed_the_next_lookup_asks_once()
-> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    workspace::rename(
        &client,
        &args(&json!({ "workspace": "Other", "name": "Crew", "confirm": true }))?,
    )
    .await?;
    let before = fake.requests().len();
    run(&client, Run::BoardLs).await?;
    run(&client, Run::BoardLs).await?;
    assert_eq!(asked(&since(&fake, before)), 1);
    Ok(())
}

#[tokio::test]
async fn a_refused_tree_forgets_the_list() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    let other = lookup::workspace(&client, Some("Other")).await?.id;
    fake.fail(&format!("workspace/{other}/tree"), 403);
    drop(tree::ls(&client, &args(&json!({}))?).await);
    lookup::board_id(&client, "Plans", None).await?;
    let before = fake.requests().len();
    lookup::board_id(&client, "Plans", None).await?;
    assert_eq!(asked(&since(&fake, before)), 1);
    Ok(())
}

async fn exact_target_asks(
    reference_of: fn(&Fake) -> String,
) -> holst_core::Result<()> {
    let fake = Fake::start(plans()).await?;
    let client = bare(&fake)?;
    let target = lookup::exact_board(&client, &reference_of(&fake), None).await?;
    assert_eq!(target.workspace.name, "Team");
    assert!(asked(&fake.requests()) <= 1, "{:?}", fake.requests());
    Ok(())
}

#[tokio::test]
async fn an_exact_board_by_name_asks_at_most_once() -> holst_core::Result<()> {
    exact_target_asks(|_| "Plans".into()).await
}

#[tokio::test]
async fn an_exact_board_by_id_asks_at_most_once() -> holst_core::Result<()> {
    exact_target_asks(|fake| fake.id_of("Plans").unwrap_or_default()).await
}

fn whole_boards() -> Fixture {
    let roadmap = || vec![FixtureObject::FreeText("Roadmap".into())];
    plans()
        .board_with_objects("Roadmap", "Team", roadmap())
        .board("Empty", "Team", None)
        .board_stored_as("Newer", "Team", 3, "hud")
        .backup_of_objects("Roadmap", "snap-1", roadmap())
}

fn channel_reads(requests: &[String], board: Option<&str>) -> usize {
    let wanted = format!("GET /gapi/channel/{}", board.unwrap_or_default());
    requests.iter().filter(|each| **each == wanted).count()
}

/// A board file of Roadmap in a scratch folder, and the folder.
async fn roadmap_file(client: &Client) -> holst_core::Result<(Arc<Scratch>, String)> {
    let scratch = Scratch::new()?;
    fs::create_dir_all(&scratch.dir)?;
    let path = scratch
        .dir
        .join("roadmap.ydoc")
        .to_string_lossy()
        .into_owned();
    let saved = json!({ "board": "Roadmap", "file": path });
    file::export(client, &CacheSettings::default(), None, &args(&saved)?).await?;
    Ok((scratch, path))
}

#[tokio::test]
async fn import_into_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client) = started(whole_boards()).await?;
    let (_scratch, path) = roadmap_file(&client).await?;
    let before = fake.requests().len();
    let given = json!({ "path": path, "into": "Empty", "confirm": true });
    file::import(&client, &CacheSettings::default(), &args(&given)?).await?;
    let sent = since(&fake, before);
    assert_eq!(
        channel_reads(&sent, fake.id_of("Empty").as_deref()),
        1,
        "{sent:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_version_3_board_is_refused_by_import_into() -> holst_core::Result<()> {
    let (fake, client) = started(whole_boards()).await?;
    let (_scratch, path) = roadmap_file(&client).await?;
    let given = json!({ "path": path, "into": "Newer", "confirm": true });
    let done = file::import(&client, &CacheSettings::default(), &args(&given)?).await;
    assert!(
        matches!(
            done,
            Err(holst_core::Error::BoardSocket(
                BoardSocketError::UnsupportedFormat { schema: 3, .. }
            ))
        ),
        "{done:?}"
    );
    assert_eq!(
        fake.requests()
            .iter()
            .filter(|each| *each == "WS open")
            .count(),
        1,
        "only the export opened a session"
    );
    Ok(())
}

#[tokio::test]
async fn backup_restore_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client) = started(whole_boards()).await?;
    let before = fake.requests().len();
    let given = json!({ "board": "Roadmap", "key": "snap-1", "confirm": true });
    backup::restore(&client, &CacheSettings::default(), &args(&given)?).await?;
    let sent = since(&fake, before);
    assert_eq!(
        channel_reads(&sent, fake.id_of("Roadmap").as_deref()),
        1,
        "{sent:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_withheld_removal_still_refuses_backup_restore() -> holst_core::Result<()> {
    let (fake, client) = started(whole_boards()).await?;
    fake.withhold("Roadmap", "who_can_delete_content", "co_owner", "editor");
    let given = json!({ "board": "Roadmap", "key": "snap-1", "confirm": true });
    let done =
        backup::restore(&client, &CacheSettings::default(), &args(&given)?).await;
    assert!(done.is_err(), "{done:?}");
    let sent = fake.requests();
    assert_eq!(
        sent.iter().filter(|each| *each == "WS open").count(),
        0,
        "{sent:?}"
    );
    Ok(())
}

#[tokio::test]
async fn export_to_downloads_reads_the_board_once() -> holst_core::Result<()> {
    let (fake, client) = started(whole_boards()).await?;
    let downloads = Scratch::new()?;
    let before = fake.requests().len();
    let saved = file::export(
        &client,
        &CacheSettings::default(),
        Some(&downloads.dir),
        &args(&json!({ "board": "Roadmap" }))?,
    )
    .await?;
    let sent = since(&fake, before);
    assert_eq!(
        channel_reads(&sent, fake.id_of("Roadmap").as_deref()),
        1,
        "{sent:?}"
    );
    let name = saved
        .file
        .file_name()
        .map(|each| each.to_string_lossy().into_owned());
    assert!(
        name.is_some_and(
            |each| each.starts_with("Roadmap-") && each.ends_with(".ydoc.gz")
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_export_writes_nothing() -> holst_core::Result<()> {
    let (_fake, client) = started(whole_boards()).await?;
    let downloads = Scratch::new()?;
    let done = file::export(
        &client,
        &CacheSettings::default(),
        Some(&downloads.dir),
        &args(&json!({ "board": "Newer" }))?,
    )
    .await;
    assert!(done.is_err(), "{done:?}");
    assert!(!downloads.dir.exists());
    Ok(())
}

fn tree_reads(requests: &[String]) -> usize {
    requests
        .iter()
        .filter(|request| request.ends_with("/tree"))
        .count()
}

#[tokio::test]
async fn board_ls_asks_the_trees_each_time_and_keeps_them() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    run(&client, Run::BoardLs).await?;
    assert_eq!(tree_reads(&fake.requests()), 4);
    let before = fake.requests().len();
    run(&client, Run::BoardShow).await?;
    assert_eq!(tree_reads(&since(&fake, before)), 0);
    Ok(())
}

#[rstest]
#[case::both("both")]
#[case::sidebar("sidebar")]
#[tokio::test]
async fn the_other_views_ask_the_trees_and_keep_them(
    #[case] view: &str,
) -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    tree::ls(&client, &args(&json!({ "view": view }))?).await?;
    tree::ls(&client, &args(&json!({ "view": view }))?).await?;
    assert_eq!(tree_reads(&fake.requests()), 4);
    let before = fake.requests().len();
    run(&client, Run::BoardShow).await?;
    assert_eq!(tree_reads(&since(&fake, before)), 0);
    Ok(())
}

#[tokio::test]
async fn recent_boards_are_named_from_the_kept_trees() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    own::touch(&client, &args(&json!({ "board": "Plans" }))?).await?;
    let before = fake.requests().len();
    let listing = tree::ls(&client, &args(&json!({ "view": "recent" }))?).await?;
    assert_eq!(tree_reads(&since(&fake, before)), 0);
    let names: Vec<_> = listing
        .nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(names, ["Plans"]);
    Ok(())
}

#[tokio::test]
async fn auth_status_keeps_the_trees_it_counted() -> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    auth::status(&client, &args(&json!({}))?).await;
    assert_eq!(tree_reads(&fake.requests()), 2);
    let before = fake.requests().len();
    run(&client, Run::BoardShow).await?;
    assert_eq!(tree_reads(&since(&fake, before)), 0);
    Ok(())
}

#[tokio::test]
async fn a_recent_board_no_kept_tree_holds_asks_the_trees_once()
-> holst_core::Result<()> {
    let (fake, client) = started(plans()).await?;
    run(&client, Run::BoardLs).await?;
    let elsewhere = bare(&fake)?;
    let made = json!({ "name": "Nova", "workspace": "Team" });
    tree_change::create(&elsewhere, &args(&made)?).await?;
    let nova = fake.id_of("Nova").unwrap_or_default();
    own::touch(&elsewhere, &args(&json!({ "board": nova }))?).await?;
    let before = fake.requests().len();
    let listing = tree::ls(&client, &args(&json!({ "view": "recent" }))?).await?;
    assert_eq!(tree_reads(&since(&fake, before)), 2);
    let names: Vec<_> = listing
        .nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(names, ["Nova"]);
    Ok(())
}
/// A client on a cache that a `board ls` filled, running changes.
async fn listed(fixture: Fixture) -> holst_core::Result<(Fake, Client)> {
    let (fake, client) = started(fixture).await?;
    run(&client, Run::BoardLs).await?;
    Ok((fake, client.for_change()))
}

fn with_folder() -> Fixture {
    plans().folder("Archive", "Team")
}

fn channel_path(fake: &Fake, name: &str, tail: &str) -> String {
    format!(
        "/gapi/channel/{}{tail}",
        fake.id_of(name).unwrap_or_default()
    )
}

#[tokio::test]
async fn a_warm_move_by_name_sends_only_the_move() -> holst_core::Result<()> {
    let (fake, client) = listed(with_folder()).await?;
    let before = fake.requests().len();
    let given = json!({ "board": "Plans", "folder": "Archive" });
    tree_change::mv(&client, &args(&given)?).await?;
    let sent = since(&fake, before);
    assert_eq!(
        sent,
        [format!("POST {}", channel_path(&fake, "Plans", "/move"))]
    );
    Ok(())
}

#[tokio::test]
async fn a_warm_rename_by_name_sends_only_the_rename() -> holst_core::Result<()> {
    let (fake, client) = listed(plans()).await?;
    let path = channel_path(&fake, "Plans", "");
    let before = fake.requests().len();
    let given = json!({ "board": "Plans", "name": "Plans2" });
    let renamed = tree_change::rename(&client, &args(&given)?).await?;
    let sent = since(&fake, before);
    assert_eq!(sent, [format!("PATCH {path}")]);
    assert_eq!(renamed.node.previous_name.as_deref(), Some("Plans"));
    Ok(())
}

#[tokio::test]
async fn a_warm_board_removal_by_name_sends_only_the_delete() -> holst_core::Result<()>
{
    let (fake, client) = listed(plans()).await?;
    let path = channel_path(&fake, "Plans", "/delete");
    let before = fake.requests().len();
    tree_change::rm(&client, &args(&json!({ "board": "Plans" }))?).await?;
    let sent = since(&fake, before);
    assert_eq!(sent, [format!("POST {path}")]);
    Ok(())
}

#[tokio::test]
async fn a_warm_board_in_a_folder_by_name_reads_no_tree() -> holst_core::Result<()> {
    let (fake, client) = listed(with_folder()).await?;
    let before = fake.requests().len();
    let given = json!({ "name": "Notes", "workspace": "Team", "folder": "Archive" });
    tree_change::create(&client, &args(&given)?).await?;
    assert_eq!(tree_reads(&since(&fake, before)), 0);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Change {
    Create,
    Rename,
    Move,
    Copy,
    Remove,
}

impl Change {
    async fn run(self, client: &Client, given: &Value) -> holst_core::Result<()> {
        match self {
            Self::Create => tree_change::create(client, &args(given)?).await.map(drop),
            Self::Rename => tree_change::rename(client, &args(given)?).await.map(drop),
            Self::Move => tree_change::mv(client, &args(given)?).await.map(drop),
            Self::Copy => tree_change::cp(client, &args(given)?).await.map(drop),
            Self::Remove => tree_change::rm(client, &args(given)?).await.map(drop),
        }
    }
}

async fn show_reads_trees(
    fake: &Fake,
    client: &Client,
    board: &str,
) -> holst_core::Result<usize> {
    let before = fake.requests().len();
    run_show(client, board).await?;
    Ok(tree_reads(&since(fake, before)))
}

async fn run_show(client: &Client, board: &str) -> holst_core::Result<()> {
    let given = json!({ "board": board });
    show(client, &CacheSettings::default(), &args(&given)?)
        .await
        .map(drop)
}

#[tokio::test]
async fn a_board_made_is_found_by_name_by_the_next_command() -> holst_core::Result<()> {
    let (_fake, client) = listed(plans()).await?;
    let given = json!({ "name": "Notes", "workspace": "Team" });
    tree_change::create(&client, &args(&given)?).await?;
    let found = list::ls(
        &client,
        &CacheSettings::default(),
        &args(&json!({ "board": "Notes" }))?,
    )
    .await;
    assert!(found.is_ok(), "{found:?}");
    Ok(())
}

#[rstest]
#[case::create(json!({ "name": "Notes", "workspace": "Team", "dry_run": true }), Change::Create)]
#[case::rename(json!({ "board": "Plans", "name": "Plans2", "dry_run": true }), Change::Rename)]
#[case::mv(json!({ "board": "Plans", "folder": "Archive", "dry_run": true }), Change::Move)]
#[case::cp(json!({ "board": "Plans", "folder": "Archive", "dry_run": true }), Change::Copy)]
#[case::trash(json!({ "board": "Plans", "dry_run": true }), Change::Remove)]
#[case::forever(json!({ "board": "Plans", "permanently": true, "dry_run": true }), Change::Remove)]
#[tokio::test]
async fn a_dry_run_keeps_the_tree(
    #[case] given: Value,
    #[case] change: Change,
) -> holst_core::Result<()> {
    let (fake, client) = listed(with_folder()).await?;
    change.run(&client, &given).await?;
    assert_eq!(show_reads_trees(&fake, &client, "Plans").await?, 0);
    Ok(())
}

#[tokio::test]
async fn a_held_change_keeps_the_tree() -> holst_core::Result<()> {
    let (fake, client) = listed(in_folder().folder("Spare", "Team")).await?;
    let given = json!({ "board": "Archive", "folder": "Spare" });
    let held = tree_change::mv(&client, &args(&given)?).await?;
    assert!(
        matches!(
            held.outcome,
            holst_core::command::change::ChangeReport::Held { .. }
        ),
        "{held:?}"
    );
    assert_eq!(show_reads_trees(&fake, &client, "Plans").await?, 0);
    Ok(())
}

#[tokio::test]
async fn a_folder_removal_held_for_good_keeps_the_tree() -> holst_core::Result<()> {
    let (fake, client) = listed(with_folder()).await?;
    let held = tree_change::rm(&client, &args(&json!({ "board": "Archive" }))?).await?;
    assert!(
        matches!(
            held.outcome,
            holst_core::command::change::ChangeReport::Held { .. }
        ),
        "{held:?}"
    );
    assert_eq!(show_reads_trees(&fake, &client, "Plans").await?, 0);
    Ok(())
}

#[tokio::test]
async fn a_rename_leaves_one_tree_to_read() -> holst_core::Result<()> {
    let (fake, client) = listed(plans()).await?;
    let given = json!({ "board": "Plans", "name": "Plans2" });
    tree_change::rename(&client, &args(&given)?).await?;
    assert_eq!(show_reads_trees(&fake, &client, "Plans2").await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_move_leaves_one_tree_to_read() -> holst_core::Result<()> {
    let (fake, client) = listed(with_folder()).await?;
    let given = json!({ "board": "Plans", "folder": "Archive" });
    tree_change::mv(&client, &args(&given)?).await?;
    assert_eq!(show_reads_trees(&fake, &client, "Plans").await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_copy_leaves_one_tree_to_read() -> holst_core::Result<()> {
    let (fake, client) = listed(plans()).await?;
    let given = json!({ "board": "Plans", "name": "Plans copy" });
    tree_change::cp(&client, &args(&given)?).await?;
    assert_eq!(show_reads_trees(&fake, &client, "Plans copy").await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_trashed_board_is_not_found_by_name_in_a_stale_tree() -> holst_core::Result<()>
{
    let (_fake, client) = listed(plans()).await?;
    tree_change::rm(&client, &args(&json!({ "board": "Plans" }))?).await?;
    let found = lookup::board_id(&client, "Plans", None).await;
    assert!(found.is_err(), "{found:?}");
    Ok(())
}

#[tokio::test]
async fn a_board_deleted_for_good_is_not_found_by_name() -> holst_core::Result<()> {
    let (_fake, client) = listed(plans()).await?;
    let given = json!({ "board": "Plans", "permanently": true, "confirm": true });
    tree_change::rm(&client, &args(&given)?).await?;
    let found = lookup::board_id(&client, "Plans", None).await;
    assert!(found.is_err(), "{found:?}");
    Ok(())
}

#[tokio::test]
async fn a_restored_board_leaves_one_tree_to_read() -> holst_core::Result<()> {
    let (fake, client) = listed(plans()).await?;
    tree_change::rm(&client, &args(&json!({ "board": "Plans" }))?).await?;
    let given = json!({ "board": "Plans", "workspace": "Team" });
    tree_change::restore(&client, &args(&given)?).await?;
    assert_eq!(show_reads_trees(&fake, &client, "Plans").await?, 1);
    Ok(())
}

fn in_folder() -> Fixture {
    Fixture::default()
        .workspace("Team")
        .folder("Archive", "Team")
        .board("Plans", "Team", Some("Archive"))
}

async fn shown_json(client: &Client, board: &Value) -> holst_core::Result<Value> {
    let given = json!({ "board": board });
    let shown = show(client, &CacheSettings::default(), &args(&given)?).await?;
    Ok(serde_json::to_value(shown)?)
}

#[tokio::test]
async fn board_show_names_its_workspace_and_folder_from_the_tree()
-> holst_core::Result<()> {
    let (fake, client) = started(in_folder()).await?;
    let shown = shown_json(&client, &json!("Plans")).await?;
    let board = shown.pointer("/boards/0").unwrap_or(&Value::Null);
    assert_eq!(board["workspace_name"], "Team", "{board}");
    assert_eq!(board["folder_name"], "Archive", "{board}");
    assert_eq!(board["folder"], json!(fake.id_of("Archive")), "{board}");
    Ok(())
}

#[tokio::test]
async fn board_show_by_id_on_an_empty_cache_leaves_the_names_null()
-> holst_core::Result<()> {
    let (fake, client) = started(in_folder()).await?;
    let id = fake.id_of("Plans").unwrap_or_default();
    let shown = shown_json(&client, &json!(id)).await?;
    let board = shown.pointer("/boards/0").unwrap_or(&Value::Null);
    assert_eq!(board["workspace_name"], Value::Null, "{board}");
    assert_eq!(board["folder_name"], Value::Null, "{board}");
    assert!(board["workspace"].is_string(), "{board}");
    Ok(())
}

#[tokio::test]
async fn board_show_by_name_sends_what_it_sent_before_names() -> holst_core::Result<()>
{
    let (fake, client) = started(in_folder()).await?;
    run_show(&client, "Plans").await?;
    let plans = fake.id_of("Plans").unwrap_or_default();
    let team = "00000000-0000-4000-8000-000000000001";
    let requests = fake.requests();
    let (first, rest) = requests.split_at(2);
    let document = format!("WS RequestDocument {plans}");
    let mut together = rest.to_vec();
    together.sort();
    assert_eq!(
        first,
        [
            CURRENT_USER.to_owned(),
            format!("GET /gapi/workspace/{team}/tree")
        ]
    );
    assert_eq!(
        together,
        [
            format!("GET /gapi/channel/{plans}"),
            document.clone(),
            "WS open".to_owned()
        ],
        "the channel read runs beside the socket"
    );
    let position = |wanted: &str| rest.iter().position(|each| each == wanted);
    assert!(position("WS open") < position(&document), "{rest:?}");
    Ok(())
}

#[tokio::test]
async fn several_boards_by_name_read_one_tree_per_workspace() -> holst_core::Result<()>
{
    let (fake, client) = started(plans().board("Ideas", "Other", None)).await?;
    let shown = shown_json(&client, &json!(["Plans", "Ideas"])).await?;
    assert_eq!(
        shown
            .pointer("/boards")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "{shown}"
    );
    let requests = fake.requests();
    assert_eq!(tree_reads(&requests), 2, "{requests:?}");
    assert_eq!(asked(&requests), 1, "{requests:?}");
    Ok(())
}

#[tokio::test]
async fn a_warm_favorite_does_not_read_the_channel() -> holst_core::Result<()> {
    let (fake, client) = listed(plans()).await?;
    let before = fake.requests().len();
    own::favorite(&client, &args(&json!({ "board": "Plans" }))?).await?;
    let sent = since(&fake, before);
    let channel = format!("GET {}", channel_path(&fake, "Plans", ""));
    assert!(!sent.contains(&channel), "{sent:?}");
    assert_eq!(sent.len(), 1, "{sent:?}");
    Ok(())
}

#[tokio::test]
async fn a_warm_favorite_reports_the_name_of_the_tree() -> holst_core::Result<()> {
    let (_fake, client) = listed(plans()).await?;
    let changed = own::favorite(&client, &args(&json!({ "board": "Plans" }))?).await?;
    assert_eq!(changed.node.name.as_deref(), Some("Plans"));
    Ok(())
}

#[tokio::test]
async fn leave_still_reads_the_channel() -> holst_core::Result<()> {
    let (fake, client) =
        listed(plans().board_owned_by("Mine", "Team", "someone")).await?;
    let before = fake.requests().len();
    drop(own::leave(&client, &args(&json!({ "board": "Mine" }))?).await);
    let sent = since(&fake, before);
    let channel = format!("GET {}", channel_path(&fake, "Mine", ""));
    assert!(sent.contains(&channel), "{sent:?}");
    Ok(())
}

const START: u64 = 1_700_000_000_000;
const TWO_HOURS: u64 = 7_200_000;
const TEAM: &str = "00000000-0000-4000-8000-000000000001";
const OTHER: &str = "00000000-0000-4000-8000-000000000002";

/// The requests one command sent, by kind; `other` holds the rest, sorted.
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

const fn sent() -> Sent {
    Sent {
        current_user: 0,
        tree: 0,
        trash: 0,
        channel: 0,
        socket: 0,
        document: 0,
        update: 0,
        storage: 0,
        other: Vec::new(),
    }
}

fn sorted_with(mut requests: Vec<String>, request: String) -> Vec<String> {
    requests.push(request);
    requests.sort();
    requests
}

impl Sent {
    fn current_user(self, count: usize) -> Self {
        Self {
            current_user: count,
            ..self
        }
    }

    fn tree(self, count: usize) -> Self {
        Self {
            tree: count,
            ..self
        }
    }

    fn trash(self, count: usize) -> Self {
        Self {
            trash: count,
            ..self
        }
    }

    fn channel(self, count: usize) -> Self {
        Self {
            channel: count,
            ..self
        }
    }

    fn socket(self, count: usize) -> Self {
        Self {
            socket: count,
            ..self
        }
    }

    fn document(self, count: usize) -> Self {
        Self {
            document: count,
            ..self
        }
    }

    fn update(self, count: usize) -> Self {
        Self {
            update: count,
            ..self
        }
    }

    fn storage(self, count: usize) -> Self {
        Self {
            storage: count,
            ..self
        }
    }

    fn other(self, requests: &[&str]) -> Self {
        Self {
            other: requests.iter().map(|each| (*each).to_owned()).collect(),
            ..self
        }
    }

    /// The same requests and `count` more trees.
    fn more_trees(self, count: usize) -> Self {
        Self {
            tree: self.tree + count,
            ..self
        }
    }

    fn counting(self, request: &str, named: String) -> Self {
        match request {
            CURRENT_USER => Self {
                current_user: self.current_user + 1,
                ..self
            },
            "WS open" => Self {
                socket: self.socket + 1,
                ..self
            },
            _ if request.starts_with("WS RequestDocument") => Self {
                document: self.document + 1,
                ..self
            },
            _ if request.starts_with("WS Update") => Self {
                update: self.update + 1,
                ..self
            },
            _ if request.starts_with("GET /storage/") => Self {
                storage: self.storage + 1,
                ..self
            },
            _ if request.ends_with("/tree") => Self {
                tree: self.tree + 1,
                ..self
            },
            _ if request.ends_with("/trash") => Self {
                trash: self.trash + 1,
                ..self
            },
            _ if request
                .strip_prefix("GET /gapi/channel/")
                .is_some_and(|rest| !rest.contains('/')) =>
            {
                Self {
                    channel: self.channel + 1,
                    ..self
                }
            }
            _ => Self {
                other: sorted_with(self.other, named),
                ..self
            },
        }
    }
}

/// How a row gets its cache warm, and what it needs on the server.
#[derive(Debug, Clone, Copy)]
enum Setup {
    /// A read: warm is the second run on the same client.
    Read,
    /// A change: warm is after a `board ls` kept the trees.
    Write,
    /// A change on a board another client moved to the trash.
    Trashed,
    /// A change that reads a board file and a cover picture.
    Filed,
    /// A change on a board whose first object another client tagged `urgent`.
    Tagged,
    /// A change on a board whose first object another client locked.
    Locked,
}

/// The fixture of every row: two workspaces, a folder, the boards `Plans` and
/// `Ideas`, comments, templates, backups and people.
fn world_fixture() -> Fixture {
    let roadmap = || {
        vec![
            FixtureObject::Sticker("Roadmap".into()),
            FixtureObject::Sticker("Second".into()),
            FixtureObject::File("spec.pdf".into(), 1000),
            FixtureObject::Task("Ship".into()),
        ]
    };
    Fixture::default()
        .workspace("Team")
        .workspace("Other")
        .folder("Archive", "Team")
        .board_with_objects("Plans", "Team", roadmap())
        .board("Ideas", "Other", None)
        .invited_board("Shared", "Team", "someone")
        .member("Ann", "Team", WorkspaceRole::Member)
        .event("Team", "Ann", "board.create", "Plans")
        .thread("Plans", FixtureThread::new("t1", 1, "Is this final?"))
        .backup_of_objects("Plans", "snap-1", roadmap())
        .personal_template("Retro")
        .template_category("Retros", "retros", &["Start Stop"])
}

struct World {
    fake: Fake,
    client: Client,
    cache: Arc<Scratch>,
    out: Arc<Scratch>,
    ids: Vec<(String, String)>,
}

fn at_clock(fake: &Fake, cache: &Arc<Scratch>, now: u64) -> holst_core::Result<Client> {
    Ok(connect(
        Some(saved_login()),
        fake.base_url().parse()?,
        FixedClock { unix_millis: now },
        RandomIds,
    )?
    .with_account_cache(Arc::<Scratch>::clone(cache)))
}

async fn logged_in(
    client: Client,
    previous: Option<Client>,
) -> holst_core::Result<Client> {
    finish(client, previous, async |_| Ok::<(), Infallible>(()))
        .await
        .map(|done| done.client)
        .map_err(|stopped| match stopped {
            Stopped::Check(error) => error,
            Stopped::Save(never) => match never {},
        })
}

impl World {
    /// After `login::finish`: account and workspaces known, no tree kept.
    async fn open(setup: Setup) -> holst_core::Result<Self> {
        let fake = Fake::start(world_fixture()).await?;
        let cache = Scratch::new()?;
        let client = logged_in(at_clock(&fake, &cache, START)?, None).await?;
        let out = Scratch::new()?;
        fs::create_dir_all(&out.dir)?;
        match setup {
            Setup::Filed => make_files(&out.dir).await,
            Setup::Read
            | Setup::Write
            | Setup::Trashed
            | Setup::Tagged
            | Setup::Locked => Ok(()),
        }?;
        let ids = (1..=4)
            .filter_map(|number| {
                fake.object_id("Plans", number)
                    .map(|id| (format!("<Plans.{number}>"), id))
            })
            .chain(
                ["Plans", "Ideas", "Archive", "Shared"]
                    .into_iter()
                    .filter_map(|name| {
                        fake.id_of(name).map(|id| (format!("<{name}>"), id))
                    }),
            )
            .chain([
                ("<Team>".to_owned(), TEAM.to_owned()),
                ("<Other>".to_owned(), OTHER.to_owned()),
                ("<me>".to_owned(), FAKE_USER.to_owned()),
                ("<out>".to_owned(), out.dir.to_string_lossy().into_owned()),
            ])
            .collect();
        Ok(Self {
            fake,
            client,
            cache,
            out,
            ids,
        })
    }

    /// The same login on a new client whose clock stands at `now`, same cache.
    fn later(self, now: u64) -> holst_core::Result<Self> {
        Ok(Self {
            client: at_clock(&self.fake, &self.cache, now)?,
            ..self
        })
    }

    /// The client runs a change when the command is not a read.
    fn acting(self, risk: CommandRisk) -> Self {
        Self {
            client: match risk {
                CommandRisk::Read => self.client,
                CommandRisk::Write | CommandRisk::Destroy => self.client.for_change(),
            },
            ..self
        }
    }

    fn filled(&self, given: &Value) -> holst_core::Result<Value> {
        let text = self
            .ids
            .iter()
            // escaped as JSON: a Windows path holds backslashes
            .fold(given.to_string(), |text, (name, id)| {
                let quoted = Value::from(id.as_str()).to_string();
                text.replace(name, quoted.trim_matches('"'))
            });
        Ok(serde_json::from_str(&text)?)
    }

    /// The request with the ids it holds replaced by the names of the fixture.
    fn named(&self, request: &str) -> String {
        self.ids
            .iter()
            .fold(request.to_owned(), |text, (name, id)| {
                text.replace(id, name)
            })
    }

    fn tally(&self, requests: &[String]) -> Sent {
        requests.iter().fold(sent(), |sent, request| {
            sent.counting(request, self.named(request))
        })
    }

    /// What another client did first, for the setups that need it.
    async fn prepared(self, setup: Setup) -> holst_core::Result<Self> {
        let cache = &CacheSettings::default();
        match setup {
            Setup::Trashed => {
                let given = args(&json!({ "board": "Plans" }))?;
                tree_change::rm(&bare(&self.fake)?, &given).await.map(drop)
            }
            Setup::Tagged => {
                let tagged = json!({
                    "board": "Plans", "object": ["<Plans.1>"], "tag": ["urgent"], "confirm": true
                });
                let given = args(&self.filled(&tagged)?)?;
                change::edit(&bare(&self.fake)?, cache, &given)
                    .await
                    .map(drop)
            }
            Setup::Locked => {
                let locked = json!({ "board": "Plans", "object": ["<Plans.1>"] });
                let given = args(&self.filled(&locked)?)?;
                lock::lock(&bare(&self.fake)?, cache, &given)
                    .await
                    .map(drop)
            }
            Setup::Read | Setup::Write | Setup::Filed => Ok(()),
        }?;
        Ok(self)
    }

    /// Runs the command; the requests it sent.
    async fn measured(
        &self,
        name: CommandName,
        given: &Value,
    ) -> holst_core::Result<Sent> {
        let before = self.fake.requests().len();
        run_command(self, name, &self.filled(given)?).await?;
        Ok(self.tally(&since(&self.fake, before)))
    }

    /// Fills the cache the way `setup` says before the command runs.
    async fn warmed(
        self,
        setup: Setup,
        name: CommandName,
        given: &Value,
    ) -> holst_core::Result<Self> {
        let world = self.acting(name.command().risk);
        match setup {
            Setup::Read => run_command(&world, name, &world.filled(given)?).await?,
            Setup::Write
            | Setup::Trashed
            | Setup::Filed
            | Setup::Tagged
            | Setup::Locked => {
                tree::ls(&world.client, &args(&json!({}))?)
                    .await
                    .map(drop)?;
            }
        }
        Ok(world)
    }
}

/// A board file of `Roadmap` and a cover picture in `dir`.
async fn make_files(dir: &std::path::Path) -> holst_core::Result<()> {
    let (_fake, client) = started(whole_boards()).await?;
    let (scratch, path) = roadmap_file(&client).await?;
    fs::copy(&path, dir.join("roadmap.ydoc"))?;
    fs::write(dir.join("cover.png"), holst_core::fake::png(8, 8))?;
    drop(scratch);
    Ok(())
}

async fn run_command(
    world: &World,
    name: CommandName,
    given: &Value,
) -> holst_core::Result<()> {
    let client = &world.client;
    match name {
        Auth(AuthVerb::Status) => {
            auth::status(client, &args(given)?).await;
            Ok(())
        }
        Auth(AuthVerb::Logout) => logout(client).await.map(drop),
        Workspace(verb) => run_workspace(client, verb, given).await,
        Board(verb) => run_board(world, verb, given).await,
        Comment(verb) => run_comment(world, verb, given).await,
        Template(verb) => run_template(client, verb, given).await,
        Backup(verb) => run_backup(world, verb, given).await,
        Object(verb) => run_object(world, verb, given).await,
        Search => search(client, &CacheSettings::default(), &args(given)?)
            .await
            .map(drop),
        // `auth login` has a test of its own; `storage` is not counted.
        Auth(AuthVerb::Login) | Storage(_) => Ok(()),
    }
}

async fn run_workspace(
    client: &Client,
    verb: WorkspaceVerb,
    given: &Value,
) -> holst_core::Result<()> {
    match verb {
        WorkspaceVerb::Ls => workspace::list(client, &args(given)?).await.map(drop),
        WorkspaceVerb::Show => workspace::show(client, &args(given)?).await.map(drop),
        WorkspaceVerb::Create => {
            workspace::create(client, &args(given)?).await.map(drop)
        }
        WorkspaceVerb::Rename => {
            workspace::rename(client, &args(given)?).await.map(drop)
        }
        WorkspaceVerb::Log => workspace::log(client, &args(given)?).await.map(drop),
    }
}

async fn run_board(
    world: &World,
    verb: BoardVerb,
    given: &Value,
) -> holst_core::Result<()> {
    let client = &world.client;
    let cache = &CacheSettings::default();
    let out = Some(world.out.dir.as_path());
    match verb {
        BoardVerb::Ls => tree::ls(client, &args(given)?).await.map(drop),
        BoardVerb::Create => tree_change::create(client, &args(given)?).await.map(drop),
        BoardVerb::Show => show(client, cache, &args(given)?).await.map(drop),
        BoardVerb::Snap => picture::snap(client, cache, out, &args(given)?)
            .await
            .map(drop),
        BoardVerb::Save => picture::save(client, cache, out, &args(given)?)
            .await
            .map(drop),
        BoardVerb::Export => file::export(client, cache, out, &args(given)?)
            .await
            .map(drop),
        BoardVerb::Import => file::import(client, cache, &args(given)?).await.map(drop),
        BoardVerb::Edit => settings::edit(client, cache, &args(given)?).await.map(drop),
        BoardVerb::Tag => tag::tag(client, cache, &args(given)?).await.map(drop),
        BoardVerb::Rename => tree_change::rename(client, &args(given)?).await.map(drop),
        BoardVerb::Mv => tree_change::mv(client, &args(given)?).await.map(drop),
        BoardVerb::Cp => tree_change::cp(client, &args(given)?).await.map(drop),
        BoardVerb::Rm => tree_change::rm(client, &args(given)?).await.map(drop),
        BoardVerb::Restore => {
            tree_change::restore(client, &args(given)?).await.map(drop)
        }
        BoardVerb::Favorite => own::favorite(client, &args(given)?).await.map(drop),
        BoardVerb::Unfavorite => own::unfavorite(client, &args(given)?).await.map(drop),
        BoardVerb::Touch => own::touch(client, &args(given)?).await.map(drop),
        BoardVerb::Leave => own::leave(client, &args(given)?).await.map(drop),
        BoardVerb::Grant => rights::grant(client, &args(given)?).await.map(drop),
        BoardVerb::Revoke => rights::revoke(client, &args(given)?).await.map(drop),
        BoardVerb::Share => rights::share(client, &args(given)?).await.map(drop),
        BoardVerb::Transfer => rights::transfer(client, &args(given)?).await.map(drop),
    }
}

async fn run_comment(
    world: &World,
    verb: CommentVerb,
    given: &Value,
) -> holst_core::Result<()> {
    let client = &world.client;
    let cache = &CacheSettings::default();
    match verb {
        CommentVerb::Ls => comment::ls(client, &args(given)?).await.map(drop),
        CommentVerb::Add => comment::add(client, cache, &args(given)?).await.map(drop),
        CommentVerb::Reply => comment::reply(client, &args(given)?).await.map(drop),
        CommentVerb::Resolve => comment::resolve(client, &args(given)?).await.map(drop),
        CommentVerb::Edit => {
            comment::edit(client, cache, &args(given)?).await.map(drop)
        }
        CommentVerb::Rm => comment::rm(client, &args(given)?).await.map(drop),
        CommentVerb::Read => comment::read(client, &args(given)?).await.map(drop),
    }
}

async fn run_template(
    client: &Client,
    verb: TemplateVerb,
    given: &Value,
) -> holst_core::Result<()> {
    match verb {
        TemplateVerb::Ls => template::ls(client, &args(given)?).await.map(drop),
        TemplateVerb::Show => template::show(client, &args(given)?).await.map(drop),
        TemplateVerb::Create => template::create(client, &args(given)?).await.map(drop),
        TemplateVerb::Edit => template::edit(client, &args(given)?).await.map(drop),
        TemplateVerb::Rm => template::rm(client, &args(given)?).await.map(drop),
    }
}

async fn run_backup(
    world: &World,
    verb: BackupVerb,
    given: &Value,
) -> holst_core::Result<()> {
    let client = &world.client;
    match verb {
        BackupVerb::Ls => backup::ls(client, &args(given)?).await.map(drop),
        BackupVerb::Get => {
            backup::get(client, Some(world.out.dir.as_path()), &args(given)?)
                .await
                .map(drop)
        }
        BackupVerb::Restore => {
            backup::restore(client, &CacheSettings::default(), &args(given)?)
                .await
                .map(drop)
        }
    }
}

async fn run_object(
    world: &World,
    verb: ObjectVerb,
    given: &Value,
) -> holst_core::Result<()> {
    let client = &world.client;
    let cache = &CacheSettings::default();
    match verb {
        ObjectVerb::Ls => list::ls(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Add => add::add(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Edit => change::edit(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Cp => copy::cp(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Group => group::group(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Ungroup => {
            group::ungroup(client, cache, &args(given)?).await.map(drop)
        }
        ObjectVerb::Lock => lock::lock(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Unlock => {
            lock::unlock(client, cache, &args(given)?).await.map(drop)
        }
        ObjectVerb::Align => align::align(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Rm => change::rm(client, cache, &args(given)?).await.map(drop),
        ObjectVerb::Get => {
            let out = Some(world.out.dir.as_path());
            get::get(client, cache, out, &args(given)?).await.map(drop)
        }
    }
}

/// Requests of `name` cold, then kept as `setup` says, then two hours ago.
/// Warm is the second run; the third is for a client that acts.
async fn observed(
    name: CommandName,
    given: &Value,
    setup: Setup,
) -> holst_core::Result<(Sent, Sent, Sent)> {
    let risk = name.command().risk;
    match setup {
        Setup::Read => {
            let world = World::open(setup).await?;
            let cold = world.measured(name, given).await?;
            let warm = world.measured(name, given).await?;
            Ok((cold, warm.clone(), warm))
        }
        Setup::Write
        | Setup::Trashed
        | Setup::Filed
        | Setup::Tagged
        | Setup::Locked => {
            let cold = World::open(setup)
                .await?
                .acting(risk)
                .prepared(setup)
                .await?
                .measured(name, given)
                .await?;
            let warm = World::open(setup)
                .await?
                .warmed(setup, name, given)
                .await?
                .prepared(setup)
                .await?
                .measured(name, given)
                .await?;
            let stale = World::open(setup)
                .await?
                .warmed(setup, name, given)
                .await?
                .prepared(setup)
                .await?
                .later(START + TWO_HOURS)?
                .acting(risk)
                .measured(name, given)
                .await?;
            Ok((cold, warm, stale))
        }
    }
}

/// Pins requests cold, warm, and with trees kept two hours ago (`later` more).
/// Reruns the world, up to twice, when a dropped connection skews counts.
async fn pins(
    name: CommandName,
    given: Value,
    setup: Setup,
    cold: Sent,
    warm: Sent,
    later: usize,
) -> holst_core::Result<()> {
    let expected = (cold.clone(), warm.clone(), warm.clone().more_trees(later));
    let matches = |seen: &holst_core::Result<(Sent, Sent, Sent)>| {
        seen.as_ref().is_ok_and(|found| *found == expected)
    };
    let mut done = observed(name, &given, setup).await;
    // imperative: a world that shows other counts runs again
    for _ in 0..2 {
        if !matches(&done) {
            done = observed(name, &given, setup).await;
        }
    }
    let (found_cold, found_warm, found_stale) = done?;
    assert_eq!(found_cold, cold, "cold");
    assert_eq!(found_warm, warm, "warm");
    assert_eq!(found_stale, warm.more_trees(later), "stale");
    Ok(())
}

/// The requests as they are, without names for the ids.
fn tallied(requests: &[String]) -> Sent {
    requests.iter().fold(sent(), |sent, request| {
        sent.counting(request, request.clone())
    })
}

/// The whole email login of the command line, to a client on `cache`.
async fn log_in(
    fake: &Fake,
    cache: &Arc<Scratch>,
    previous: Option<Client>,
) -> holst_core::Result<Client> {
    let anonymous = connect(
        None,
        fake.base_url().parse()?,
        FixedClock { unix_millis: START },
        RandomIds,
    )?
    .with_account_cache(Arc::<Scratch>::clone(cache));
    let login = EmailLogin::new(anonymous, "you@example.com".parse()?);
    login.methods().await?;
    let awaiting = login.send_code().await?;
    let client = awaiting.log_in(&SecretString::from(LOGIN_CODE)).await?;
    logged_in(client, previous).await
}

#[tokio::test]
async fn a_login_asks_the_account_once_and_ends_the_session_it_replaces()
-> holst_core::Result<()> {
    let fake = Fake::start(world_fixture()).await?;
    let cache = Scratch::new()?;
    let first = log_in(&fake, &cache, None).await?;
    let cold = tallied(&fake.requests());
    let before = fake.requests().len();
    log_in(&fake, &cache, Some(first)).await?;
    let again = tallied(&since(&fake, before));
    let steps = [
        "POST /gapi/login/get_public_options",
        "POST /gapi/login/with_email",
        "POST /gapi/send_temporary_password",
    ];
    assert_eq!(cold, sent().current_user(1).other(&steps));
    let ended = [steps[0], steps[1], "POST /gapi/logout", steps[2]];
    assert_eq!(again, sent().current_user(1).other(&ended));
    Ok(())
}

#[rstest]
#[case::auth_status(Auth(AuthVerb::Status), json!({}), Setup::Read, sent().current_user(1).tree(2), sent().current_user(1).tree(2), 0)]
#[case::auth_status_sessions(Auth(AuthVerb::Status), json!({ "with": ["sessions"] }), Setup::Read, sent().current_user(1).tree(2).other(&["GET /gapi/session/list"]), sent().current_user(1).tree(2).other(&["GET /gapi/session/list"]), 0)]
#[case::auth_status_tokens(Auth(AuthVerb::Status), json!({ "with": ["tokens"] }), Setup::Read, sent().current_user(1).tree(2).other(&["GET /gapi/personal_access_token/list"]), sent().current_user(1).tree(2).other(&["GET /gapi/personal_access_token/list"]), 0)]
#[case::auth_logout(Auth(AuthVerb::Logout), json!({}), Setup::Write, sent().other(&["POST /gapi/logout"]), sent().other(&["POST /gapi/logout"]), 0)]
#[case::ws_ls(Workspace(WorkspaceVerb::Ls), json!({}), Setup::Read, sent().current_user(1), sent().current_user(1), 0)]
#[case::ws_show(Workspace(WorkspaceVerb::Show), json!({ "workspace": "Team" }), Setup::Read, sent().other(&["GET /gapi/workspace/<Team>"]), sent().other(&["GET /gapi/workspace/<Team>"]), 0)]
#[case::ws_log(Workspace(WorkspaceVerb::Log), json!({ "workspace": "Team" }), Setup::Read, sent().other(&["GET /gapi/audit_events"]), sent().other(&["GET /gapi/audit_events"]), 0)]
#[case::ws_create(Workspace(WorkspaceVerb::Create), json!({ "name": "Studio" }), Setup::Write, sent().other(&["POST /gapi/workspace/create"]), sent().other(&["POST /gapi/workspace/create"]), 0)]
#[case::ws_create_dry(Workspace(WorkspaceVerb::Create), json!({ "name": "Studio", "dry_run": true }), Setup::Write, sent(), sent(), 0)]
#[case::ws_rename(Workspace(WorkspaceVerb::Rename), json!({ "workspace": "Other", "name": "Crew", "confirm": true }), Setup::Write, sent().other(&["PATCH /gapi/workspace/<Other>"]), sent().other(&["PATCH /gapi/workspace/<Other>"]), 0)]
#[case::ws_rename_dry(Workspace(WorkspaceVerb::Rename), json!({ "workspace": "Other", "name": "Crew", "dry_run": true }), Setup::Write, sent(), sent(), 0)]
#[tokio::test]
async fn account_pins(
    #[case] name: CommandName,
    #[case] given: Value,
    #[case] setup: Setup,
    #[case] cold: Sent,
    #[case] warm: Sent,
    #[case] later: usize,
) -> holst_core::Result<()> {
    pins(name, given, setup, cold, warm, later).await
}

#[rstest]
#[case::board_ls(Board(BoardVerb::Ls), json!({}), Setup::Read, sent().tree(2), sent().tree(2), 0)]
#[case::board_ls_both(Board(BoardVerb::Ls), json!({ "view": "both" }), Setup::Read, sent().tree(2).trash(2), sent().tree(2).trash(2), 0)]
#[case::board_ls_sidebar(Board(BoardVerb::Ls), json!({ "view": "sidebar" }), Setup::Read, sent().tree(2).other(&["GET /gapi/channel_user_order_index/<Other>/list", "GET /gapi/channel_user_order_index/<Team>/list"]), sent().tree(2).other(&["GET /gapi/channel_user_order_index/<Other>/list", "GET /gapi/channel_user_order_index/<Team>/list"]), 0)]
#[case::board_ls_trash(Board(BoardVerb::Ls), json!({ "view": "trash" }), Setup::Read, sent().trash(2), sent().trash(2), 0)]
#[case::board_ls_favorites(Board(BoardVerb::Ls), json!({ "view": "favorites" }), Setup::Read, sent().other(&["GET /gapi/favorite_channel/channels/<Other>", "GET /gapi/favorite_channel/channels/<Team>"]), sent().other(&["GET /gapi/favorite_channel/channels/<Other>", "GET /gapi/favorite_channel/channels/<Team>"]), 0)]
#[case::board_ls_recent(Board(BoardVerb::Ls), json!({ "view": "recent" }), Setup::Read, sent().tree(2).other(&["GET /gapi/recent/channels/page"]), sent().other(&["GET /gapi/recent/channels/page"]), 0)]
#[case::board_show_name(Board(BoardVerb::Show), json!({ "board": "Plans" }), Setup::Read, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 0)]
#[case::board_show_id(Board(BoardVerb::Show), json!({ "board": "<Plans>" }), Setup::Read, sent().channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 0)]
#[case::board_show_two(Board(BoardVerb::Show), json!({ "board": ["Plans", "Ideas"] }), Setup::Read, sent().tree(2).channel(2).socket(2).document(2), sent().channel(2).socket(2).document(2), 0)]
#[case::board_snap_name(Board(BoardVerb::Snap), json!({ "board": "Plans", "picture_format": "svg" }), Setup::Read, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 0)]
#[case::board_snap_id(Board(BoardVerb::Snap), json!({ "board": "<Plans>", "picture_format": "svg" }), Setup::Read, sent().channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 0)]
#[case::comment_ls_id(Comment(CommentVerb::Ls), json!({ "board": "<Plans>" }), Setup::Read, sent().socket(1).document(1).other(&["GET /gapi/comment/thread_list/<Plans>"]), sent().socket(1).document(1).other(&["GET /gapi/comment/thread_list/<Plans>"]), 0)]
#[case::search_trash(Search, json!({ "query": "plans", "mode": "names", "trash": true }), Setup::Read, sent().tree(2).trash(2), sent().trash(2), 0)]
#[case::comment_ls_name(Comment(CommentVerb::Ls), json!({ "board": "Plans" }), Setup::Read, sent().tree(2).socket(1).document(1).other(&["GET /gapi/comment/thread_list/<Plans>"]), sent().socket(1).document(1).other(&["GET /gapi/comment/thread_list/<Plans>"]), 0)]
#[case::object_ls_name(Object(ObjectVerb::Ls), json!({ "board": "Plans" }), Setup::Read, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 0)]
#[case::object_ls_id(Object(ObjectVerb::Ls), json!({ "board": "<Plans>" }), Setup::Read, sent().channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 0)]
#[case::search_names(Search, json!({ "query": "plans", "mode": "names" }), Setup::Read, sent().tree(2), sent(), 0)]
#[case::search_text(Search, json!({ "query": "roadmap", "mode": "board_text" }), Setup::Read, sent().tree(2).channel(3).socket(3).document(3), sent().channel(3).socket(3).document(3), 0)]
#[tokio::test]
async fn read_pins(
    #[case] name: CommandName,
    #[case] given: Value,
    #[case] setup: Setup,
    #[case] cold: Sent,
    #[case] warm: Sent,
    #[case] later: usize,
) -> holst_core::Result<()> {
    pins(name, given, setup, cold, warm, later).await
}

#[rstest]
#[case::board_export(Board(BoardVerb::Export), json!({ "board": "Plans" }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 1)]
#[case::board_import_into(Board(BoardVerb::Import), json!({ "path": "<out>/roadmap.ydoc", "into": "Ideas", "confirm": true }), Setup::Filed, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::board_import_into_dry(Board(BoardVerb::Import), json!({ "path": "<out>/roadmap.ydoc", "into": "Ideas", "dry_run": true }), Setup::Filed, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 1)]
#[case::board_import_new(Board(BoardVerb::Import), json!({ "path": "<out>/roadmap.ydoc", "workspace": "Team", "confirm": true }), Setup::Filed, sent().current_user(1).channel(1).socket(1).document(1).update(1).other(&["POST /gapi/channel/create"]), sent().current_user(1).channel(1).socket(1).document(1).update(1).other(&["POST /gapi/channel/create"]), 0)]
#[case::board_edit_note(Board(BoardVerb::Edit), json!({ "board": "Plans", "note": "hello" }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::board_edit_cover_file(Board(BoardVerb::Edit), json!({ "board": "Plans", "cover": "<out>/cover.png" }), Setup::Filed, sent().tree(2).channel(1).socket(1).document(1).other(&["PATCH /gapi/channel/<Plans>", "POST /gapi/upload_file"]), sent().channel(1).socket(1).document(1).other(&["PATCH /gapi/channel/<Plans>", "POST /gapi/upload_file"]), 1)]
#[case::board_create(Board(BoardVerb::Create), json!({ "name": "Notes", "workspace": "Team" }), Setup::Write, sent().current_user(1).other(&["POST /gapi/channel/create"]), sent().current_user(1).other(&["POST /gapi/channel/create"]), 0)]
#[case::board_create_in_folder(Board(BoardVerb::Create), json!({ "name": "Notes", "workspace": "Team", "folder": "Archive" }), Setup::Write, sent().tree(1).other(&["POST /gapi/channel/create"]), sent().other(&["POST /gapi/channel/create"]), 1)]
#[case::board_tag(Board(BoardVerb::Tag), json!({ "board": "Plans", "tag": "urgent", "name": "soon", "confirm": true }), Setup::Tagged, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::board_tag_dry(Board(BoardVerb::Tag), json!({ "board": "Plans", "tag": "urgent", "name": "soon", "dry_run": true }), Setup::Tagged, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 1)]
#[case::board_rename(Board(BoardVerb::Rename), json!({ "board": "Plans", "name": "Plans2" }), Setup::Write, sent().tree(2).other(&["PATCH /gapi/channel/<Plans>"]), sent().other(&["PATCH /gapi/channel/<Plans>"]), 1)]
#[case::board_mv(Board(BoardVerb::Mv), json!({ "board": "Plans", "folder": "Archive", "confirm": true }), Setup::Write, sent().tree(2).other(&["POST /gapi/channel/<Plans>/move"]), sent().other(&["POST /gapi/channel/<Plans>/move"]), 1)]
#[case::board_mv_dry(Board(BoardVerb::Mv), json!({ "board": "Plans", "folder": "Archive", "dry_run": true }), Setup::Write, sent().tree(2), sent(), 1)]
#[case::board_cp(Board(BoardVerb::Cp), json!({ "board": "Plans", "name": "Plans copy" }), Setup::Write, sent().tree(2).channel(1).other(&["POST /gapi/channel/duplicate"]), sent().channel(1).other(&["POST /gapi/channel/duplicate"]), 1)]
#[case::board_cp_into_folder(Board(BoardVerb::Cp), json!({ "board": "Plans", "folder": "Archive" }), Setup::Write, sent().tree(2).channel(1).other(&["POST /gapi/channel/duplicate"]), sent().channel(1).other(&["POST /gapi/channel/duplicate"]), 1)]
#[case::board_rm(Board(BoardVerb::Rm), json!({ "board": "Plans", "confirm": true }), Setup::Write, sent().tree(2).other(&["POST /gapi/channel/<Plans>/delete"]), sent().other(&["POST /gapi/channel/<Plans>/delete"]), 1)]
#[case::board_rm_dry(Board(BoardVerb::Rm), json!({ "board": "Plans", "dry_run": true }), Setup::Write, sent().tree(2), sent(), 1)]
#[case::board_rm_permanently(Board(BoardVerb::Rm), json!({ "board": "Plans", "permanently": true, "confirm": true }), Setup::Write, sent().tree(2).channel(1).other(&["POST /gapi/channel/<Plans>/delete"]), sent().channel(1).other(&["POST /gapi/channel/<Plans>/delete"]), 1)]
#[case::board_restore(Board(BoardVerb::Restore), json!({ "board": "Plans", "workspace": "Team" }), Setup::Trashed, sent().trash(1).other(&["POST /gapi/channel/restore/<Plans>"]), sent().trash(1).other(&["POST /gapi/channel/restore/<Plans>"]), 0)]
#[case::board_favorite(Board(BoardVerb::Favorite), json!({ "board": "Plans" }), Setup::Write, sent().tree(2).other(&["POST /gapi/favorite_channel/add"]), sent().other(&["POST /gapi/favorite_channel/add"]), 1)]
#[case::board_unfavorite(Board(BoardVerb::Unfavorite), json!({ "board": "Plans" }), Setup::Write, sent().tree(2).other(&["DELETE /gapi/favorite_channel/delete"]), sent().other(&["DELETE /gapi/favorite_channel/delete"]), 1)]
#[case::board_touch(Board(BoardVerb::Touch), json!({ "board": "Plans" }), Setup::Write, sent().tree(2).other(&["POST /gapi/recent/set"]), sent().other(&["POST /gapi/recent/set"]), 1)]
#[case::board_leave(Board(BoardVerb::Leave), json!({ "board": "Shared" }), Setup::Write, sent().tree(2).channel(1).other(&["POST /gapi/channel/<Shared>/leave"]), sent().channel(1).other(&["POST /gapi/channel/<Shared>/leave"]), 1)]
#[case::board_grant(Board(BoardVerb::Grant), json!({ "board": "Plans", "user": "Ann", "right": "editor", "confirm": true }), Setup::Write, sent().tree(2).channel(1).other(&["GET /gapi/workspace/<Team>", "POST /gapi/channel_user/set"]), sent().channel(1).other(&["GET /gapi/workspace/<Team>", "POST /gapi/channel_user/set"]), 1)]
#[case::board_grant_dry(Board(BoardVerb::Grant), json!({ "board": "Plans", "user": "Ann", "right": "editor", "dry_run": true }), Setup::Write, sent().tree(2).channel(1).other(&["GET /gapi/workspace/<Team>"]), sent().channel(1).other(&["GET /gapi/workspace/<Team>"]), 1)]
#[case::board_revoke(Board(BoardVerb::Revoke), json!({ "board": "Plans", "user": "Ann", "confirm": true }), Setup::Write, sent().tree(2).channel(1).other(&["GET /gapi/channel_user/users", "GET /gapi/workspace/<Team>"]), sent().channel(1).other(&["GET /gapi/channel_user/users", "GET /gapi/workspace/<Team>"]), 1)]
#[case::board_revoke_dry(Board(BoardVerb::Revoke), json!({ "board": "Plans", "user": "Ann", "dry_run": true }), Setup::Write, sent().tree(2).channel(1).other(&["GET /gapi/channel_user/users", "GET /gapi/workspace/<Team>"]), sent().channel(1).other(&["GET /gapi/channel_user/users", "GET /gapi/workspace/<Team>"]), 1)]
#[case::board_share(Board(BoardVerb::Share), json!({ "board": "Plans", "link": "viewer", "confirm": true }), Setup::Write, sent().tree(2).channel(1).other(&["PATCH /gapi/channel/<Plans>"]), sent().channel(1).other(&["PATCH /gapi/channel/<Plans>"]), 1)]
#[case::board_share_dry(Board(BoardVerb::Share), json!({ "board": "Plans", "link": "viewer", "dry_run": true }), Setup::Write, sent().tree(2).channel(1), sent().channel(1), 1)]
#[case::board_transfer(Board(BoardVerb::Transfer), json!({ "board": "Plans", "user": "Ann", "confirm": true }), Setup::Write, sent().tree(2).channel(1).other(&["GET /gapi/workspace/<Team>", "POST /gapi/channel/<Plans>/transfer_ownership"]), sent().channel(1).other(&["GET /gapi/workspace/<Team>", "POST /gapi/channel/<Plans>/transfer_ownership"]), 1)]
#[case::board_transfer_dry(Board(BoardVerb::Transfer), json!({ "board": "Plans", "user": "Ann", "dry_run": true }), Setup::Write, sent().tree(2).channel(1).other(&["GET /gapi/workspace/<Team>"]), sent().channel(1).other(&["GET /gapi/workspace/<Team>"]), 1)]
#[tokio::test]
async fn board_change_pins(
    #[case] name: CommandName,
    #[case] given: Value,
    #[case] setup: Setup,
    #[case] cold: Sent,
    #[case] warm: Sent,
    #[case] later: usize,
) -> holst_core::Result<()> {
    pins(name, given, setup, cold, warm, later).await
}

#[rstest]
#[case::object_get(Object(ObjectVerb::Get), json!({ "board": "Plans", "object": ["<Plans.3>"] }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).storage(1), sent().channel(1).socket(1).document(1).storage(1), 1)]
#[case::object_add(Object(ObjectVerb::Add), json!({ "board": "Plans", "kind": "sticker", "text": "note" }), Setup::Write, sent().tree(2).channel(1).socket(2).document(2).update(1), sent().channel(1).socket(2).document(2).update(1), 1)]
#[case::object_add_by_id(Object(ObjectVerb::Add), json!({ "board": "<Plans>", "kind": "sticker", "text": "note" }), Setup::Write, sent().channel(1).socket(2).document(2).update(1), sent().channel(1).socket(2).document(2).update(1), 0)]
#[case::object_edit(Object(ObjectVerb::Edit), json!({ "board": "Plans", "object": ["<Plans.1>"], "text": "changed", "confirm": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::object_edit_dry(Object(ObjectVerb::Edit), json!({ "board": "Plans", "object": ["<Plans.1>"], "text": "changed", "dry_run": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 1)]
#[case::object_edit_assignee(Object(ObjectVerb::Edit), json!({ "board": "Plans", "object": ["<Plans.4>"], "assignee": ["Ann"], "confirm": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1).other(&["GET /gapi/workspace/<Team>"]), sent().channel(1).socket(1).document(1).update(1).other(&["GET /gapi/workspace/<Team>"]), 1)]
#[case::object_cp_into(Object(ObjectVerb::Cp), json!({ "board": "Plans", "object": ["<Plans.1>"], "into": "Ideas" }), Setup::Write, sent().tree(2).channel(2).socket(3).document(3).update(1), sent().channel(2).socket(3).document(3).update(1), 2)]
#[case::object_group(Object(ObjectVerb::Group), json!({ "board": "Plans", "object": ["<Plans.1>", "<Plans.2>"] }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::object_lock(Object(ObjectVerb::Lock), json!({ "board": "Plans", "object": ["<Plans.1>"] }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::object_unlock(Object(ObjectVerb::Unlock), json!({ "board": "Plans", "all": true }), Setup::Locked, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::object_align(Object(ObjectVerb::Align), json!({ "board": "Plans", "object": ["<Plans.1>", "<Plans.2>"], "side": "left" }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::object_rm(Object(ObjectVerb::Rm), json!({ "board": "Plans", "object": ["<Plans.1>"], "confirm": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1), sent().channel(1).socket(1).document(1).update(1), 1)]
#[case::object_rm_dry(Object(ObjectVerb::Rm), json!({ "board": "Plans", "object": ["<Plans.1>"], "dry_run": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1), sent().channel(1).socket(1).document(1), 1)]
#[tokio::test]
async fn object_pins(
    #[case] name: CommandName,
    #[case] given: Value,
    #[case] setup: Setup,
    #[case] cold: Sent,
    #[case] warm: Sent,
    #[case] later: usize,
) -> holst_core::Result<()> {
    pins(name, given, setup, cold, warm, later).await
}

#[rstest]
#[case::comment_add(Comment(CommentVerb::Add), json!({ "board": "Plans", "text": "Is this final?" }), Setup::Write, sent().tree(2).socket(1).document(1).update(1), sent().socket(1).document(1).update(1), 1)]
#[case::comment_add_object(Comment(CommentVerb::Add), json!({ "board": "Plans", "text": "Is this final?", "object": "<Plans.1>" }), Setup::Write, sent().tree(2).channel(1).socket(2).document(2).update(1), sent().channel(1).socket(2).document(2).update(1), 1)]
#[case::comment_reply(Comment(CommentVerb::Reply), json!({ "board": "Plans", "thread": "t1", "text": "Yes" }), Setup::Write, sent().tree(2).socket(1).document(1).update(1), sent().socket(1).document(1).update(1), 1)]
#[case::comment_resolve(Comment(CommentVerb::Resolve), json!({ "board": "Plans", "thread": "t1" }), Setup::Write, sent().tree(2).socket(1).document(1).update(1), sent().socket(1).document(1).update(1), 1)]
#[case::comment_edit(Comment(CommentVerb::Edit), json!({ "board": "Plans", "thread": "t1", "color": "red10" }), Setup::Write, sent().tree(2).socket(1).document(1).update(1), sent().socket(1).document(1).update(1), 1)]
#[case::comment_rm(Comment(CommentVerb::Rm), json!({ "board": "Plans", "thread": "t1", "confirm": true }), Setup::Write, sent().tree(2).socket(1).document(1).update(1), sent().socket(1).document(1).update(1), 1)]
#[case::comment_rm_dry(Comment(CommentVerb::Rm), json!({ "board": "Plans", "thread": "t1", "dry_run": true }), Setup::Write, sent().tree(2).socket(1).document(1), sent().socket(1).document(1), 1)]
#[case::comment_read(Comment(CommentVerb::Read), json!({ "board": "Plans", "thread": "t1" }), Setup::Write, sent().tree(2).socket(1).document(1).other(&["POST /gapi/comment/set_read_at"]), sent().socket(1).document(1).other(&["POST /gapi/comment/set_read_at"]), 1)]
#[tokio::test]
async fn comment_pins(
    #[case] name: CommandName,
    #[case] given: Value,
    #[case] setup: Setup,
    #[case] cold: Sent,
    #[case] warm: Sent,
    #[case] later: usize,
) -> holst_core::Result<()> {
    pins(name, given, setup, cold, warm, later).await
}

#[rstest]
#[case::template_ls(Template(TemplateVerb::Ls), json!({}), Setup::Read, sent().other(&["GET /gapi/board_template/personal"]), sent().other(&["GET /gapi/board_template/personal"]), 0)]
#[case::template_ls_workspace(Template(TemplateVerb::Ls), json!({ "view": "workspace" }), Setup::Read, sent().other(&["GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), sent().other(&["GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), 0)]
#[case::template_ls_standard(Template(TemplateVerb::Ls), json!({ "view": "standard" }), Setup::Read, sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard"]), sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard"]), 0)]
#[case::template_ls_categories(Template(TemplateVerb::Ls), json!({ "view": "categories" }), Setup::Read, sent().other(&["GET /gapi/board_template/categories/standard"]), sent().other(&["GET /gapi/board_template/categories/standard"]), 0)]
#[case::template_show(Template(TemplateVerb::Show), json!({ "template": "Retro" }), Setup::Read, sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), 0)]
#[case::template_create(Template(TemplateVerb::Create), json!({ "board": "Plans", "name": "Mine" }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).other(&["POST /gapi/board_template/create", "POST /gapi/upload_file"]), sent().channel(1).socket(1).document(1).other(&["POST /gapi/board_template/create", "POST /gapi/upload_file"]), 1)]
#[case::template_create_for_workspace(Template(TemplateVerb::Create), json!({ "board": "Plans", "name": "Mine", "for": "workspace", "workspace": "Team" }), Setup::Write, sent().current_user(1).tree(1).channel(1).socket(1).document(1).other(&["POST /gapi/board_template/create", "POST /gapi/upload_file"]), sent().current_user(1).channel(1).socket(1).document(1).other(&["POST /gapi/board_template/create", "POST /gapi/upload_file"]), 1)]
#[case::template_edit(Template(TemplateVerb::Edit), json!({ "template": "Retro", "body": "{\"name\":\"Retro 2\"}", "confirm": true }), Setup::Write, sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>", "PATCH /gapi/board_template/update"]), sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>", "PATCH /gapi/board_template/update"]), 0)]
#[case::template_edit_dry(Template(TemplateVerb::Edit), json!({ "template": "Retro", "body": "{\"name\":\"Retro 2\"}", "dry_run": true }), Setup::Write, sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), 0)]
#[case::template_rm(Template(TemplateVerb::Rm), json!({ "template": "Retro", "confirm": true }), Setup::Write, sent().other(&["DELETE /gapi/board_template/delete", "GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), sent().other(&["DELETE /gapi/board_template/delete", "GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), 0)]
#[case::template_rm_dry(Template(TemplateVerb::Rm), json!({ "template": "Retro", "dry_run": true }), Setup::Write, sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), sent().other(&["GET /gapi/board_template/categories/retros/standard_templates", "GET /gapi/board_template/categories/standard", "GET /gapi/board_template/get", "GET /gapi/board_template/personal", "GET /gapi/board_template/workspace/<Other>", "GET /gapi/board_template/workspace/<Team>"]), 0)]
#[case::backup_ls(Backup(BackupVerb::Ls), json!({ "board": "Plans" }), Setup::Read, sent().tree(2).other(&["GET /gapi/backup/list"]), sent().other(&["GET /gapi/backup/list"]), 0)]
#[case::backup_get(Backup(BackupVerb::Get), json!({ "key": "snap-1" }), Setup::Read, sent().other(&["GET /gapi/backup/download"]), sent().other(&["GET /gapi/backup/download"]), 0)]
#[case::backup_restore(Backup(BackupVerb::Restore), json!({ "board": "Plans", "key": "snap-1", "confirm": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).update(1).other(&["GET /gapi/backup/download", "GET /gapi/backup/list"]), sent().channel(1).socket(1).document(1).update(1).other(&["GET /gapi/backup/download", "GET /gapi/backup/list"]), 1)]
#[case::backup_restore_dry(Backup(BackupVerb::Restore), json!({ "board": "Plans", "key": "snap-1", "dry_run": true }), Setup::Write, sent().tree(2).channel(1).socket(1).document(1).other(&["GET /gapi/backup/download", "GET /gapi/backup/list"]), sent().channel(1).socket(1).document(1).other(&["GET /gapi/backup/download", "GET /gapi/backup/list"]), 1)]
#[tokio::test]
async fn template_and_backup_pins(
    #[case] name: CommandName,
    #[case] given: Value,
    #[case] setup: Setup,
    #[case] cold: Sent,
    #[case] warm: Sent,
    #[case] later: usize,
) -> holst_core::Result<()> {
    pins(name, given, setup, cold, warm, later).await
}

/// `channel/{id}` reads of one `object edit` that replaces or extracts pages.
async fn edit_channel_reads(
    objects: Vec<FixtureObject>,
    given: impl FnOnce(&str) -> Value,
) -> holst_core::Result<usize> {
    let seed = Fixture::default()
        .workspace("Team")
        .board_with_objects("Plans", "Team", objects)
        .page_count("spec.pdf", 3, true);
    let (fake, client) = started(seed).await?;
    let first = fake.object_id("Plans", 1).unwrap_or_default();
    let before = fake.requests().len();
    change::edit(&client, &CacheSettings::default(), &args(&given(&first))?).await?;
    let sent = since(&fake, before);
    assert!(
        sent.iter().any(|each| each.starts_with("WS Update")),
        "{sent:?}"
    );
    Ok(channel_reads(&sent, fake.id_of("Plans").as_deref()))
}

#[tokio::test]
async fn edit_path_reads_the_board_once() -> holst_core::Result<()> {
    let scratch = Scratch::new()?;
    fs::create_dir_all(&scratch.dir)?;
    let picture = scratch.dir.join("new.png");
    fs::write(&picture, holst_core::fake::png(8, 8))?;
    let reads = edit_channel_reads(vec![FixtureObject::Image("old.png".into())], |first| {
        json!({ "board": "Plans", "object": [first], "path": picture, "confirm": true })
    })
    .await?;
    assert_eq!(reads, 1);
    Ok(())
}

#[tokio::test]
async fn edit_extract_reads_the_board_once() -> holst_core::Result<()> {
    let counted = FixtureObject::File("spec.pdf".into(), 1000)
        .set("pagesInfo", json!({ "valid": true, "count": 3 }));
    let reads = edit_channel_reads(vec![counted], |first| {
        json!({ "board": "Plans", "object": [first], "extract": "1-2", "confirm": true })
    })
    .await?;
    assert_eq!(reads, 1);
    Ok(())
}
