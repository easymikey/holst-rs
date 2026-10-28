#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        board::rights::{Shared, share},
        fixture::{arguments, client, json_reply, uuid},
    },
    effect::http::HttpResponse,
    remote::client::fake::Fake,
};

/// The board's holders as the server replies: `result` with `permits`.
fn holders(rights: &[(&str, &str)]) -> HttpResponse {
    let users: Vec<Value> = rights
        .iter()
        .map(|(user, role)| {
            let permits = match *role {
                "co_owner" => json!(["read", "write", "set_permits"]),
                "editor" => json!(["read", "write", "create_channel"]),
                _ => json!(["read", "read_comments"]),
            };
            json!({
                "channel_id": uuid(13),
                "user_id": user,
                "permits": permits,
                "created_at": "2026-10-01T00:00:00Z",
            })
        })
        .collect();
    json_reply(&json!({ "result": users }))
}

fn done() -> HttpResponse {
    json_reply(&json!({}))
}

/// A board every workspace member co-owns, in the workspace.
fn open_channel() -> HttpResponse {
    json_reply(&json!({
        "id": uuid(13),
        "name": "Retro",
        "workspace_id": uuid(1),
        "workspace_member_access": "co_owner",
    }))
}

/// The signed-in user is `u1`.
fn me() -> HttpResponse {
    json_reply(&json!({ "id": "u1", "name": "Mia" }))
}

/// The workspace with the caller `u1` in it, as `role`; `admin_owns` is its
/// switch that lets admins own every board.
fn workspace_of_caller(role: &str, admin_owns: bool) -> HttpResponse {
    json_reply(&json!({
        "id": uuid(1),
        "name": "easy",
        "workspace_admin_as_channels_owner": admin_owns,
        "users": [{ "id": "u1", "name": "Mia" }],
        "workspace_users": [{ "user_id": "u1", "role": role }],
    }))
}

/// The writes that were made, as `METHOD /path` and their JSON body.
fn written(fake: &Fake) -> Vec<(String, Value)> {
    fake.requests()
        .iter()
        .filter(|request| request.method.as_str() != "GET")
        .map(|request| {
            (
                format!("{} {}", request.method, request.url.path()),
                request
                    .body
                    .as_ref()
                    .and_then(|body| serde_json::from_slice(&body.bytes).ok())
                    .unwrap_or(Value::Null),
            )
        })
        .collect()
}

/// The requests a share of the members' access to `members` makes for a
/// caller whose replies are `replies` (after the board's own).
async fn share_members(
    members: &str,
    replies: [HttpResponse; 3],
) -> Result<Vec<(String, Value)>> {
    let [me, holders, workspace] = replies;
    let (fake, client) =
        client([open_channel(), me, holders, workspace, done(), done()])?;
    share(
        &client,
        &arguments(json!({
            "board": uuid(13), "members": members, "confirm": true
        }))?,
    )
    .await?;
    Ok(written(&fake))
}

#[tokio::test]
async fn the_caller_is_made_co_owner_before_their_access_by_members_drops() -> Result<()>
{
    let sent = share_members(
        "viewer",
        [
            me(),
            holders(&[("u1", "viewer")]),
            workspace_of_caller("member", false),
        ],
    )
    .await?;
    assert_eq!(
        sent,
        vec![
            (
                "POST /gapi/channel_user/set".to_owned(),
                json!({ "channel_id": uuid(13), "user_id": "u1", "role": "co_owner" })
            ),
            (
                format!("PATCH /gapi/channel/{}", uuid(13)),
                json!({
                    "workspace_member_access": "viewer",
                    "in_trash": null,
                    "thumbnail_updated_at": null,
                })
            ),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_caller_who_keeps_their_access_gets_no_extra_step() -> Result<()> {
    let own = share_members(
        "viewer",
        [
            me(),
            holders(&[("u1", "co_owner")]),
            workspace_of_caller("member", false),
        ],
    )
    .await?;
    let admin = share_members(
        "viewer",
        [me(), holders(&[]), workspace_of_caller("admin", true)],
    )
    .await?;
    let paths = |sent: Vec<(String, Value)>| {
        sent.into_iter().map(|(line, _)| line).collect::<Vec<_>>()
    };
    let only = vec![format!("PATCH /gapi/channel/{}", uuid(13))];
    assert_eq!((paths(own), paths(admin)), (only.clone(), only));
    Ok(())
}

fn message(result: Result<Shared>) -> String {
    result
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_share_needs_a_setting_and_each_flag_takes_its_own_rights() -> Result<()> {
    let (fake, client) = client([])?;
    let nothing =
        message(share(&client, &arguments(json!({ "board": uuid(13) }))?).await);
    let commentators = message(
        share(
            &client,
            &arguments(json!({ "board": uuid(13), "copy": "commentator" }))?,
        )
        .await,
    );
    let none_to_delete = message(
        share(
            &client,
            &arguments(json!({ "board": uuid(13), "delete": "none" }))?,
        )
        .await,
    );
    assert!(nothing.contains("--link"), "{nothing}");
    assert!(
        commentators.contains("one of viewer, editor, full"),
        "{commentators}"
    );
    assert!(
        none_to_delete.contains("one of editor, full"),
        "{none_to_delete}"
    );
    assert_eq!(fake.requests().len(), 0);
    Ok(())
}
