//! The routes of users, auth extras and workspaces, called over HTTP.
#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use std::io::{self, Read as _};

use futures_util::{SinkExt as _, StreamExt as _};
use holst_core::fake::{
    ACCEPTED_COOKIES,
    Fake,
    Fixture,
    FixtureObject,
    FixtureThread,
    LOGIN_CODE,
    Right,
    ThreadAnchor,
    WorkspaceRole,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest as _};
use yrs::{Doc, Map as _, ReadTxn as _, StateVector, Transact as _};

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Fixture(#[from] holst_core::fake::FixtureError),
    #[error(transparent)]
    Socket(#[from] tungstenite::Error),
    #[error("the reply has no {0}")]
    Malformed(&'static str),
}

type Outcome<T> = Result<T, Failure>;

struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    fn text(&self, pointer: &str) -> Option<&str> {
        self.body.pointer(pointer).and_then(Value::as_str)
    }

    fn count(&self, pointer: &str) -> usize {
        self.body
            .pointer(pointer)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    }
}

async fn send(
    fake: &Fake,
    cookie: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Outcome<Reply> {
    let address = fake.base_url().trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).await?;
    let request = format!(
        "{method} /gapi/{path} HTTP/1.1\r\nhost: fake\r\ncookie: {cookie}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    let mut text = String::new();
    stream.read_to_string(&mut text).await?;
    let (head, payload) = text
        .split_once("\r\n\r\n")
        .ok_or(Failure::Malformed("body"))?;
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(Failure::Malformed("status"))?;
    Ok(Reply {
        status,
        // An empty body is a reply of its own: the read mark answers so.
        body: Some(payload)
            .filter(|raw| !raw.is_empty())
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null),
    })
}

async fn call(fake: &Fake, method: &str, path: &str, body: &str) -> Outcome<Reply> {
    send(fake, ACCEPTED_COOKIES, method, path, body).await
}

async fn get(fake: &Fake, path: &str) -> Outcome<Reply> {
    call(fake, "GET", path, "").await
}

/// The id of the `number`th workspace set up, counting from 1.
fn workspace_id(number: usize) -> String {
    format!("00000000-0000-4000-8000-{number:012}")
}

fn id_of(fake: &Fake, name: &str) -> Outcome<String> {
    fake.id_of(name).ok_or(Failure::Malformed("board"))
}

async fn favorite(fake: &Fake, id: &str, index: i64) -> Outcome<()> {
    let body = format!(r#"{{"channelId":"{id}","index":{index}}}"#);
    let added = call(fake, "POST", "favorite_channel/add", &body).await?;
    assert_eq!(added.status, 200);
    Ok(())
}

async fn open(fake: &Fake, name: &str) -> Outcome<()> {
    let body = format!(r#"{{"channelId":"{}"}}"#, id_of(fake, name)?);
    assert_eq!(call(fake, "POST", "recent/set", &body).await?.status, 200);
    Ok(())
}

/// Duplicates `source` as `name` with the id `copy`, into the workspace `team`.
async fn duplicate(
    fake: &Fake,
    source: &str,
    (copy, name): (&str, &str),
    (team, permissions): (&str, bool),
) -> Outcome<Reply> {
    let body = format!(
        r#"{{"source_channel_id":"{source}","channel_id":"{copy}","name":"{name}","duplicate_permissions":{permissions},"parent_id":"{team}","parent_type":"workspace","order_index":1.5}}"#
    );
    call(fake, "POST", "channel/duplicate", &body).await
}

#[tokio::test]
async fn favorites_are_listed_in_order_and_removed() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .workspace("Ops")
            .board("Plan", "Team", None)
            .board("Notes", "Team", None)
            .board("Rota", "Ops", None),
    )
    .await?;
    let (plan, notes, rota) = (
        id_of(&fake, "Plan")?,
        id_of(&fake, "Notes")?,
        id_of(&fake, "Rota")?,
    );
    let team = workspace_id(1);
    assert_eq!(
        get(&fake, &format!("favorite_channel/channels/{team}"))
            .await?
            .body,
        Value::Array(Vec::new())
    );
    favorite(&fake, &plan, -1).await?;
    favorite(&fake, &notes, 0).await?;
    favorite(&fake, &rota, -1).await?;
    let listed = get(&fake, &format!("favorite_channel/channels/{team}")).await?;
    assert_eq!(listed.count(""), 2);
    assert_eq!(listed.text("/0/name"), Some("Notes"));
    assert_eq!(listed.text("/1/name"), Some("Plan"));
    assert_eq!(fake.favorites_of("Team"), ["Notes", "Plan"]);
    let removed = call(
        &fake,
        "DELETE",
        "favorite_channel/delete",
        &format!(r#"{{"channelId":"{notes}"}}"#),
    )
    .await?;
    assert_eq!(removed.status, 200);
    assert_eq!(fake.favorites_of("Team"), ["Plan"]);
    let unknown = call(
        &fake,
        "DELETE",
        "favorite_channel/delete",
        r#"{"channelId":"nowhere"}"#,
    )
    .await?;
    assert_eq!(unknown.status, 404);
    let elsewhere = get(&fake, "favorite_channel/channels/nowhere").await?;
    assert_eq!(elsewhere.status, 404);
    Ok(())
}

#[tokio::test]
async fn recent_boards_cross_workspaces_and_come_in_pages() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .workspace("Ops")
            .boards(3, "Team")
            .board("Rota", "Ops", None),
    )
    .await?;
    assert_eq!(
        get(&fake, "recent/channels/page?pageSize=5")
            .await?
            .count("/channels"),
        0
    );
    open(&fake, "Board 1").await?;
    open(&fake, "Rota").await?;
    open(&fake, "Board 3").await?;
    open(&fake, "Board 1").await?;
    assert_eq!(fake.recent_of(), ["Board 1", "Board 3", "Rota"]);
    let first = get(&fake, "recent/channels/page?pageSize=2").await?;
    assert_eq!(first.count("/channels"), 2);
    assert_eq!(
        first.text("/channels/0/channelId"),
        Some(id_of(&fake, "Board 1")?.as_str())
    );
    assert!(first.text("/channels/0/date") > first.text("/channels/1/date"));
    let cursor = first.text("/cursor").ok_or(Failure::Malformed("cursor"))?;
    let second = get(
        &fake,
        &format!("recent/channels/page?pageSize=2&cursor={cursor}"),
    )
    .await?;
    assert_eq!(second.count("/channels"), 1);
    assert_eq!(
        second.text("/channels/0/channelId"),
        Some(id_of(&fake, "Rota")?.as_str())
    );
    assert_eq!(second.body.pointer("/cursor"), Some(&Value::Null));
    let unknown =
        call(&fake, "POST", "recent/set", r#"{"channelId":"nowhere"}"#).await?;
    assert_eq!(unknown.status, 404);
    Ok(())
}

#[tokio::test]
async fn an_invited_board_has_another_owner_and_can_be_left() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .member("Ann", "Team", WorkspaceRole::Member)
            .board("Mine", "Team", None)
            .invited_board("Shared", "Team", "Ann")
            .board_owned_by("Theirs", "Team", "Bob"),
    )
    .await?;
    let team = get(&fake, &format!("workspace/{}", workspace_id(1))).await?;
    assert_eq!(team.count("/users"), 3);
    let user = team.text("/users/0/id");
    let ann = team.text("/users/1/id");
    let bob = team.text("/users/2/id");
    let owner = async |name: &str| -> Outcome<Option<String>> {
        let board = get(&fake, &format!("channel/{}", id_of(&fake, name)?)).await?;
        Ok(board.text("/owner_id").map(str::to_owned))
    };
    assert_eq!(owner("Mine").await?.as_deref(), user);
    assert_eq!(owner("Shared").await?.as_deref(), ann);
    assert_eq!(owner("Theirs").await?.as_deref(), bob);
    let leave = |name: &str| {
        let path = format!("channel/{}/leave", fake.id_of(name).unwrap_or_default());
        let fake = &fake;
        async move { call(fake, "POST", &path, "").await }
    };
    assert_eq!(leave("Mine").await?.status, 403);
    assert_eq!(leave("Theirs").await?.status, 403);
    assert_eq!(leave("Shared").await?.status, 200);
    let names: Vec<_> = fake
        .tree_of("Team")
        .into_iter()
        .map(|node| node.name)
        .collect();
    assert_eq!(names, ["Mine", "Theirs"]);
    assert_eq!(leave("Shared").await?.status, 404);
    Ok(())
}

