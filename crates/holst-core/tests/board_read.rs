//! Reading boards and searching their text against the fake Holst server.
use std::path::Path;

use holst_core::{
    command::{
        board::{picture, show},
        login::connect,
        object::get,
        search,
        table::{self, CommandArgs},
        template,
    },
    domain::session::Session,
    effect::settings::CacheSettings,
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
    remote::client::Client,
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;
use serde_json::{Value, json};

async fn started() -> holst_core::Result<(Fake, Client)> {
    let seed = Fixture::default()
        .workspace("Team")
        .board_with_objects(
            "Plans",
            "Team",
            vec![
                FixtureObject::Sticker("Buy milk".into()),
                FixtureObject::Frame("Q4".into()),
            ],
        )
        .board_with_objects(
            "Groceries",
            "Team",
            vec![
                FixtureObject::Sticker("Oat milk".into()),
                FixtureObject::Shape("Milk run".into()),
                FixtureObject::MindMapNode("Idea map".into()),
            ],
        );
    let fake = Fake::start(seed).await?;
    let session = Session::with_cookies(
        SecretString::from(ACCEPTED_TOKEN),
        ACCEPTED_COOKIES.parse().unwrap_or_default(),
    );
    let client = connect(
        Some(session),
        fake.base_url().parse()?,
        SystemClock,
        RandomIds,
    )?;
    Ok((fake, client))
}

async fn found_of_kind(kind: &str) -> holst_core::Result<usize> {
    let (_fake, client) = started().await?;
    let args = table::parse(
        json!({ "query": "idea", "mode": "board_text", "board": "Groceries", "kind": kind, "limit": 0 })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    )?;
    let found = search::search(&client, &CacheSettings::default(), &args).await?;
    Ok(found.total)
}

#[tokio::test]
async fn a_kind_without_variant_filters_the_text_search() -> holst_core::Result<()> {
    assert_eq!(found_of_kind("Mind-Map-Node").await?, 1);
    assert_eq!(found_of_kind("sticker").await?, 0);
    Ok(())
}

#[tokio::test]
async fn a_kind_no_board_object_has_is_refused() -> holst_core::Result<()> {
    assert!(matches!(
        found_of_kind("kanban").await,
        Err(holst_core::Error::Input(
            holst_core::error::input::InputError::NotObjectKind { .. }
        ))
    ));
    Ok(())
}

fn args<A: CommandArgs>(given: &Value) -> holst_core::Result<A> {
    table::parse(given.as_object().cloned().unwrap_or_default())
}

fn channel_reads(fake: &Fake, board: &str) -> usize {
    let channel = format!("GET /gapi/channel/{board}");
    fake.requests()
        .iter()
        .filter(|request| **request == channel)
        .count()
}

#[expect(
    clippy::disallowed_methods,
    reason = "the cache lives in a temp directory"
)]
fn warm_cache(name: &str) -> CacheSettings {
    let root =
        std::env::temp_dir().join(format!("holst-read-{name}-{}", std::process::id()));
    CacheSettings {
        root: Some(root.clone()),
        roots: vec![root],
        host: "app.holst.so".into(),
        max_bytes: 1 << 24,
        trust_millis: 60_000,
    }
}

fn plans_id(fake: &Fake) -> holst_core::Result<String> {
    fake.id_of("Plans").ok_or(holst_core::Error::LocalData(
        holst_core::error::local_data::LocalDataError::NoHome,
    ))
}

#[tokio::test]
async fn a_show_without_a_cache_reads_the_channel_once() -> holst_core::Result<()> {
    let (fake, client) = started().await?;
    let given =
        args::<holst_core::command::board::ShowArgs>(&json!({ "board": "Plans" }))?;
    show::show(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(channel_reads(&fake, &plans_id(&fake)?), 1);
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test removes its temp directory"
)]
async fn a_show_over_a_warm_cache_reads_the_channel_once() -> holst_core::Result<()> {
    let (fake, client) = started().await?;
    let cache = warm_cache("show");
    let given =
        args::<holst_core::command::board::ShowArgs>(&json!({ "board": "Plans" }))?;
    show::show(&client, &cache, &given).await?;
    let before = channel_reads(&fake, &plans_id(&fake)?);
    show::show(&client, &cache, &given).await?;
    let after = channel_reads(&fake, &plans_id(&fake)?);
    cache
        .root
        .iter()
        .for_each(|root| drop(std::fs::remove_dir_all(root)));
    assert_eq!(after - before, 1);
    Ok(())
}

