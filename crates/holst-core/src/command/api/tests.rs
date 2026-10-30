#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        api::{ApiArgs, ApiReply, refuse_admin_write, request},
        change::ChangeReport,
        fixture::{arguments, client, json_reply},
    },
    remote::client::fake::Fake,
};

const CHANNEL: &str = "/gapi/channel/b1";

/// The requests that were made, as `METHOD /path`.
fn asked(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.url.path()))
        .collect()
}

/// What `request` says to these arguments, and what the fake saw.
async fn refused(args: Value) -> Result<(Option<String>, Vec<String>)> {
    let (fake, client) = client([])?;
    let error = request(&client, &arguments::<ApiArgs>(args)?)
        .await
        .err()
        .map(|error| error.to_string());
    Ok((error, asked(&fake)))
}

#[test]
fn a_method_outside_the_set_is_refused_in_our_words() {
    let error = arguments::<ApiArgs>(json!({ "method": "TRACE", "path": "/gapi/x" }))
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(error.contains("one of GET, POST, PATCH, DELETE"), "{error}");
}

#[test]
fn put_is_no_method_of_the_api() {
    assert!(arguments::<ApiArgs>(json!({ "method": "PUT", "path": CHANNEL })).is_err());
}

#[rstest]
#[case::unknown(
    json!({ "method": "GET", "path": "/gapi/x" }),
    "holst api sends only the calls holst knows; GET /gapi/x is not one of them"
)]
#[case::money(
    json!({ "method": "GET", "path": "/gapi/subscription/get_available_plans" }),
    "holst api sends only the calls holst knows; GET /gapi/subscription/get_available_plans is not one of them"
)]
#[case::method(
    json!({ "method": "DELETE", "path": CHANNEL, "confirm": true }),
    "holst api sends only the calls holst knows; /gapi/channel/{channel_id} takes GET or PATCH, not DELETE"
)]
#[case::segment(
    json!({ "method": "GET", "path": "/gapi/channel/a%2F.." }),
    "\"a%2F..\" is not an id segment in /gapi/channel/{channel_id}"
)]
#[case::query_in_the_path(
    json!({ "method": "GET", "path": "/gapi/recent/channels/page?pageSize=3&size=3" }),
    "GET /gapi/recent/channels/page takes the query pageSize, cursor; not size"
)]
#[case::query_item(
    json!({ "method": "GET", "path": "/gapi/recent/channels/page", "query": { "pageSize": "3", "size": "3" } }),
    "GET /gapi/recent/channels/page takes the query pageSize, cursor; not size"
)]
#[case::no_body(
    json!({ "method": "POST", "path": "/gapi/channel/b1/leave", "body": "{}", "confirm": true }),
    "POST /gapi/channel/{channel_id}/leave takes no body"
)]
#[case::typed_body(
    json!({ "method": "POST", "path": "/gapi/recent/set", "body": r#"{"channel":"b1"}"#, "confirm": true }),
    "the body does not fit POST /gapi/recent/set: unknown field `channel`, expected `channelId` at line 1 column 10"
)]
#[case::missing_query_key(
    json!({ "method": "GET", "path": "/gapi/recent/channels/page", "query": { "cursor": "c" } }),
    "GET /gapi/recent/channels/page needs the query pageSize"
)]
#[case::missing_query_key_when_none_given(
    json!({ "method": "GET", "path": "/gapi/link_preview" }),
    "GET /gapi/link_preview needs the query link"
)]
#[tokio::test]
async fn a_call_outside_the_catalogue_is_refused_before_anything_is_sent(
    #[case] args: Value,
    #[case] text: &str,
) -> Result<()> {
    let (error, sent) = refused(args).await?;
    assert_eq!(error.as_deref(), Some(text));
    assert!(sent.is_empty(), "{sent:?}");
    Ok(())
}

#[rstest]
#[case::with_email("/gapi/login/with_email")]
#[case::options("/gapi/login/get_public_options")]
#[case::password("/gapi/send_temporary_password")]
#[case::logout("/gapi/logout")]
#[tokio::test]
async fn a_login_call_is_refused_and_points_to_auth_login(
    #[case] path: &str,
) -> Result<()> {
    let (fake, client) = client([])?;
    let args = arguments::<ApiArgs>(
        json!({ "method": "POST", "path": path, "body": "{}", "confirm": true }),
    )?;
    let error = request(&client, &args).await.err();
    assert_eq!(
        error.as_ref().map(ToString::to_string).as_deref(),
        Some(format!("holst api does not send login calls: POST {path}").as_str())
    );
    assert_eq!(
        error.and_then(|error| error.hint()),
        Some(crate::error::hint::Hint::LoginAgain)
    );
    assert!(asked(&fake).is_empty());
    Ok(())
}

