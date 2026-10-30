//! How many requests each command sends, counted on the fake's log.
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
    command::{
        auth,
        backup,
        board::{file, own, show::show, tree, tree_change},
        comment,
        login::connect,
        lookup,
        object::list,
        search::search,
        table::{self, CommandArgs},
        workspace,
    },
    domain::{account::Workspace, session::Session},
    effect::{
        cache::{AccountCache, CachedTree, Forget},
        settings::CacheSettings,
    },
    error::board_socket::BoardSocketError,
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
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

    fn forget(&self, account: &str, what: Forget<'_>) -> holst_core::Result<()> {
        self.disk.forget(account, what)
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

/// The fake and a client with a saved login (with its account id) and a
/// temporary account cache.
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
/// The requests that change something: anything but a read.
fn sent_changes(requests: &[String]) -> Vec<String> {
    requests
        .iter()
        .filter(|request| !request.starts_with("GET "))
        .cloned()
        .collect()
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
async fn a_warm_folder_removal_by_name_sends_only_the_delete() -> holst_core::Result<()>
{
    let (fake, client) = listed(with_folder()).await?;
    let path = channel_path(&fake, "Archive", "/delete");
    let before = fake.requests().len();
    let given = json!({ "board": "Archive", "confirm": true });
    tree_change::rm(&client, &args(&given)?).await?;
    let sent = since(&fake, before);
    assert_eq!(sent, [format!("POST {path}")]);
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

#[rstest]
#[case::create(json!({ "name": "Notes", "workspace": "Team", "folder": "Archive" }), Change::Create)]
#[case::rename(json!({ "board": "Plans", "name": "Plans2" }), Change::Rename)]
#[case::mv(json!({ "board": "Plans", "folder": "Archive" }), Change::Move)]
#[case::cp(json!({ "board": "Plans", "folder": "Archive" }), Change::Copy)]
#[case::rm(json!({ "board": "Plans" }), Change::Remove)]
#[case::rm_folder(json!({ "board": "Archive" }), Change::Remove)]
#[tokio::test]
async fn a_dry_run_sends_no_change(
    #[case] given: Value,
    #[case] change: Change,
) -> holst_core::Result<()> {
    let (fake, client) = listed(with_folder()).await?;
    let dry = Value::Object(
        given
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain([("dry_run".to_owned(), json!(true))])
            .collect(),
    );
    change.run(&client, &dry).await?;
    assert_eq!(sent_changes(&fake.requests()), Vec::<String>::new());
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

#[tokio::test]
async fn a_dry_run_create_forgets_the_tree_of_its_workspace() -> holst_core::Result<()>
{
    let (fake, client) = listed(plans()).await?;
    let given = json!({ "name": "Notes", "workspace": "Team", "dry_run": true });
    tree_change::create(&client, &args(&given)?).await?;
    assert_eq!(show_reads_trees(&fake, &client, "Plans").await?, 1);
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
    assert_eq!(
        fake.requests(),
        [
            CURRENT_USER.to_owned(),
            format!("GET /gapi/workspace/{team}/tree"),
            "WS open".to_owned(),
            format!("GET /gapi/channel/{plans}"),
            format!("WS RequestDocument {plans}"),
        ]
    );
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
