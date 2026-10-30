#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use std::sync::Arc;

use rstest::rstest;
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
            JoinRequest,
            ListArgs,
            LogArgs,
            Member,
            PartRead,
            Plan,
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

#[tokio::test]
async fn the_own_list_names_the_account_from_its_one_reply() -> Result<()> {
    let (fake, client) = served(&[me()])?;
    let rows = list(&client, &ListArgs { joinable: false }).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(sent(&fake), ["Get /gapi/current_user"]);
    assert_eq!(client.account_id().as_deref(), Some("u1"));
    Ok(())
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

fn member(id: &str, name: &str, role: &str, email: Option<&str>) -> Member {
    Member {
        id: id.to_owned(),
        name: Some(name.to_owned()),
        email: email.map(str::to_owned),
        role: Some(role.to_owned()),
        owner: false,
        licence: None,
        last_active: None,
    }
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
    assert_eq!(
        members_of(reply)?,
        [
            Member {
                last_active: Some("t1".to_owned()),
                ..member("u1", "Ann", "admin", Some("ann@x.io"))
            },
            member("u2", "bob", "member", None),
        ]
    );
    Ok(())
}

fn licensed(listed: &Value) -> Value {
    json!({
        "users": [
            { "id": "u1", "name": "Ann" },
            { "id": "u2", "name": "Bob" },
            { "id": "u3", "name": "Cy" },
        ],
        "workspace_users": [
            { "user_id": "u1", "role": "admin", "role_reason": "by_create_workspace" },
            { "user_id": "u2", "role": "admin", "role_reason": "by_invite" },
            { "user_id": "u3", "role": "member", "role_reason": "by_invite" },
        ],
        "subscription_users": listed,
    })
}

#[test]
fn the_owner_is_the_admin_who_made_the_workspace() -> Result<()> {
    let owners: Vec<_> = members_of(licensed(&Value::Null))?
        .into_iter()
        .map(|each| (each.owner, each.role))
        .collect();
    assert_eq!(
        owners,
        [
            (true, Some("admin".to_owned())),
            (false, Some("admin".to_owned())),
            (false, Some("member".to_owned())),
        ]
    );
    Ok(())
}

#[test]
fn a_licence_is_held_by_the_listed_users_with_one() -> Result<()> {
    let held = members_of(licensed(&json!([
        { "user_id": "u1", "has_license": true },
        { "user_id": "u2", "has_license": false },
    ])))?;
    assert_eq!(
        held.iter().map(|each| each.licence).collect::<Vec<_>>(),
        [Some(true), Some(false), Some(false)]
    );
    Ok(())
}

#[test]
fn no_list_of_licences_is_no_licence_information() -> Result<()> {
    let unknown = members_of(licensed(&Value::Null))?;
    assert_eq!(
        unknown.iter().map(|each| each.licence).collect::<Vec<_>>(),
        [None, None, None]
    );
    Ok(())
}

async fn part_of(part: ShowPart, reply: Value) -> Result<(Vec<String>, PartRead)> {
    let (fake, client) = served(&[me(), roster(), reply])?;
    let mut shown = show(&client, &shown_with(vec![part], None)).await?;
    let read = shown.parts.remove(&part).unwrap_or(PartRead::NotRead {
        error: "missing".to_owned(),
    });
    Ok((sent(&fake).into_iter().skip(2).collect(), read))
}

fn request_from(name: &str) -> JoinRequest {
    JoinRequest {
        id: Some("r1".to_owned()),
        name: Some(name.to_owned()),
        email: Some("zoe@x.io".to_owned()),
        created: Some("t1".to_owned()),
        status: Some("pending".to_owned()),
    }
}

#[rstest]
#[case::none(json!({ "joinRequests": [] }), Vec::new())]
#[case::one(
    json!({ "joinRequests": [{
        "id": "r1", "status": "pending", "createdAt": "t1",
        "user": { "id": "u9", "name": "Zoe", "email": "zoe@x.io" },
    }] }),
    vec![request_from("Zoe")]
)]
#[tokio::test]
async fn requests_are_the_active_join_requests(
    #[case] reply: Value,
    #[case] rows: Vec<JoinRequest>,
) -> Result<()> {
    let (asked, read) = part_of(ShowPart::Requests, reply).await?;
    assert_eq!(
        (asked, read),
        (
            vec!["Get /gapi/workspace/w1/join_request".to_owned()],
            PartRead::Requests(rows)
        )
    );
    Ok(())
}

fn plan_named(name: &str) -> Plan {
    Plan {
        name: name.to_owned(),
        level: None,
        period: None,
        price: None,
        currency: None,
        trial_days: None,
    }
}

fn paid(currency: Option<&str>) -> Value {
    json!({
        "billingDetails": null,
        "brotherPlan": null,
        "subscription": {
            "currency": currency,
            "plan": {
                "id": "p1", "name": "Team", "trialPeriodInDays": 14,
                "price": 1200, "period": "Month", "level": 5,
                "brotherKey": "k",
            },
        },
    })
}

#[rstest]
#[case::free(
    json!({ "billingDetails": null, "brotherPlan": null, "subscription": null }),
    plan_named("Free"),
    None
)]
#[case::paid_without_currency(
    paid(None),
    Plan {
        level: Some(5),
        period: Some("Month".to_owned()),
        price: Some(1200),
        trial_days: Some(14),
        ..plan_named("Team")
    },
    Some("price 1200")
)]
#[case::paid_with_currency(
    paid(Some("USD")),
    Plan {
        level: Some(5),
        period: Some("Month".to_owned()),
        price: Some(1200),
        currency: Some("USD".to_owned()),
        trial_days: Some(14),
        ..plan_named("Team")
    },
    Some("12.00 USD/month")
)]
#[tokio::test]
async fn the_plan_is_typed_and_free_without_a_subscription(
    #[case] reply: Value,
    #[case] plan: Plan,
    #[case] price: Option<&str>,
) -> Result<()> {
    let (_, read) = part_of(ShowPart::Plan, reply).await?;
    assert_eq!(
        (read, plan.price_text().as_deref()),
        (PartRead::Plan(plan.clone()), price)
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
