#![expect(
    clippy::panic_in_result_fn,
    reason = "asserting is the point of a test"
)]

use secrecy::SecretString;
use serde_json::{Value, json};

use crate::{
    Result,
    domain::{
        id_source::SequentialIds,
        node::{
            CopyTarget,
            NewNode,
            NewParent,
            NodeCopy,
            NodeKind,
            Parent,
            RightsCopy,
        },
        session::Session,
        template::Template,
        url::BaseUrl,
    },
    effect::{clock::FixedClock, request::ApiRequest},
    remote::client::fake::{Fake, on_fake, reply},
};

const WORKSPACE_ID: &str = "00000000-0000-4000-8000-000000000001";

fn new_board() -> Result<NewNode> {
    Ok(NewNode {
        name: "zz".parse()?,
        kind: NodeKind::Board,
        workspace: WORKSPACE_ID.parse()?,
        parent: NewParent::Root,
    })
}

fn sent(fake: &Fake, index: usize) -> Result<Option<Value>> {
    fake.requests()
        .get(index)
        .and_then(|request| request.body.as_ref())
        .map(|payload| serde_json::from_slice(&payload.bytes))
        .transpose()
        .map_err(Into::into)
}

#[tokio::test]
async fn create_and_duplicate_are_reproducible_on_fakes() -> Result<()> {
    let (fake, client) = on_fake(
        [reply(200, &[], "{}"), reply(200, &[], "{}")],
        Some(Session::new("t".into())),
    )?;
    let client = client
        .with_clock(FixedClock {
            unix_millis: 1_792_577_145_404,
        })
        .with_ids(SequentialIds::new("id"));
    client
        .send_json(crate::remote::api::channel::create_stamped_request(
            &new_board()?,
            None,
            "me",
            &client.stamp(),
        )?)
        .await?;
    let parent = Parent::Workspace("ws-1".into());
    let target = CopyTarget {
        name: "Copy".into(),
        parent,
    };
    let duplication = NodeCopy {
        source_id: "src",
        target: &target,
        permissions: RightsCopy::Without,
    };
    client
        .send_json(crate::remote::api::channel::duplicate_stamped_request(
            &duplication,
            &client.stamp(),
        )?)
        .await?;

    let created = sent(&fake, 0)?;
    let field = |key| created.as_ref().and_then(|b| b.get(key)).cloned();
    assert_eq!(field("id"), Some(json!("id-1")));
    assert_eq!(field("author_id"), Some(json!("me")));
    assert_eq!(field("created_at"), Some(json!("2026-10-21T10:05:45.404Z")));
    assert_eq!(field("order_index"), Some(json!(1_792_577_145.404)));
    let copy = sent(&fake, 1)?;
    assert_eq!(
        copy.as_ref().and_then(|b| b.get("channel_id")),
        Some(&json!("id-2"))
    );
    Ok(())
}

