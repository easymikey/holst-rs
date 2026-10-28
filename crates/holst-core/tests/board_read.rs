//! Reading boards and searching their text against the fake Holst server.

use holst_core::{
    command::{board_text, login::connect, lookup},
    domain::{
        board::ObjectKind,
        board_text::{BoardTextQuery, search::ObjectFilter},
        limit::Limit,
        session::Session,
    },
    remote::client::Client,
    system::{clock::SystemClock, id::RandomIds},
};
use holst_fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject};
use secrecy::SecretString;

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
    let groceries = lookup::board_id(&client, "Groceries", None).await?;
    let search = BoardTextQuery {
        query: "idea",
        filter: ObjectFilter {
            board: None,
            kind: ObjectKind::typed(kind),
        },
        limit: Limit::All,
    };
    let found = board_text::search_text(
        vec![(groceries, "Groceries".to_owned())],
        |board| {
            let client = client.clone();
            async move {
                board_text::load_index(&client, &board, None)
                    .await
                    .map(std::sync::Arc::new)
            }
        },
        &search,
    )
    .await?;
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