#[tokio::test]
async fn a_moved_node_takes_the_place_the_client_sends() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("First", "Team", None)
            .board("Second", "Team", None),
    )
    .await?;
    let second = id_of(&fake, "Second")?;
    let path = format!("channel/{second}/move");
    assert_eq!(
        call(&fake, "POST", &path, r#"{"newParentId":null}"#)
            .await?
            .status,
        400
    );
    let body = r#"{"newParentId":null,"newIndex":-17345.5}"#;
    assert_eq!(call(&fake, "POST", &path, body).await?.status, 200);
    let tree = get(&fake, &format!("workspace/{}/tree", workspace_id(1))).await?;
    let place = |id: &str| {
        tree.body
            .pointer("/channels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|node| node.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|node| node.get("order_index"))
            .and_then(Value::as_f64)
    };
    assert_eq!(place(&second), Some(-17345.5));
    assert!(place(&id_of(&fake, "First")?).is_some_and(|first| first >= 0.0));
    Ok(())
}

#[tokio::test]
async fn a_restore_without_a_json_body_is_refused() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Gone", "Team", None),
    )
    .await?;
    let id = id_of(&fake, "Gone")?;
    let deleted = call(
        &fake,
        "POST",
        &format!("channel/{id}/delete"),
        r#"{"permanently":false}"#,
    )
    .await?;
    assert_eq!(deleted.status, 200);
    let path = format!("channel/restore/{id}");
    assert_eq!(call(&fake, "POST", &path, "").await?.status, 400);
    assert_eq!(call(&fake, "POST", &path, "{}").await?.status, 200);
    Ok(())
}

#[tokio::test]
async fn a_duplicate_copies_the_roles_only_when_asked() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None),
    )
    .await?;
    let plan = id_of(&fake, "Plan")?;
    let team = workspace_id(1);
    let role = format!(r#"{{"channel_id":"{plan}","user_id":"ann","role":"editor"}}"#);
    assert_eq!(
        call(&fake, "POST", "channel_user/set", &role).await?.status,
        200
    );
    let with = "00000000-0000-4000-8000-00000000c001";
    let made = duplicate(&fake, &plan, (with, "With roles"), (&team, true)).await?;
    assert_eq!(made.status, 200);
    assert_eq!(made.text("/result/id"), Some(with));
    assert_eq!(
        made.text("/result/owner_id"),
        Some("00000000-0000-4000-8000-000000000000")
    );
    let without = "00000000-0000-4000-8000-00000000c002";
    let bare =
        duplicate(&fake, &plan, (without, "Without roles"), (&team, false)).await?;
    assert_eq!(bare.status, 200);
    assert_eq!(fake.role_of("Plan", "ann").as_deref(), Some("editor"));
    assert_eq!(fake.role_of("With roles", "ann").as_deref(), Some("editor"));
    assert_eq!(fake.role_of("Without roles", "ann"), None);
    Ok(())
}

#[tokio::test]
async fn the_user_workspaces_carry_organization_and_role() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace_in("Team", "Acme")
            .workspace("Ops")
            .user_role("Ops", WorkspaceRole::Member),
    )
    .await?;
    let user = get(&fake, "current_user").await?;
    assert_eq!(user.text("/workspaces/0/organization/name"), Some("Acme"));
    assert_eq!(user.text("/workspaces/0/role"), Some("admin"));
    assert_eq!(
        user.body.pointer("/workspaces/1/organization"),
        Some(&Value::Null)
    );
    assert_eq!(user.text("/workspaces/1/role"), Some("member"));
    assert_eq!(user.text("/email"), Some("fake.user@example.test"));
    Ok(())
}