#[tokio::test]
async fn a_known_get_is_sent_at_once() -> Result<()> {
    let (fake, client) = client([json_reply(&json!({ "id": "u1" }))])?;
    let args = arguments::<ApiArgs>(
        json!({ "method": "GET", "path": "/gapi/link_preview", "query": { "link": "v" } }),
    )?;
    let ApiReply { report, reply } = request(&client, &args).await?;
    assert!(matches!(report, ChangeReport::Sent));
    assert_eq!(reply, Some(json!({ "id": "u1" })));
    assert_eq!(asked(&fake), ["GET /gapi/link_preview"]);
    assert_eq!(
        fake.requests().first().and_then(|sent| sent.url.query()),
        Some("link=v")
    );
    Ok(())
}

#[tokio::test]
async fn a_typed_body_that_fits_is_sent_as_given() -> Result<()> {
    let (fake, client) = client([json_reply(&json!({}))])?;
    let args = arguments::<ApiArgs>(json!({
        "method": "POST", "path": "/gapi/recent/set", "confirm": true,
        "body": r#"{ "channelId": "00000000-0000-4000-8000-0000000000a3" }"#,
    }))?;
    let ApiReply { report, .. } = request(&client, &args).await?;
    assert!(matches!(report, ChangeReport::Sent));
    assert_eq!(asked(&fake), ["POST /gapi/recent/set"]);
    Ok(())
}

#[tokio::test]
async fn a_known_post_without_confirm_is_held() -> Result<()> {
    let (fake, client) = client([])?;
    let args = arguments::<ApiArgs>(
        json!({ "method": "POST", "path": "/gapi/channel/b1/leave" }),
    )?;
    let ApiReply { report, reply } = request(&client, &args).await?;
    assert_eq!(
        report
            .refusal()
            .map(|held| held.on(crate::error::hint::Surface::Cli)),
        Some(
            "sending POST /gapi/channel/b1/leave cannot be undone; confirm with --yes"
                .to_owned()
        )
    );
    assert_eq!(reply, None);
    assert!(asked(&fake).is_empty());
    Ok(())
}

#[rstest]
#[case::sharing(
    "PATCH",
    "/gapi/channel/b1/sharing_settings",
    "{}",
    "changes who can open or edit it"
)]
#[case::workspace(
    "PATCH",
    "/gapi/workspace/w1",
    r#"{"name":"N"}"#,
    "changes it for everyone in the workspace"
)]
#[tokio::test]
async fn an_admin_change_is_held_with_its_own_risk(
    #[case] method: &str,
    #[case] path: &str,
    #[case] body: &str,
    #[case] risk: &str,
) -> Result<()> {
    let (fake, client) = client([])?;
    let args =
        arguments::<ApiArgs>(json!({ "method": method, "path": path, "body": body }))?;
    let ApiReply { report, .. } = request(&client, &args).await?;
    assert_eq!(
        report
            .refusal()
            .map(|held| held.on(crate::error::hint::Surface::Cli)),
        Some(format!(
            "sending {method} {path} {risk}; confirm with --yes"
        ))
    );
    assert!(asked(&fake).is_empty());
    Ok(())
}

#[tokio::test]
async fn a_body_that_is_not_json_is_refused() -> Result<()> {
    let (fake, client) = client([])?;
    let args = arguments::<ApiArgs>(json!({
        "method": "POST", "path": "/gapi/channel/create", "body": "{oops", "confirm": true,
    }))?;
    assert!(request(&client, &args).await.is_err());
    assert!(asked(&fake).is_empty());
    Ok(())
}

#[test]
fn a_query_item_without_a_name_is_refused() {
    assert!(
        arguments::<ApiArgs>(
            json!({ "method": "GET", "path": "/gapi/current_user", "query": ["=v", "k"] })
        )
        .is_err()
    );
}

#[rstest]
#[case::sharing(
    "PATCH",
    "/gapi/channel/b1/sharing_settings",
    Some(
        "PATCH /gapi/channel/{channel_id}/sharing_settings changes access or workspace settings, which MCP does not send through `api`"
    )
)]
#[case::workspace_read("GET", "/gapi/audit_events", None)]
#[case::plain_write("POST", "/gapi/channel/b1/leave", None)]
fn mcp_sends_no_admin_change(
    #[case] method: &str,
    #[case] path: &str,
    #[case] refusal: Option<&str>,
) -> Result<()> {
    let args = arguments::<ApiArgs>(json!({ "method": method, "path": path }))?;
    assert_eq!(
        refuse_admin_write(&args)
            .err()
            .map(|error| error.to_string())
            .as_deref(),
        refusal
    );
    Ok(())
}
