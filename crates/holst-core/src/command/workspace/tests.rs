#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::{
    Error,
    Result,
    command::{
        change::ChangeReport,
        fixture::json_reply,
        table::parse,
        workspace::{
            CreateArgs,
            ListArgs,
            LogArgs,
            Member,
            PartRead,
            ShowArgs,
            ShowPart,
            create,
            list,
            log,
            members_of,
            show,
        },
    },
    effect::{board_write::WriteMode, request::QueryParameter},
    remote::client::{
        Client,
        fake::{Fake, on_fake, stored},
    },
};

fn me() -> Value {
    json!({
        "id": "u1",
        "name": "Ann",
        "workspaces": [
            { "id": "w1", "name": "easy", "organization": { "name": "Acme" } },
            { "id": "w2", "name": "Team" },
        ],
        "workspace_users": [{ "workspace_id": "w1", "role": "admin" }],
    })
}

fn roster() -> Value {
    json!({
        "id": "w1",
        "name": "easy",
        "members_count": 2,
        "users_count_by_role": { "guest": 1 },
        "users": [
            { "id": "u1", "name": "Ann", "email": "ann@x.io" },
            { "id": "u2", "name": "Bob" },
        ],
        "workspace_users": [
            { "workspace_id": "w1", "user_id": "u1", "role": "admin",
              "last_active_at": "t1" },
        ],
    })
}

fn served(replies: &[Value]) -> Result<(Arc<Fake>, Client)> {
    on_fake(replies.iter().map(json_reply), Some(stored("tok", "s=1")))
}

/// The method and path of each request, in order.
fn sent(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|request| format!("{:?} {}", request.method, request.url.path()))
        .collect()
}

fn shown_with(with: Vec<ShowPart>, user: Option<&str>) -> ShowArgs {
    ShowArgs {
        workspace: Some("w1".to_owned()),
        with,
        user: user.map(str::to_owned),
    }
}