#[tokio::test]
async fn members_are_listed_with_roles_counts_and_last_activity() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .member("Ann", "Team", WorkspaceRole::Admin)
            .member("Bob", "Team", WorkspaceRole::Member)
            .member("Eve", "Team", WorkspaceRole::Guest),
    )
    .await?;
    let id = workspace_id(1);
    let shown = get(&fake, &format!("workspace/{id}")).await?;
    assert_eq!(shown.text("/name"), Some("Team"));
    assert_eq!(shown.count("/users"), 4);
    assert_eq!(shown.text("/users/1/email"), Some("ann@example.test"));
    assert_eq!(shown.text("/workspace_users/1/role"), Some("admin"));
    assert_eq!(shown.text("/workspace_users/3/role"), Some("guest"));
    assert_eq!(shown.text("/owner_user_id"), shown.text("/users/0/id"));
    assert_eq!(shown.body.pointer("/members_count"), Some(&Value::from(3)));
    let counts = get(&fake, &format!("workspace_user/{id}/count_info")).await?;
    assert_eq!(counts.body.pointer("/fullMembers"), Some(&Value::from(3)));
    assert_eq!(counts.body.pointer("/admins"), Some(&Value::from(2)));
    assert_eq!(counts.body.pointer("/guests"), Some(&Value::from(1)));
    let active = get(
        &fake,
        &format!("workspace_user/{id}/list_last_active_dates"),
    )
    .await?;
    assert_eq!(active.body.as_object().map(serde_json::Map::len), Some(4));
    let unknown = get(&fake, "workspace/nowhere").await?;
    assert_eq!(unknown.status, 404);
    Ok(())
}

#[tokio::test]
async fn joinable_workspaces_are_listed_apart_from_the_users_own() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .workspace("Ops")
            .joinable_workspace("Guild"),
    )
    .await?;
    let user = get(&fake, "current_user").await?;
    assert_eq!(user.count("/workspaces"), 2);
    let joinable = get(&fake, "current_user/joinable_workspaces").await?;
    assert_eq!(joinable.count("/workspaces"), 1);
    assert_eq!(joinable.text("/workspaces/0/name"), Some("Guild"));
    let guild = workspace_id(3);
    assert_eq!(get(&fake, &format!("workspace/{guild}")).await?.status, 404);
    Ok(())
}

#[tokio::test]
async fn a_workspace_is_created_and_renamed_by_an_owner() -> Outcome<()> {
    let fake = Fake::start(Fixture::default().workspace("Team")).await?;
    let created =
        call(&fake, "POST", "workspace/create", r#"{"name":"Studio"}"#).await?;
    assert_eq!(created.status, 200);
    let id = created.text("/id").unwrap_or_default().to_owned();
    let empty = call(&fake, "POST", "workspace/create", r#"{"name":""}"#).await?;
    assert_eq!(empty.status, 400);
    let renamed = call(
        &fake,
        "PATCH",
        &format!("workspace/{id}"),
        r#"{"name":"Crew"}"#,
    )
    .await?;
    assert_eq!(renamed.status, 200);
    let user = get(&fake, "current_user").await?;
    assert_eq!(user.text("/workspaces/1/name"), Some("Crew"));
    let missing =
        call(&fake, "PATCH", "workspace/nowhere", r#"{"name":"Crew"}"#).await?;
    assert_eq!(missing.status, 404);
    Ok(())
}

#[tokio::test]
async fn a_plain_member_cannot_rename_a_workspace() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .user_role("Team", WorkspaceRole::Member),
    )
    .await?;
    let id = workspace_id(1);
    let refused = call(
        &fake,
        "PATCH",
        &format!("workspace/{id}"),
        r#"{"name":"Crew"}"#,
    )
    .await?;
    assert_eq!(refused.status, 403);
    let user = get(&fake, "current_user").await?;
    assert_eq!(user.text("/workspaces/0/name"), Some("Team"));
    Ok(())
}

#[tokio::test]
async fn the_settings_pages_show_what_the_fixture_holds() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .sso("Team")
            .domain("Team", "acme.test")
            .join_request("Team", "Zoe")
            .plan("Team", "Business")
            .billing("Team", "Acme Ltd"),
    )
    .await?;
    let id = workspace_id(1);
    let sso = get(&fake, &format!("workspace/{id}/sso_settings")).await?;
    assert_eq!(sso.body.pointer("/enabled"), Some(&Value::Bool(true)));
    let domains = get(&fake, &format!("workspace_domains/{id}/list")).await?;
    assert_eq!(domains.text("/domains/0/domain"), Some("acme.test"));
    let requests =
        get(&fake, &format!("workspace/{id}/join_request?scope=all")).await?;
    assert_eq!(requests.text("/joinRequests/0/name"), Some("Zoe"));
    let billing = get(&fake, &format!("subscription/get_billing_details/{id}")).await?;
    assert_eq!(billing.text("/billingDetails/company"), Some("Acme Ltd"));
    let current = get(
        &fake,
        &format!("subscription/get_subscription_with_billing_details/{id}"),
    )
    .await?;
    assert_eq!(current.text("/subscription/plan/name"), Some("Business"));
    let plans = get(&fake, "subscription/get_available_plans").await?;
    assert_eq!(plans.count("/plans"), 4);
    let plan = get(&fake, "subscription/get_plan/plan-team").await?;
    assert_eq!(plan.text("/name"), Some("Team"));
    assert_eq!(
        get(&fake, "subscription/get_plan/plan-none").await?.status,
        404
    );
    assert_eq!(
        get(&fake, "workspace_domains/nowhere/list").await?.status,
        404
    );
    Ok(())
}

#[tokio::test]
async fn the_event_log_is_newest_first_filtered_and_paged() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .event("Team", "Ann", "board.create", "Plan")
            .event("Team", "Bob", "board.delete", "Plan")
            .event("Team", "Ann", "board.rename", "Roadmap")
            .event("Team", "Ann", "board.delete", "Roadmap"),
    )
    .await?;
    let all = get(&fake, "audit_events").await?;
    assert_eq!(all.count("/events"), 4);
    assert_eq!(all.text("/events/0/action"), Some("board.delete"));
    assert_eq!(all.text("/events/3/action"), Some("board.create"));
    assert_eq!(all.body.pointer("/cursor"), Some(&Value::Null));
    let ann = get(&fake, "audit_events?user=Ann").await?;
    assert_eq!(ann.count("/events"), 3);
    let both = get(&fake, "audit_events?user=Ann&action=board.delete").await?;
    assert_eq!(both.count("/events"), 1);
    assert_eq!(both.text("/events/0/target"), Some("Roadmap"));
    let first = get(&fake, "audit_events?pageSize=3").await?;
    assert_eq!(first.count("/events"), 3);
    assert_eq!(first.text("/cursor"), Some("3"));
    let rest = get(&fake, "audit_events?pageSize=3&cursor=3").await?;
    assert_eq!(rest.count("/events"), 1);
    assert_eq!(rest.text("/events/0/action"), Some("board.create"));
    assert_eq!(rest.body.pointer("/cursor"), Some(&Value::Null));
    Ok(())
}