#[tokio::test]
async fn a_link_preview_reply_is_typed_and_a_failed_one_is_an_error() -> Result<()> {
    let page = r#"{"title":"YouTube","faviconKey":"a.ico","embedInfo":{"type":"video","providerName":"YouTube","width":200,"height":113}}"#;
    let (_, client) = on_fake(
        [
            reply(200, &[], page),
            reply(200, &[], r#"{"title":"Example Domain"}"#),
            reply(400, &[], r#"{"errCode":"x","msg":"","vars":null}"#),
        ],
        Some(Session::new("t".into())),
    )?;
    let link = |url: &'static str| crate::remote::api::link::link_preview(&client, url);
    let (embed, plain) = (link("https://a.test").await?, link("https://b.test").await?);
    assert_eq!(
        (
            embed.title.as_deref(),
            embed.embed.and_then(|info| info.provider_name),
            plain.title.as_deref(),
            plain.embed.is_none(),
            link("https://c.test").await.is_err(),
        ),
        (
            Some("YouTube"),
            Some("YouTube".to_owned()),
            Some("Example Domain"),
            true,
            true
        )
    );
    Ok(())
}

#[test]
fn requests_are_what_the_web_client_sends() -> Result<()> {
    use crate::remote::api::{
        channel,
        comment,
        favorite,
        link,
        template,
        user,
        workspace,
    };
    let base: BaseUrl = "https://app.holst.so".parse()?;
    let wire = |request: Result<ApiRequest>| -> Result<String> {
        let request = request?;
        let body = request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(&body.bytes).into_owned());
        Ok(format!(
            "{} {}\n{}",
            request.method,
            request.url(&base)?,
            body.unwrap_or_default()
        ))
    };
    let root = Parent::Workspace("w".into());
    let folder = Parent::Folder("f".into());
    let gapi = "https://app.holst.so/gapi";
    let table = [
        (
            wire(channel::rename_request("b/1", "New"))?,
            format!(
                "PATCH {gapi}/channel/b%2F1\n{{\"in_trash\":null,\"name\":\"New\",\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(channel::move_to_request("b", &root, 3.0))?,
            format!(
                "POST {gapi}/channel/b/move\n{{\"newIndex\":3.0,\"newParentId\":null}}"
            ),
        ),
        (
            wire(channel::move_to_request("b", &folder, 3.0))?,
            format!(
                "POST {gapi}/channel/b/move\n{{\"newIndex\":3.0,\"newParentId\":\"f\"}}"
            ),
        ),
        (
            wire(template::delete_request("t 1"))?,
            format!("DELETE {gapi}/board_template/delete?board_template_id=t+1\n"),
        ),
        (
            wire(template::update_request("t 1", &Template::default()))?,
            format!("PATCH {gapi}/board_template/update?board_template_id=t+1\n{{}}"),
        ),
        (
            wire(workspace::tree_text_request("w 1"))?,
            format!("GET {gapi}/workspace/w%201/tree\n"),
        ),
        (
            wire(workspace::created_channels_count_request("w", "u"))?,
            format!("GET {gapi}/workspace/w/created_channels_count?userId=u\n"),
        ),
        (
            wire(favorite::remove_request("c"))?,
            format!("DELETE {gapi}/favorite_channel/delete\n{{\"channelId\":\"c\"}}"),
        ),
        (
            wire(user::pat_list_request())?,
            format!("GET {gapi}/personal_access_token/list?statuses=active\n"),
        ),
        (
            wire(comment::thread_list_request("b"))?,
            format!("GET {gapi}/comment/thread_list/b?channelId=b\n"),
        ),
        (
            wire(link::link_preview_request("https://e.com/путь a"))?,
            format!(
                "GET {gapi}/link_preview?link=https%3A%2F%2Fe.com%2F%25D0%25BF%25D1%2583%25D1%2582%25D1%258C%2520a\n"
            ),
        ),
    ];
    let (sent, expected): (Vec<_>, Vec<_>) = table.into_iter().unzip();
    assert_eq!(sent, expected);
    Ok(())
}

#[test]
fn restore_and_sharing_requests_carry_the_clients_keys() -> Result<()> {
    use crate::{
        protocol::sharing::{SharingPatch, WireRole},
        remote::api::{access, channel},
    };
    let base: BaseUrl = "https://app.holst.so".parse()?;
    let wire = |request: Result<ApiRequest>| -> Result<String> {
        let request = request?;
        let body = request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(&body.bytes).into_owned());
        Ok(format!(
            "{} {}\n{}",
            request.method,
            request.url(&base)?,
            body.unwrap_or_default()
        ))
    };
    let gapi = "https://app.holst.so/gapi";
    let table = [
        (
            wire(channel::restore_request("b"))?,
            format!("POST {gapi}/channel/restore/b\n{{}}"),
        ),
        (
            wire(access::set_channel_access_request(
                "b",
                SharingPatch {
                    workspace_member_access: Some(WireRole::None(())),
                    ..SharingPatch::default()
                },
            ))?,
            format!(
                "PATCH {gapi}/channel/b\n{{\"workspace_member_access\":null,\"in_trash\":null,\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(access::set_sharing_settings_request(
                "b",
                SharingPatch {
                    who_can_copy_content: Some(WireRole::Held("editor")),
                    ..SharingPatch::default()
                },
            ))?,
            format!(
                "PATCH {gapi}/channel/b/sharing_settings\n{{\"who_can_copy_content\":\"editor\",\"in_trash\":null,\"thumbnail_updated_at\":null}}"
            ),
        ),
    ];
    let (sent, expected): (Vec<_>, Vec<_>) = table.into_iter().unzip();
    assert_eq!(sent, expected);
    Ok(())
}

#[test]
fn the_password_is_sent_as_typed() -> Result<()> {
    let body = |password: &str| -> Result<Option<String>> {
        Ok(crate::remote::api::login::with_email_request(
            &"me@example.com".parse()?,
            &SecretString::from(password),
        )?
        .body
        .map(|b| String::from_utf8_lossy(&b.bytes).into_owned()))
    };
    assert_eq!(
        body(" pw ")?,
        Some(r#"{"email":"me@example.com","password":" pw "}"#.to_owned())
    );
    Ok(())
}