fn arguments(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[test]
fn a_member_list_decodes_with_each_field() -> Result<()> {
    let reply = json!({
        "users": [
            { "id": "u1", "name": "Ann", "email": "ann@x.io" },
            { "id": "u2", "username": "bob" },
        ],
        "workspace_users": [
            { "user_id": "u1", "role": "admin", "last_active_at": "t1" },
            { "user_id": "u2", "role": "member" },
        ],
    });
    let member = |id: &str,
                  name: &str,
                  email: Option<&str>,
                  role: &str,
                  last_active: Option<&str>| Member {
        id: id.to_owned(),
        name: Some(name.to_owned()),
        email: email.map(str::to_owned),
        role: Some(role.to_owned()),
        last_active: last_active.map(str::to_owned),
    };
    assert_eq!(
        members_of(reply)?,
        [
            member("u1", "Ann", Some("ann@x.io"), "admin", Some("t1")),
            member("u2", "bob", None, "member", None),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn ls_gives_the_organization_and_role_of_each_workspace() -> Result<()> {
    let (fake, client) = served(&[me()])?;
    let rows = list(&client, &ListArgs { joinable: false }).await?;
    let seen: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.organization.as_deref(),
                row.role.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [("easy", Some("Acme"), Some("admin")), ("Team", None, None)]
    );
    assert_eq!(sent(&fake), ["Get /gapi/current_user"]);
    Ok(())
}

#[tokio::test]
async fn ls_joinable_reads_the_joinable_list_only() -> Result<()> {
    let (fake, client) =
        served(&[json!([{ "id": "w5", "name": "Guild", "members_count": 7 }])])?;
    let rows = list(&client, &ListArgs { joinable: true }).await?;
    assert_eq!(
        rows.iter()
            .map(|row| (row.name.as_str(), row.members))
            .collect::<Vec<_>>(),
        [("Guild", Some(7))]
    );
    assert_eq!(sent(&fake), ["Get /gapi/current_user/joinable_workspaces"]);
    Ok(())
}

#[tokio::test]
async fn show_renders_what_a_guest_reply_has() -> Result<()> {
    let nulls = json!({
        "id": "w1", "name": null, "members_count": null,
        "users": null, "workspace_users": null,
    });
    let (_, client) = served(&[me(), nulls])?;
    let shown = show(&client, &shown_with(vec![ShowPart::Members], None)).await?;
    assert_eq!(
        (
            shown.name.as_str(),
            shown.member_count,
            shown.parts.get(&ShowPart::Members)
        ),
        ("easy", None, Some(&PartRead::Members(Vec::new())))
    );
    Ok(())
}

#[tokio::test]
async fn show_fetches_each_asked_part_once() -> Result<()> {
    let (fake, client) =
        served(&[me(), roster(), json!({ "plan": 1 }), json!({ "sso": 0 })])?;
    let args = shown_with(vec![ShowPart::Sso, ShowPart::Plan, ShowPart::Sso], None);
    let shown = show(&client, &args).await?;
    let asked: Vec<_> = sent(&fake).into_iter().skip(2).collect();
    assert_eq!(
        (asked, shown.parts.keys().copied().collect::<Vec<_>>()),
        (
            vec![
                "Get /gapi/subscription/get_subscription_with_billing_details/w1"
                    .to_owned(),
                "Get /gapi/workspace/w1/sso_settings".to_owned(),
            ],
            vec![ShowPart::Plan, ShowPart::Sso]
        )
    );
    Ok(())
}

#[tokio::test]
async fn created_counts_each_member_unless_one_is_chosen() -> Result<()> {
    let (fake, client) = served(&[me(), roster(), json!(3), json!(5)])?;
    let shown = show(&client, &shown_with(vec![ShowPart::Created], None)).await?;
    let queries: Vec<_> = fake
        .requests()
        .iter()
        .skip(2)
        .map(|request| request.url.query().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        (shown.parts.get(&ShowPart::Created), queries),
        (
            Some(&PartRead::Found(json!({ "u1": 3, "u2": 5 }))),
            vec!["userId=u1".to_owned(), "userId=u2".to_owned()]
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_part_fails_alone() -> Result<()> {
    let (_, client) = served(&[me(), roster()])?;
    let shown = show(&client, &shown_with(vec![ShowPart::Plan], None)).await?;
    assert!(matches!(
        shown.parts.get(&ShowPart::Plan),
        Some(PartRead::NotRead { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn a_workspace_you_do_not_have_is_refused_with_your_list() -> Result<()> {
    let (fake, client) = served(&[me()])?;
    let args = ShowArgs {
        workspace: Some("Nowhere".to_owned()),
        with: Vec::new(),
        user: None,
    };
    let refused = show(&client, &args).await.err();
    let text = refused
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        matches!(refused, Some(Error::Lookup(_)))
            && text.contains("easy")
            && text.contains("Team")
            && sent(&fake).len() == 1,
        "{text}"
    );
    Ok(())
}

#[tokio::test]
async fn create_posts_the_trimmed_name_and_refuses_a_blank_one() -> Result<()> {
    let (fake, client) = served(&[json!({ "id": "w9", "name": "Studio" })])?;
    let args = |name: &str| CreateArgs {
        name: name.to_owned(),
        dry_run: WriteMode::Send,
    };
    let done = create(&client, &args(" Studio ")).await?;
    let blank = create(&client, &args("  ")).await;
    assert_eq!(
        (
            matches!(done.report, ChangeReport::Sent),
            done.id.as_deref(),
            done.name.as_str(),
            sent(&fake),
            matches!(blank, Err(Error::Input(_)))
        ),
        (
            true,
            Some("w9"),
            "Studio",
            vec!["Post /gapi/workspace/create".to_owned()],
            true
        )
    );
    Ok(())
}

#[tokio::test]
async fn log_with_a_workspace_id_query_looks_up_no_workspace() -> Result<()> {
    let (fake, client) = served(&[me(), json!({ "events": [] })])?;
    let args = LogArgs {
        workspace: None,
        param: vec![QueryParameter {
            name: "workspaceId".to_owned(),
            value: "w9".to_owned(),
        }],
    };
    log(&client, &args).await?;
    assert_eq!(sent(&fake).len(), 1);
    Ok(())
}

#[tokio::test]
async fn log_asks_for_the_workspace_given() -> Result<()> {
    let (fake, client) = served(&[me(), json!({ "events": [] })])?;
    let args = LogArgs {
        workspace: Some("Team".to_owned()),
        param: vec![QueryParameter {
            name: "limit".to_owned(),
            value: "5".to_owned(),
        }],
    };
    log(&client, &args).await?;
    let requests = fake.requests();
    let query: Vec<_> = requests.last().map_or_else(Vec::new, |request| {
        request
            .url
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect()
    });
    assert_eq!(
        query,
        [
            ("workspaceId".to_owned(), "w2".to_owned()),
            ("limit".to_owned(), "5".to_owned())
        ]
    );
    Ok(())
}

#[test]
fn a_query_is_an_object_of_text_or_numbers() {
    let wrong = |param: &Value| {
        parse::<LogArgs>(arguments(&json!({ "param": param })))
            .err()
            .map(|error| error.to_string())
    };
    let words = "param of `workspace log` must be an object of text or number values";
    assert_eq!(
        (
            wrong(&json!(["limit=50"])).as_deref(),
            wrong(&json!({ "limit": true })).as_deref(),
            parse::<LogArgs>(Map::new())
                .ok()
                .map(|args| args.param.len())
        ),
        (Some(words), Some(words), Some(0))
    );
}
