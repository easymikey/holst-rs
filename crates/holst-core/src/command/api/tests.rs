#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use serde_json::json;

use crate::{
    Result,
    command::{
        api::{ApiArgs, ApiReply, request},
        change::ChangeReport,
        fixture::{arguments, client, json_reply},
    },
    remote::client::fake::Fake,
};

/// The requests that were made, as `METHOD /path`.
fn asked(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.url.path()))
        .collect()
}

#[test]
fn a_method_outside_the_set_is_refused_in_our_words() {
    let error = arguments::<ApiArgs>(json!({ "method": "TRACE", "path": "/gapi/x" }))
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        error.contains("one of GET, POST, PUT, PATCH, DELETE"),
        "{error}"
    );
}

#[tokio::test]
async fn a_get_is_sent_at_once() -> Result<()> {
    let (fake, client) = client([json_reply(&json!({ "id": "u1" }))])?;
    let args = arguments::<ApiArgs>(
        json!({ "method": "GET", "path": "/gapi/x", "query": { "k": "v" } }),
    )?;
    let ApiReply { report, reply } = request(&client, &args).await?;
    assert!(matches!(report, ChangeReport::Sent));
    assert_eq!(reply, Some(json!({ "id": "u1" })));
    assert_eq!(asked(&fake), ["GET /gapi/x"]);
    assert_eq!(
        fake.requests().first().and_then(|sent| sent.url.query()),
        Some("k=v")
    );
    Ok(())
}

#[tokio::test]
async fn a_body_that_is_not_json_is_refused() -> Result<()> {
    let (fake, client) = client([])?;
    let args = arguments::<ApiArgs>(json!({
        "method": "POST", "path": "/gapi/x", "body": "{oops", "confirm": true,
    }))?;
    assert!(request(&client, &args).await.is_err());
    assert!(asked(&fake).is_empty());
    Ok(())
}

#[test]
fn a_query_item_without_a_name_is_refused() {
    assert!(
        arguments::<ApiArgs>(
            json!({ "method": "GET", "path": "/gapi/x", "query": ["=v", "k"] })
        )
        .is_err()
    );
}
