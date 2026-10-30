//! Reading boards and searching their text against the fake Holst server.

use holst_core::{
    command::{login::connect, search, table},
    domain::session::Session,
    effect::settings::CacheSettings,
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
    remote::client::Client,
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;
use serde_json::json;

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