#[tokio::test]
async fn the_event_log_filters_on_percent_decoded_values() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .event("Team", "Ann", "board.create", "Привет")
            .event("Team", "Ann", "board.create", "a&b")
            .event("Team", "Ann", "board.create", "Plan"),
    )
    .await?;
    let cyrillic = get(
        &fake,
        "audit_events?target=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82",
    )
    .await?;
    assert_eq!(cyrillic.count("/events"), 1);
    assert_eq!(cyrillic.text("/events/0/target"), Some("Привет"));
    let ampersand = get(&fake, "audit_events?target=a%26b").await?;
    assert_eq!(ampersand.count("/events"), 1);
    assert_eq!(ampersand.text("/events/0/target"), Some("a&b"));
    Ok(())
}

#[tokio::test]
async fn tokens_and_sessions_are_listed_without_secrets() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .access_token("Audit export")
            .access_token("CI")
            .other_session("Chrome"),
    )
    .await?;
    let tokens = get(&fake, "personal_access_token/list?statuses=active").await?;
    assert_eq!(tokens.count("/tokens"), 2);
    assert_eq!(tokens.text("/tokens/0/name"), Some("Audit export"));
    assert!(!tokens.body.to_string().contains("secret"));
    let sessions = get(&fake, "session/list").await?;
    assert_eq!(sessions.count("/sessions"), 2);
    assert_eq!(
        sessions.body.pointer("/sessions/0/current"),
        Some(&Value::Bool(true))
    );
    assert_eq!(sessions.text("/sessions/1/client"), Some("Chrome"));
    assert_eq!(
        sessions.body.pointer("/sessions/1/current"),
        Some(&Value::Bool(false))
    );
    Ok(())
}

#[tokio::test]
async fn a_logout_ends_the_server_session_and_a_wrong_code_is_refused() -> Outcome<()> {
    let fake = Fake::start(Fixture::default()).await?;
    let wrong = call(&fake, "POST", "login/with_email", r#"{"code":"000000"}"#).await?;
    assert_eq!(wrong.status, 400);
    let right = call(
        &fake,
        "POST",
        "login/with_email",
        &format!(r#"{{"code":"{LOGIN_CODE}"}}"#),
    )
    .await?;
    assert_eq!(right.status, 200);
    assert_eq!(call(&fake, "POST", "logout", "{}").await?.status, 200);
    let after = get(&fake, "current_user").await?;
    assert_eq!(after.status, 401);
    Ok(())
}

fn plan() -> Fixture {
    Fixture::default().workspace("Team").board_with_objects(
        "Plan",
        "Team",
        vec![
            FixtureObject::Frame("Q4".into()).at(0, 0).sized(900, 700),
            FixtureObject::Sticker("Buy milk".into())
                .at(150, 160)
                .sized(100, 90)
                .stacked(5)
                .inside(1),
            FixtureObject::Arrow(1, 2),
            FixtureObject::Link("https://example.com".into(), "Example".into()),
            FixtureObject::File("report.pdf".into(), 2048),
        ],
    )
}

fn field<'a>(object: &'a Value, pointer: &str) -> Option<&'a Value> {
    object.pointer(pointer)
}

#[tokio::test]
async fn a_fixture_board_holds_objects_with_place_size_and_links() -> Outcome<()> {
    let fake = Fake::start(plan()).await?;
    let objects = <[Value; 5]>::try_from(fake.objects_of("Plan"))
        .map_err(|_| Failure::Malformed("five objects"))?;
    let [frame, sticker, arrow, link, file] = &objects;
    assert_eq!(field(frame, "/type"), Some(&json!("frame")));
    assert_eq!(field(frame, "/width"), Some(&json!(900)));
    assert_eq!(
        field(sticker, "/position"),
        Some(&json!({ "x": 150, "y": 160 }))
    );
    assert_eq!(field(sticker, "/height"), Some(&json!(90)));
    assert_eq!(field(sticker, "/zIndex"), Some(&json!(5)));
    let frame_id = fake
        .object_id("Plan", 1)
        .ok_or(Failure::Malformed("frame id"))?;
    assert_eq!(field(sticker, "/parentId"), Some(&json!(frame_id)));
    let sticker_id = fake.object_id("Plan", 2).ok_or(Failure::Malformed("id"))?;
    assert_eq!(field(arrow, "/start/objectId"), Some(&json!(frame_id)));
    assert_eq!(field(arrow, "/end/objectId"), Some(&json!(sticker_id)));
    assert_eq!(field(link, "/link"), Some(&json!("https://example.com")));
    assert_eq!(field(link, "/title"), Some(&json!("Example")));
    assert_eq!(field(file, "/fileName"), Some(&json!("report.pdf")));
    assert_eq!(field(file, "/fileSize"), Some(&json!(2048)));
    let address = fake.base_url().trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).await?;
    stream
        .write_all(b"GET /storage/report.pdf HTTP/1.1\r\nhost: fake\r\nconnection: close\r\n\r\n")
        .await?;
    let mut text = String::new();
    stream.read_to_string(&mut text).await?;
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    Ok(())
}

#[tokio::test]
async fn an_edit_behind_the_clients_back_adds_objects_and_no_write() -> Outcome<()> {
    let fake = Fake::start(plan()).await?;
    fake.edit_board(
        "Plan",
        vec![
            FixtureObject::Sticker("Added".into()),
            FixtureObject::Arrow(6, 1),
        ],
    )?;
    let objects = fake.objects_of("Plan");
    assert_eq!(objects.len(), 7);
    let added = fake.object_id("Plan", 6).ok_or(Failure::Malformed("id"))?;
    assert_eq!(
        objects
            .iter()
            .filter_map(|object| field(object, "/id"))
            .next_back(),
        fake.object_id("Plan", 7).map(Value::from).as_ref()
    );
    assert!(
        objects
            .iter()
            .any(|object| field(object, "/id") == Some(&json!(added)))
    );
    assert_eq!(fake.updates_received(), 0);
    assert!(fake.edit_board("Nowhere", Vec::new()).is_err());
    Ok(())
}

fn uint(value: u64) -> Vec<u8> {
    let low = u8::try_from(value & 0x7f).unwrap_or_default();
    match value >> 7 {
        0 => vec![low],
        rest => [vec![low | 0x80], uint(rest)].concat(),
    }
}

fn string(bytes: &[u8]) -> Vec<u8> {
    [
        uint(u64::try_from(bytes.len()).unwrap_or_default()),
        bytes.to_vec(),
    ]
    .concat()
}

