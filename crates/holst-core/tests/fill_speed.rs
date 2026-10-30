//! Timing of a big table fill: `--release -- --ignored fill_speed`.
use holst_core::{
    command::{
        login::connect,
        object::{AddArgs, EditArgs, add, change},
        table,
    },
    domain::session::Session,
    effect::{clock::Clock as _, settings::CacheSettings},
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture, FixtureObject},
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;
use serde_json::{Value, json};

fn parsed<A: table::CommandArgs>(given: &Value) -> holst_core::Result<A> {
    table::parse(given.as_object().cloned().unwrap_or_default())
}

#[tokio::test]
#[ignore = "timing, release mode only"]
async fn fill_speed_of_49_by_49_cells_on_3000_objects() -> holst_core::Result<()> {
    let stickers = (0..3000)
        .map(|number| FixtureObject::Sticker(format!("sticker {number}")))
        .collect();
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board_with_objects("Plans", "Team", stickers),
    )
    .await?;
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
    let cache = CacheSettings::default();
    let made = json!({ "board": "Plans", "kind": "table", "rows": 50, "columns": 50 });
    let table = add::add(&client, &cache, &parsed::<AddArgs>(&made)?).await?;
    let id = table.objects.last().map(|object| object.id.clone());
    let fill = json!({
        "board": "Plans", "object": [id], "cell": "1,1,49,49", "color": "blue3",
        "confirm": true
    });
    let started = SystemClock.unix_millis();
    let filled = change::edit(&client, &cache, &parsed::<EditArgs>(&fill)?).await?;
    let elapsed = SystemClock.unix_millis().saturating_sub(started);
    assert!(
        filled.report.objects.len() > 2400 && elapsed < 2000,
        "{} objects in {elapsed} ms",
        filled.report.objects.len()
    );
    Ok(())
}
