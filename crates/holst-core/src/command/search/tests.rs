#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        fixture::uuid,
        search::{NameMatch, Plan, SearchArgs, SearchHit, merged, search},
        table::parse,
    },
    domain::limit::Limit,
    remote::client::fake::{on_fake, reply, stored},
};

fn account() -> String {
    json!({
        "id": "u1",
        "name": "Ann",
        "workspaces": [
            { "id": uuid(1), "name": "easy" },
            { "id": uuid(2), "name": "team" },
        ],
    })
    .to_string()
}

fn one_workspace() -> String {
    json!({
        "id": "u1",
        "name": "Ann",
        "workspaces": [{ "id": uuid(1), "name": "easy" }],
    })
    .to_string()
}

fn easy_tree() -> String {
    json!({ "channels": [
        { "id": uuid(11), "type": "folder", "name": "Road plans" },
        { "id": uuid(12), "type": "board", "name": "Roadmap", "parent_id": uuid(11) },
        { "id": uuid(13), "type": "board", "name": "Retro" },
    ]})
    .to_string()
}

fn arguments(given: &Value) -> Result<SearchArgs> {
    parse(given.as_object().cloned().unwrap_or_default())
}

/// Runs `search` on the replies in order, without a cache; returns the hits
/// as `kind name` and the paths asked.
async fn run(given: &Value, replies: &[String]) -> Result<(Vec<String>, usize)> {
    let canned = replies
        .iter()
        .map(|body| reply(200, &[("content-type", "application/json")], body));
    let (fake, client) = on_fake(canned, Some(stored("tok", "s=1")))?;
    let found = search(&client, &arguments(given)?, None).await?;
    let lines = found
        .hits
        .iter()
        .map(|hit| match hit {
            SearchHit::Name(name) => format!(
                "{} {}{}",
                name.kind,
                name.name,
                if name.in_trash { " (trash)" } else { "" }
            ),
            SearchHit::Text(text) => format!("text {}", text.excerpt),
        })
        .collect();
    Ok((lines, fake.requests().len()))
}

fn mode_names(extra: &Value) -> Value {
    let mut given = json!({ "query": "road", "mode": "names" });
    if let (Some(given), Some(extra)) = (given.as_object_mut(), extra.as_object()) {
        given.extend(extra.clone());
    }
    given
}

#[tokio::test]
async fn a_workspace_that_is_not_yours_is_refused_with_the_list() {
    let refused = run(
        &mode_names(&json!({ "workspace": "nowhere" })),
        &[account()],
    )
    .await
    .err()
    .map(|error| error.to_string())
    .unwrap_or_default();
    assert!(
        refused.contains("easy") && refused.contains("team"),
        "{refused}"
    );
}

#[tokio::test]
async fn an_upper_case_workspace_id_finds_the_workspace() -> Result<()> {
    let id = uuid(1).to_ascii_uppercase();
    let (hits, _) = run(
        &mode_names(&json!({ "workspace": id })),
        &[one_workspace(), easy_tree()],
    )
    .await?;
    assert_eq!(hits.len(), 2, "{hits:?}");
    Ok(())
}

#[tokio::test]
async fn the_limit_keeps_the_best_and_zero_keeps_all() -> Result<()> {
    let replies = [one_workspace(), easy_tree()];
    let (one, _) = run(&mode_names(&json!({ "limit": 1 })), &replies).await?;
    let (all, _) = run(&mode_names(&json!({ "limit": 0 })), &replies).await?;
    assert_eq!((one.first(), one.len(), all.len()), (all.first(), 1, 2));
    Ok(())
}

#[rstest]
#[case(json!({ "query": "x", "mode": "names", "board": "Plans" }), "board of `search` must be left out with --mode names")]
#[case(json!({ "query": "x", "mode": "names", "kind": "sticker" }), "kind of `search` must be board, folder or any with --mode names")]
#[case(json!({ "query": "x", "mode": "board_text", "trash": true }), "trash of `search` must be left out with --mode board_text")]
#[case(json!({ "query": "x", "mode": "board_text", "kind": "folder" }), "kind of `search` must be an object kind")]
#[case(json!({ "query": "x", "board": "Plans", "kind": "folder" }), "kind of `search` must be an object kind when --board is given")]
#[case(json!({ "query": "x", "board": "Plans", "trash": true }), "trash of `search` must be left out with --board")]
fn arguments_that_do_not_fit_the_mode_are_refused_in_our_words(
    #[case] given: Value,
    #[case] wanted: &str,
) -> Result<()> {
    let args = arguments(&given)?;
    let reported = Plan::of(&args)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(reported.contains(wanted), "{given}: {reported}");
    Ok(())
}

#[test]
fn both_is_the_default_and_a_kind_picks_its_half() -> Result<()> {
    let halves = |given: Value| -> Result<(bool, bool)> {
        let args = arguments(&given)?;
        let plan = Plan::of(&args)?;
        Ok((plan.names.is_some(), plan.text.is_some()))
    };
    assert_eq!(halves(json!({ "query": "x" }))?, (true, true));
    assert_eq!(
        halves(json!({ "query": "x", "kind": "any" }))?,
        (true, true)
    );
    assert_eq!(
        halves(json!({ "query": "x", "kind": "folder" }))?,
        (true, false)
    );
    assert_eq!(
        halves(json!({ "query": "x", "kind": "sticker" }))?,
        (false, true)
    );
    assert_eq!(
        halves(json!({ "query": "x", "board": "Plans" }))?,
        (false, true)
    );
    Ok(())
}

fn name(text: &str, score: u32) -> SearchHit {
    SearchHit::Name(NameMatch {
        id: text.to_owned(),
        name: text.to_owned(),
        kind: "board",
        workspace: "easy".to_owned(),
        path: String::new(),
        in_trash: false,
        link: None,
        score,
    })
}

fn named(hits: &[SearchHit]) -> Vec<&str> {
    hits.iter()
        .filter_map(|hit| match hit {
            SearchHit::Name(found) => Some(found.name.as_str()),
            SearchHit::Text(_) => None,
        })
        .collect()
}

#[test]
fn merged_hits_are_best_first_and_cut_to_the_limit() {
    let all = merged(
        vec![name("a", 400), name("b", 1000)],
        vec![name("c", 400), name("d", 1000)],
        Limit::All,
    );
    assert_eq!(named(&all), ["b", "d", "a", "c"]);
    let two = merged(
        vec![name("a", 400), name("b", 1000)],
        vec![name("c", 400), name("d", 1000)],
        Limit::from(2),
    );
    assert_eq!(named(&two), ["b", "d"]);
}