#[tokio::test]
async fn the_socket_counts_the_writes_of_each_board() -> Outcome<()> {
    let fake = Fake::start(plan().board("Other", "Team", None)).await?;
    let client = Doc::new();
    let extra = client.get_or_insert_map("objects");
    extra.insert(&mut client.transact_mut(), "extra", "x");
    let update = client
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    let id = fake.id_of("Plan").ok_or(Failure::Malformed("board"))?;
    let address = fake.base_url().trim_start_matches("http://");
    let mut request = format!("ws://{address}/hud/ws").into_client_request()?;
    request.headers_mut().insert(
        "cookie",
        ACCEPTED_COOKIES.parse().map_err(io::Error::other)?,
    );
    let (mut socket, _) =
        tokio_tungstenite::client_async(request, TcpStream::connect(address).await?)
            .await?;
    let message = [
        string(id.as_bytes()),
        uint(0),
        uint(1),
        uint(0),
        string(&update),
    ]
    .concat();
    socket.send(Message::binary(message)).await?;
    assert!(socket.next().await.is_some());
    assert_eq!(fake.updates_received(), 1);
    assert_eq!(fake.objects_of("Plan").len(), 6);
    Ok(())
}

/// Sends a request with a binary body and returns the status and the body of
/// the reply.
async fn exchange(
    fake: &Fake,
    (method, path): (&str, &str),
    cookie: &str,
    body: &[u8],
) -> Outcome<(u16, Vec<u8>)> {
    let address = fake.base_url().trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).await?;
    let head = format!(
        "{method} {path} HTTP/1.1\r\nhost: fake\r\ncookie: {cookie}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(&[head.as_bytes(), body].concat()).await?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await?;
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(Failure::Malformed("body"))?;
    let status = String::from_utf8_lossy(bytes.get(..split).unwrap_or_default())
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(Failure::Malformed("status"))?;
    Ok((status, bytes.get(split + 4..).unwrap_or_default().to_vec()))
}

fn multipart(file_name: &str, content: &str) -> String {
    format!(
        "--edge\r\ncontent-disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n\r\n{content}\r\n--edge--\r\n"
    )
}

#[tokio::test]
async fn a_link_preview_answers_per_fixture_page() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .link_preview("https://example.com/a?x=1", "Example")
            .failing_link_preview("https://down.test"),
    )
    .await?;
    let answered = get(
        &fake,
        "link_preview?link=https%3A%2F%2Fexample.com%2Fa%3Fx%3D1",
    )
    .await?;
    assert_eq!(answered.status, 200);
    assert_eq!(answered.text("/title"), Some("Example"));
    let unknown = get(&fake, "link_preview?link=https%3A%2F%2Fother.test").await?;
    assert_eq!((unknown.status, unknown.body), (200, json!({})));
    let failing = get(&fake, "link_preview?link=https%3A%2F%2Fdown.test").await?;
    assert_eq!(
        (failing.status, failing.body),
        (
            400,
            json!({ "errCode": "bad-request", "msg": "", "vars": null })
        )
    );
    assert_eq!(get(&fake, "link_preview").await?.status, 400);
    Ok(())
}

#[tokio::test]
async fn an_upload_is_named_by_a_key_and_the_extension_of_the_file() -> Outcome<()> {
    let fake = Fake::start(Fixture::default().workspace("Team")).await?;
    let names = [
        call(
            &fake,
            "POST",
            "upload_file?user_id=u1&key=k",
            &multipart("cat.photo.png", "x"),
        )
        .await?,
        call(
            &fake,
            "POST",
            "upload_file?user_id=u1&key=k",
            &multipart("README", "y"),
        )
        .await?,
    ]
    .map(|reply| reply.text("/name").map(str::to_owned));
    assert_eq!(
        names,
        [
            Some("1-00000001.png".to_owned()),
            Some("2-00000002".to_owned())
        ]
    );
    Ok(())
}

#[tokio::test]
async fn an_upload_records_its_board_and_a_too_big_file_is_refused() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None)
            .upload_limit(8),
    )
    .await?;
    let id = id_of(&fake, "Plan")?;
    let small = call(
        &fake,
        "POST",
        &format!("upload_file?channel_id={id}&key=k"),
        &multipart("small.txt", "tiny"),
    )
    .await?;
    assert_eq!(small.status, 200);
    let big = call(
        &fake,
        "POST",
        &format!("upload_file?channel_id={id}&key=k"),
        &multipart("big.txt", "far too big"),
    )
    .await?;
    assert_eq!(big.status, 413);
    let uploads = fake.uploads();
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        uploads.first().map(|upload| upload.file_name.as_str()),
        Some("small.txt")
    );
    assert_eq!(
        uploads.first().and_then(|upload| upload.board.clone()),
        Some(id)
    );
    Ok(())
}

#[tokio::test]
async fn an_upload_without_a_key_or_with_other_than_one_bind_is_refused() -> Outcome<()>
{
    let fake = Fake::start(Fixture::default().workspace("Team")).await?;
    let bad = |query: &'static str| async {
        Ok::<_, Failure>(
            call(&fake, "POST", query, &multipart("a.txt", "x"))
                .await?
                .status,
        )
    };
    let statuses = [
        bad("upload_file?user_id=u1").await?,
        bad("upload_file?key=k").await?,
        bad("upload_file?user_id=u1&workspace_id=w1&key=k").await?,
        bad("upload_file?user_id=u1&key=k").await?,
        bad("upload_file?workspace_id=w1&key=k").await?,
    ];
    assert_eq!(
        (statuses, fake.uploads().len()),
        ([400, 400, 400, 200, 200], 2)
    );
    Ok(())
}

#[tokio::test]
async fn an_uploaded_update_is_downloaded_without_a_session() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None),
    )
    .await?;
    let id = id_of(&fake, "Plan")?;
    let bytes = [0xff_u8, 0xfe, 0x00, 0x80, 0x7f];
    let path = format!("/gapi/hud/upload?documentName={id}&updateId=7");
    let stored = exchange(&fake, ("POST", &path), ACCEPTED_COOKIES, &bytes).await?;
    assert_eq!(stored.0, 200);
    assert_eq!(exchange(&fake, ("POST", &path), "", &bytes).await?.0, 401);
    let nowhere = "/gapi/hud/upload?documentName=nowhere&updateId=1";
    assert_eq!(
        exchange(&fake, ("POST", nowhere), ACCEPTED_COOKIES, &bytes)
            .await?
            .0,
        404
    );
    let link = format!("/gapi/hud/download/{id}/7?md5=x&expires=1");
    let (status, body) = exchange(&fake, ("GET", &link), "", &[]).await?;
    assert_eq!((status, body), (200, bytes.to_vec()));
    let missing = format!("/gapi/hud/download/{id}/8");
    assert_eq!(exchange(&fake, ("GET", &missing), "", &[]).await?.0, 404);
    Ok(())
}