#[tokio::test]
async fn a_show_with_the_note_reads_the_channel_once() -> holst_core::Result<()> {
    let (fake, client) = started().await?;
    let given = args::<holst_core::command::board::ShowArgs>(
        &json!({ "board": "Plans", "with": ["note"] }),
    )?;
    show::show(&client, &CacheSettings::default(), &given).await?;
    assert_eq!(channel_reads(&fake, &plans_id(&fake)?), 1);
    Ok(())
}

#[tokio::test]
#[expect(clippy::disallowed_methods, reason = "the test removes its temp file")]
async fn a_snap_reads_the_channel_once() -> holst_core::Result<()> {
    let (fake, client) = started().await?;
    let file = std::env::temp_dir()
        .join(format!("holst-read-snap-{}.svg", std::process::id()));
    let given = args::<holst_core::command::board::SnapArgs>(
        &json!({ "board": "Plans", "file": file }),
    )?;
    picture::snap(&client, &CacheSettings::default(), None, &given).await?;
    drop(std::fs::remove_file(&file));
    assert_eq!(channel_reads(&fake, &plans_id(&fake)?), 1);
    Ok(())
}

#[tokio::test]
async fn a_template_for_me_reads_the_channel_once() -> holst_core::Result<()> {
    let (fake, client) = started().await?;
    let given =
        args::<template::CreateArgs>(&json!({ "board": "Plans", "name": "Mine" }))?;
    template::create(&client, &given).await?;
    assert_eq!(channel_reads(&fake, &plans_id(&fake)?), 1);
    Ok(())
}

/// A board "Plans" with one image, and a client that names its account.
async fn with_image() -> holst_core::Result<(Fake, Client)> {
    let seed = Fixture::default().workspace("Team").board_with_objects(
        "Plans",
        "Team",
        vec![FixtureObject::Image("cat.png".into())],
    );
    let fake = Fake::start(seed).await?;
    let session = Session::with_cookies(
        SecretString::from(ACCEPTED_TOKEN),
        ACCEPTED_COOKIES.parse().unwrap_or_default(),
    )
    .with_account_id("00000000-0000-4000-8000-000000000000");
    let client = connect(
        Some(session),
        fake.base_url().parse()?,
        SystemClock,
        RandomIds,
    )?;
    Ok((fake, client))
}

fn get_args(
    fake: &Fake,
    offline: bool,
) -> holst_core::Result<holst_core::command::object::GetArgs> {
    let board = plans_id(fake)?;
    let object = fake.object_id("Plans", 1).unwrap_or_default();
    args(&json!({ "board": board, "object": [object], "offline": offline }))
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test removes its temp directory"
)]
async fn an_offline_get_reads_the_board_from_the_cache() -> holst_core::Result<()> {
    let (fake, client) = with_image().await?;
    let cache = warm_cache("get");
    let dir =
        std::env::temp_dir().join(format!("holst-read-get-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let downloads = Some(dir.as_path());
    get::get(&client, &cache, downloads, &get_args(&fake, false)?).await?;
    let before = channel_reads(&fake, &plans_id(&fake)?);
    let got = get::get(&client, &cache, downloads, &get_args(&fake, true)?).await;
    let after = channel_reads(&fake, &plans_id(&fake)?);
    std::fs::remove_dir_all(&dir)?;
    cache
        .root
        .iter()
        .for_each(|root| drop(std::fs::remove_dir_all(root)));
    assert_eq!(got?.objects.len(), 1);
    assert_eq!(after, before);
    Ok(())
}

#[tokio::test]
async fn an_offline_get_without_a_cached_board_says_so() -> holst_core::Result<()> {
    let (fake, client) = with_image().await?;
    let cache = warm_cache("get-miss");
    let refused = get::get(
        &client,
        &cache,
        Some(Path::new("/nowhere")),
        &get_args(&fake, true)?,
    )
    .await
    .err()
    .map(|error| error.to_string());
    assert_eq!(
        refused.as_deref(),
        Some(
            "board 00000000-0000-4000-8000-00000000a000 is not in the cache, and --offline allows no network"
        )
    );
    Ok(())
}