#[tokio::test]
async fn a_linked_update_is_applied_to_the_board_document() -> Outcome<()> {
    let fake = Fake::start(plan()).await?;
    let client = Doc::new();
    let extra = client.get_or_insert_map("objects");
    extra.insert(&mut client.transact_mut(), "extra", "x");
    let update = client
        .transact()
        .encode_state_as_update_v2(&StateVector::default());
    let id = id_of(&fake, "Plan")?;
    let path = format!("/gapi/hud/upload?documentName={id}&updateId=1");
    let stored = exchange(&fake, ("POST", &path), ACCEPTED_COOKIES, &update).await?;
    assert_eq!(stored.0, 200);
    let address = fake.base_url().trim_start_matches("http://");
    let mut request = format!("ws://{address}/hud/ws").into_client_request()?;
    request.headers_mut().insert(
        "cookie",
        ACCEPTED_COOKIES.parse().map_err(io::Error::other)?,
    );
    let (mut socket, _) =
        tokio_tungstenite::client_async(request, TcpStream::connect(address).await?)
            .await?;
    let linked = |update_id: u64, link: &str| {
        [
            string(id.as_bytes()),
            uint(0),
            uint(update_id),
            uint(1),
            uint(u64::try_from(update.len()).unwrap_or_default()),
            string(link.as_bytes()),
        ]
        .concat()
    };
    socket
        .send(Message::binary(linked(2, &format!("{id}/2"))))
        .await?;
    assert!(socket.next().await.is_some());
    let before = fake.objects_of("Plan");
    socket
        .send(Message::binary(linked(1, &format!("{id}/1"))))
        .await?;
    assert!(socket.next().await.is_some());
    let after = fake.objects_of("Plan");
    assert_eq!(after.len(), before.len() + 1);
    assert!(after.contains(&json!("x")));
    Ok(())
}

#[tokio::test]
async fn the_role_on_a_board_decides_who_may_change_its_access() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board_as("Mine", "Team", Right::Full)
            .board_as("Theirs", "Team", Right::Editor)
            .board_as("Glance", "Team", Right::Viewer),
    )
    .await?;
    let (mine, theirs) = (id_of(&fake, "Mine")?, id_of(&fake, "Theirs")?);
    let shown = get(&fake, &format!("channel/{theirs}")).await?;
    assert_eq!(shown.text("/channel_user/role"), Some("editor"));
    assert_eq!(shown.text("/workspace_member_access"), Some("co_owner"));
    assert_eq!(shown.text("/who_can_copy_content"), Some("viewer"));
    let patch = r#"{"public":true,"public_access":"viewer"}"#;
    let settings = r#"{"who_can_copy_content":"editor"}"#;
    let refused = call(
        &fake,
        "PATCH",
        &format!("channel/{theirs}/sharing_settings"),
        settings,
    )
    .await?;
    assert_eq!(refused.status, 403);
    let refused_link =
        call(&fake, "PATCH", &format!("channel/{theirs}"), patch).await?;
    assert_eq!(refused_link.status, 403);
    let grant = |board: &str| {
        format!(r#"{{"channel_id":"{board}","user_id":"u-9","role":"viewer"}}"#)
    };
    assert_eq!(
        call(&fake, "POST", "channel_user/set", &grant(&theirs))
            .await?
            .status,
        403
    );
    let allowed = call(&fake, "PATCH", &format!("channel/{mine}"), patch).await?;
    assert_eq!(allowed.status, 200);
    let allowed_settings = call(
        &fake,
        "PATCH",
        &format!("channel/{mine}/sharing_settings"),
        settings,
    )
    .await?;
    assert_eq!(allowed_settings.status, 200);
    let misplaced = call(
        &fake,
        "PATCH",
        &format!("channel/{mine}/sharing_settings"),
        patch,
    )
    .await?;
    assert_eq!(misplaced.status, 400);
    assert_eq!(
        call(&fake, "POST", "channel_user/set", &grant(&mine))
            .await?
            .status,
        200
    );
    let changed = get(&fake, &format!("channel/{mine}")).await?;
    assert_eq!(changed.text("/public_access"), Some("viewer"));
    assert_eq!(changed.text("/who_can_copy_content"), Some("editor"));
    let holders = get(&fake, &format!("channel_user/users?channel_id={mine}")).await?;
    assert_eq!(holders.count("/result"), 2);
    assert_eq!(holders.text("/result/1/user_id"), Some("u-9"));
    assert_eq!(holders.count("/result/1/permits"), 2);
    assert_eq!(fake.role_of("Mine", "u-9"), Some("viewer".to_owned()));
    Ok(())
}

#[tokio::test]
async fn ownership_is_transferred_by_its_owner_with_or_without_the_subtree()
-> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .folder("Shelf", "Team")
            .board("Inside", "Team", Some("Shelf"))
            .board("Alone", "Team", None),
    )
    .await?;
    let transfer = |board: String, subtree: bool| {
        let body =
            format!(r#"{{"newOwnerUserId":"u-2","transferOwnedSubtree":{subtree}}}"#);
        (format!("channel/{board}/transfer_ownership"), body)
    };
    let (shelf_path, shelf_body) = transfer(id_of(&fake, "Shelf")?, false);
    assert_eq!(
        call(&fake, "POST", &shelf_path, &shelf_body).await?.status,
        200
    );
    assert_eq!(fake.owner_of("Shelf"), Some("u-2".to_owned()));
    assert_ne!(fake.owner_of("Inside"), Some("u-2".to_owned()));
    assert_eq!(
        call(&fake, "POST", &shelf_path, &shelf_body).await?.status,
        403
    );
    let (alone_path, alone_body) = transfer(id_of(&fake, "Alone")?, true);
    assert_eq!(
        call(&fake, "POST", &alone_path, &alone_body).await?.status,
        200
    );
    assert_eq!(fake.owner_of("Alone"), Some("u-2".to_owned()));
    let (nowhere_path, nowhere_body) = transfer("nowhere".to_owned(), false);
    assert_eq!(
        call(&fake, "POST", &nowhere_path, &nowhere_body)
            .await?
            .status,
        404
    );
    Ok(())
}

#[tokio::test]
async fn workspace_templates_are_listed_apart_and_updated() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .workspace("Ops")
            .workspace_template("Retro", "Team")
            .personal_template("Mine")
            .template_with_objects("Canvas", vec![FixtureObject::Sticker("Hi".into())]),
    )
    .await?;
    let team = get(
        &fake,
        &format!("board_template/workspace/{}", workspace_id(1)),
    )
    .await?;
    assert_eq!(team.count("/board_templates"), 1);
    assert_eq!(team.text("/board_templates/0/name"), Some("Retro"));
    let ops = get(
        &fake,
        &format!("board_template/workspace/{}", workspace_id(2)),
    )
    .await?;
    assert_eq!(ops.count("/board_templates"), 0);
    let personal = get(&fake, "board_template/personal").await?;
    assert_eq!(personal.count("/board_templates"), 2);
    assert_eq!(fake.personal_templates(), ["Mine", "Canvas"]);
    assert_eq!(fake.workspace_templates("Team"), ["Retro"]);
    let canvas = fake.template_id("Canvas").ok_or(Failure::Malformed("id"))?;
    let shown = get(
        &fake,
        &format!("board_template/get?board_template_id={canvas}"),
    )
    .await?;
    let objects: Value = serde_json::from_str(
        shown.text("/data/data").ok_or(Failure::Malformed("data"))?,
    )?;
    assert_eq!(objects.as_array().map(Vec::len), Some(1));
    let change = r#"{"name":"Board","description":"Plain"}"#;
    let updated = call(
        &fake,
        "PATCH",
        &format!("board_template/update?board_template_id={canvas}"),
        change,
    )
    .await?;
    assert_eq!(updated.status, 200);
    assert_eq!(updated.text("/description"), Some("Plain"));
    assert_eq!(fake.personal_templates(), ["Mine", "Board"]);
    let unknown = call(
        &fake,
        "PATCH",
        "board_template/update?board_template_id=x",
        "{}",
    )
    .await?;
    assert_eq!(unknown.status, 404);
    let shared = format!(
        r#"{{"name":"Plan","scope":"workspace","workspace_id":"{}"}}"#,
        workspace_id(2)
    );
    assert_eq!(
        call(&fake, "POST", "board_template/create", &shared)
            .await?
            .status,
        200
    );
    assert_eq!(fake.workspace_templates("Ops"), ["Plan"]);
    let upload = call(
        &fake,
        "POST",
        "upload_file?user_id=u1&key=k",
        &multipart("cover.jpg", "jpeg"),
    )
    .await?;
    assert_eq!(upload.status, 200);
    assert_eq!(fake.thumbnails().len(), 1);
    Ok(())
}

/// The status and the body bytes of a GET of `path`, whatever the body is.
async fn fetch(fake: &Fake, cookie: &str, path: &str) -> Outcome<(u16, Vec<u8>)> {
    let address = fake.base_url().trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).await?;
    let request = format!(
        "GET /gapi/{path} HTTP/1.1\r\nhost: fake\r\ncookie: {cookie}\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await?;
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(Failure::Malformed("body"))?;
    let status = String::from_utf8_lossy(bytes.get(..split).unwrap_or_default())
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(Failure::Malformed("status"))?;
    Ok((status, bytes.get(split + 4..).unwrap_or_default().to_vec()))
}

#[tokio::test]
async fn backups_are_listed_newest_first_and_downloaded_by_provider_and_key()
-> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None)
            .board("Other", "Team", None)
            .backup_made("Plan", "old", "one", ("2026-10-01T10:00:00Z", "hud"))
            .gzipped_backup("Plan", "new", "two", ("2026-10-02T10:00:00Z", "s3"))
            .backup_made("Plan", "mid", "three", ("2026-10-01T20:00:00Z", "hud")),
    )
    .await?;
    let id = id_of(&fake, "Plan")?;
    let listed = get(&fake, &format!("backup/list?name={id}")).await?;
    let keys: Vec<_> = (0..3)
        .map(|at| listed.text(&format!("/result/{at}/key")))
        .collect();
    assert_eq!(keys, [Some("new"), Some("mid"), Some("old")]);
    assert_eq!(listed.text("/result/0/server_provider_type"), Some("s3"));
    assert_eq!(listed.text("/result/2/date"), Some("2026-10-01T10:00:00Z"));
    let other = id_of(&fake, "Other")?;
    let none = get(&fake, &format!("backup/list?name={other}")).await?;
    assert_eq!(none.count("/result"), 0);
    let plain = fetch(
        &fake,
        ACCEPTED_COOKIES,
        "backup/download?key=old&server_provider_type=hud",
    )
    .await?;
    assert_eq!(plain, (200, b"one".to_vec()));
    let (status, gzip) = fetch(
        &fake,
        ACCEPTED_COOKIES,
        "backup/download?key=new&server_provider_type=hocuspocus",
    )
    .await?;
    assert_eq!(status, 200);
    assert_eq!(gzip.get(..2), Some([0x1f, 0x8b].as_slice()));
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(gzip.as_slice()).read_to_end(&mut unpacked)?;
    assert_eq!(unpacked, b"two");
    let wrong = fetch(
        &fake,
        ACCEPTED_COOKIES,
        "backup/download?key=new&server_provider_type=hud",
    )
    .await?;
    assert_eq!(wrong.0, 404);
    let missing = fetch(&fake, ACCEPTED_COOKIES, "backup/download?key=none").await?;
    assert_eq!(missing.0, 404);
    let signed_out = fetch(&fake, "", "backup/download?key=old").await?;
    assert_eq!(signed_out.0, 401);
    Ok(())
}

#[tokio::test]
async fn a_cover_is_an_upload_and_a_channel_field_and_a_reset_keeps_the_file()
-> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None),
    )
    .await?;
    let id = id_of(&fake, "Plan")?;
    assert_eq!(
        fake.cover_of("Plan").get("thumbnail_fixed"),
        Some(&json!(false))
    );
    let posted = call(
        &fake,
        "POST",
        &format!("upload_file?channel_id={id}&key=k"),
        &multipart("cover.png", "png"),
    )
    .await?;
    let stored = posted.text("/name").ok_or(Failure::Malformed("name"))?;
    let body = json!({
        "thumbnail": stored,
        "in_trash": null,
        "thumbnail_updated_at": null,
        "thumbnail_fixed": true,
        "thumbnail_color": "#ffffff",
    });
    let set = call(&fake, "PATCH", &format!("channel/{id}"), &body.to_string()).await?;
    assert_eq!(set.text("/thumbnail"), Some(stored));
    let cover = fake.cover_of("Plan");
    assert_eq!(cover.get("thumbnail"), Some(&json!(stored)));
    assert_eq!(cover.get("thumbnail_fixed"), Some(&json!(true)));
    assert_eq!(cover.get("thumbnail_color"), Some(&json!("#ffffff")));
    assert_eq!(
        fake.uploads().first().map(|file| file.stored.as_str()),
        Some(stored)
    );
    let reset = json!({
        "in_trash": null,
        "thumbnail_updated_at": null,
        "thumbnail_fixed": false,
        "thumbnail_color": null,
    });
    let after =
        call(&fake, "PATCH", &format!("channel/{id}"), &reset.to_string()).await?;
    assert_eq!(after.status, 200);
    let reset_cover = fake.cover_of("Plan");
    assert_eq!(reset_cover.get("thumbnail"), Some(&json!(stored)));
    assert_eq!(reset_cover.get("thumbnail_fixed"), Some(&json!(false)));
    assert_eq!(reset_cover.get("thumbnail_color"), Some(&Value::Null));
    let shown = get(&fake, &format!("channel/{id}")).await?;
    assert_eq!(shown.text("/thumbnail"), Some(stored));
    let empty = call(&fake, "PATCH", &format!("channel/{id}"), "{}").await?;
    assert_eq!(empty.status, 400);
    assert!(fake.cover_of("Nowhere").is_empty());
    Ok(())
}

#[tokio::test]
async fn fixture_threads_are_read_back_and_their_read_marks_are_listed() -> Outcome<()>
{
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None)
            .board("Bare", "Team", None)
            .thread(
                "Plan",
                FixtureThread::new("9YSRw", 1, "Why?")
                    .at(10.0, 20.0)
                    .colored("red")
                    .reply(2, "Because")
                    .read_at("2026-10-27T12:27:28.401Z"),
            )
            .thread(
                "Plan",
                FixtureThread::new("ZTaaJ", 2, "Done")
                    .on_object(2)
                    .resolved(),
            ),
    )
    .await?;
    let threads = fake.threads_of("Plan");
    let first = threads.first().ok_or(Failure::Malformed("first thread"))?;
    let second = threads.get(1).ok_or(Failure::Malformed("second thread"))?;
    assert_eq!(first.anchor, ThreadAnchor::Point(10.0, 20.0));
    assert_eq!(first.color.as_deref(), Some("red"));
    assert_eq!(first.messages.len(), 2);
    assert_eq!(second.anchor, ThreadAnchor::Object(2));
    assert!(second.resolved);
    assert!(fake.threads_of("Bare").is_empty());
    assert_eq!(
        fake.thread_read_at("Plan", "9YSRw").as_deref(),
        Some("2026-10-27T12:27:28.401Z")
    );
    assert_eq!(fake.thread_read_at("Plan", "ZTaaJ"), None);
    let id = id_of(&fake, "Plan")?;
    let listed = get(&fake, &format!("comment/thread_list/{id}")).await?;
    assert_eq!(listed.status, 200);
    assert_eq!(listed.count(""), 1);
    assert_eq!(listed.text("/0/threadId"), Some("9YSRw"));
    assert_eq!(listed.text("/0/channelId"), Some(id.as_str()));
    assert_eq!(listed.text("/0/readAt"), Some("2026-10-27T12:27:28.401Z"));
    assert!(listed.text("/0/userId").is_some());
    assert_eq!(get(&fake, "comment/thread_list/nowhere").await?.status, 404);
    assert!(
        Fake::start(
            Fixture::default().thread("Nowhere", FixtureThread::new("a", 3, "c"))
        )
        .await
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn a_read_mark_is_set_again_and_outlives_no_check_of_its_thread() -> Outcome<()> {
    let fake = Fake::start(
        Fixture::default()
            .workspace("Team")
            .board("Plan", "Team", None)
            .thread(
                "Plan",
                FixtureThread::new("9YSRw", 1, "Why?")
                    .read_at("2026-10-27T12:00:00.000Z"),
            ),
    )
    .await?;
    let id = id_of(&fake, "Plan")?;
    let mark = |thread: &str, time: &str| {
        format!(r#"{{"channelId":"{id}","threadId":"{thread}","readAt":"{time}"}}"#)
    };
    let again = set_read_at(&fake, &mark("9YSRw", "2026-10-27T12:30:00.000Z")).await?;
    assert_eq!((again.status, &again.body), (200, &Value::Null));
    assert_eq!(
        set_read_at(&fake, &mark("gone1", "2026-10-27T12:31:00.000Z"))
            .await?
            .status,
        200
    );
    assert_eq!(
        fake.thread_read_at("Plan", "9YSRw").as_deref(),
        Some("2026-10-27T12:30:00.000Z")
    );
    let listed = get(&fake, &format!("comment/thread_list/{id}")).await?;
    assert_eq!(listed.count(""), 2);
    let unknown = set_read_at(
        &fake,
        r#"{"channelId":"nowhere","threadId":"t","readAt":"2026-10-27T12:31:00.000Z"}"#,
    )
    .await?;
    assert_eq!(unknown.status, 404);
    assert_eq!(
        set_read_at(&fake, &format!(r#"{{"channelId":"{id}"}}"#))
            .await?
            .status,
        400
    );
    Ok(())
}

async fn set_read_at(fake: &Fake, body: &str) -> Outcome<Reply> {
    call(fake, "POST", "comment/set_read_at", body).await
}

#[tokio::test]
async fn a_template_update_names_the_template_in_the_query() -> Outcome<()> {
    let fake = Fake::start(Fixture::default().personal_template("Retro")).await?;
    let id = fake.template_id("Retro").ok_or(Failure::Malformed("id"))?;
    let body = r#"{"description":"Fresh"}"#;
    let without = call(&fake, "PATCH", "board_template/update", body).await?;
    let with = call(
        &fake,
        "PATCH",
        &format!("board_template/update?board_template_id={id}"),
        body,
    )
    .await?;
    assert_eq!(
        (
            without.status,
            with.status,
            fake.template_of("Retro")
                .and_then(|t| t.get("description").cloned())
        ),
        (400, 200, Some(json!("Fresh")))
    );
    Ok(())
}
