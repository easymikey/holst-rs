#![expect(
    clippy::panic_in_result_fn,
    reason = "asserting is the point of a test"
)]

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::{
    Result,
    domain::{
        id::WorkspaceId,
        id_source::SequentialIds,
        node::{
            CopyTarget,
            CreationStamp,
            DeleteMode,
            NewNode,
            NewParent,
            NodeCopy,
            NodeKind,
            Parent,
            RightsCopy,
        },
        session::Session,
        template::Template,
        time::Timestamp,
        url::BaseUrl,
    },
    effect::{clock::FixedClock, http::HttpMethod, request::ApiRequest},
    remote::{
        api::{
            CATALOGUE,
            channel::{
                CreateChannel,
                Duplicate,
                Rename,
                ResetCover,
                SetCover,
                SetPreview,
            },
        },
        catalogue::Body,
        client::fake::{Fake, on_fake, reply},
    },
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

fn wire(request: Result<ApiRequest>) -> Result<String> {
    let base: BaseUrl = "https://app.holst.so".parse()?;
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
        .send_json(crate::remote::api::channel::create_request(
            &CreateChannel::new(&new_board()?, "me", &client.stamp(), None)?,
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
        .send_json(crate::remote::api::channel::duplicate_request(
            &Duplicate::new(&duplication, &client.stamp()),
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
fn favorite_and_workspace_bodies_keep_their_wire_shape() -> Result<()> {
    use crate::remote::api::{favorite, workspace};
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
            wire(favorite::add_request(&favorite::AddFavorite {
                channel_id: "c".to_owned(),
                index: -1,
            }))?,
            format!(
                "POST {gapi}/favorite_channel/add\n{{\"channelId\":\"c\",\"index\":-1}}"
            ),
        ),
        (
            wire(workspace::rename_request(
                "w",
                &workspace::WorkspaceName {
                    name: "N".to_owned(),
                },
            ))?,
            format!("PATCH {gapi}/workspace/w\n{{\"name\":\"N\"}}"),
        ),
        (
            wire(workspace::create_request(&workspace::WorkspaceName {
                name: "N".to_owned(),
            }))?,
            format!("POST {gapi}/workspace/create\n{{\"name\":\"N\"}}"),
        ),
    ];
    let (sent, expected): (Vec<_>, Vec<_>) = table.into_iter().unzip();
    assert_eq!(sent, expected);
    Ok(())
}

#[test]
fn requests_are_what_the_web_client_sends() -> Result<()> {
    use crate::remote::api::{
        channel::{self, MoveTo, Rename},
        comment,
        favorite,
        link,
        template,
        user,
        workspace,
    };
    let root = Parent::Workspace("w".into());
    let folder = Parent::Folder("f".into());
    let gapi = "https://app.holst.so/gapi";
    let table = [
        (
            wire(channel::rename_request("b/1", &Rename::new("New")))?,
            format!(
                "PATCH {gapi}/channel/b%2F1\n{{\"in_trash\":null,\"name\":\"New\",\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(channel::move_to_request("b", &MoveTo::new(&root, 3.0)))?,
            format!(
                "POST {gapi}/channel/b/move\n{{\"newIndex\":3.0,\"newParentId\":null}}"
            ),
        ),
        (
            wire(channel::move_to_request("b", &MoveTo::new(&folder, 3.0)))?,
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
            wire(favorite::remove_request(&favorite::RemoveFavorite {
                channel_id: "c".to_owned(),
            }))?,
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
        remote::api::{
            access::{self, OwnershipTransfer, RoleGrant},
            channel::{self, Restore},
            comment::{self, SetReadAt},
        },
    };
    let gapi = "https://app.holst.so/gapi";
    let table = [
        (
            wire(channel::restore_request("b", &Restore {}))?,
            format!("POST {gapi}/channel/restore/b\n{{}}"),
        ),
        (
            wire(access::set_channel_access_request(
                "b",
                &SharingPatch {
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
                &SharingPatch {
                    who_can_copy_content: Some(WireRole::Held("editor".to_owned())),
                    ..SharingPatch::default()
                },
            ))?,
            format!(
                "PATCH {gapi}/channel/b/sharing_settings\n{{\"who_can_copy_content\":\"editor\",\"in_trash\":null,\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(access::set_member_right_request(&RoleGrant {
                channel_id: "c".to_owned(),
                user_id: "u".to_owned(),
                role: Some("editor".to_owned()),
            }))?,
            format!(
                "POST {gapi}/channel_user/set\n{{\"channel_id\":\"c\",\"user_id\":\"u\",\"role\":\"editor\"}}"
            ),
        ),
        (
            wire(access::set_member_right_request(&RoleGrant {
                channel_id: "c".to_owned(),
                user_id: "u".to_owned(),
                role: None,
            }))?,
            format!(
                "POST {gapi}/channel_user/set\n{{\"channel_id\":\"c\",\"user_id\":\"u\",\"role\":null}}"
            ),
        ),
        (
            wire(access::transfer_ownership_request(
                "b 1",
                &OwnershipTransfer {
                    new_owner_user_id: "u".to_owned(),
                    transfer_owned_subtree: true,
                },
            ))?,
            format!(
                "POST {gapi}/channel/b%201/transfer_ownership\n{{\"newOwnerUserId\":\"u\",\"transferOwnedSubtree\":true}}"
            ),
        ),
        (
            wire(comment::set_read_at_time_request(&SetReadAt {
                channel_id: "b".to_owned(),
                thread_id: "t".to_owned(),
                read_at: "2026-10-21T10:05:45.404Z".to_owned(),
            }))?,
            format!(
                "POST {gapi}/comment/set_read_at\n{{\"channelId\":\"b\",\"threadId\":\"t\",\"readAt\":\"2026-10-21T10:05:45.404Z\"}}"
            ),
        ),
    ];
    let (sent, expected): (Vec<_>, Vec<_>) = table.into_iter().unzip();
    assert_eq!(sent, expected);
    Ok(())
}

#[test]
fn cover_and_delete_requests_carry_the_clients_keys() -> Result<()> {
    use crate::remote::api::channel::{self, ResetCover, SetCover, SetPreview};
    let gapi = "https://app.holst.so/gapi";
    let table = [
        (
            wire(channel::set_cover_request("b", &SetCover::new("pic.png")))?,
            format!(
                "PATCH {gapi}/channel/b\n{{\"in_trash\":null,\"thumbnail\":\"pic.png\",\"thumbnail_color\":\"#ffffff\",\"thumbnail_fixed\":true,\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(channel::reset_cover_request("b", &ResetCover::default()))?,
            format!(
                "PATCH {gapi}/channel/b\n{{\"in_trash\":null,\"thumbnail_color\":null,\"thumbnail_fixed\":false,\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(channel::set_preview_request(
                "b",
                &SetPreview::new("auto.png"),
            ))?,
            format!(
                "PATCH {gapi}/channel/b\n{{\"in_trash\":null,\"thumbnail\":\"auto.png\",\"thumbnail_color\":null,\"thumbnail_updated_at\":null}}"
            ),
        ),
        (
            wire(channel::delete_request("b", &DeleteMode::Trash.into()))?,
            format!("POST {gapi}/channel/b/delete\n{{\"permanently\":false}}"),
        ),
        (
            wire(channel::delete_request("b", &DeleteMode::Permanent.into()))?,
            format!("POST {gapi}/channel/b/delete\n{{\"permanently\":true}}"),
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

#[rstest]
#[case::null("null")]
#[case::empty_object("{}")]
#[case::object(r#"{"ok":true}"#)]
fn an_empty_reply_is_done(#[case] reply: &str) {
    assert!(serde_json::from_str::<super::Done>(reply).is_ok());
}

#[rstest]
#[case::role(r#"{"channel_id":"c","user_id":"u","role":"viewer"}"#, true)]
#[case::revoke(r#"{"channel_id":"c","user_id":"u","role":null}"#, true)]
#[case::unknown_key(r#"{"channel_id":"c","user_id":"u","role":null,"x":1}"#, false)]
#[case::missing_user(r#"{"channel_id":"c","role":null}"#, false)]
fn a_role_grant_is_read_by_its_keys(#[case] text: &str, #[case] fits: bool) {
    use crate::remote::{api::access::RoleGrant, catalogue::check};
    assert_eq!(check::<RoleGrant>(text).is_ok(), fits);
}

#[rstest]
#[case::transfer(r#"{"newOwnerUserId":"u","transferOwnedSubtree":false}"#, true)]
#[case::snake_case(
    r#"{"new_owner_user_id":"u","transfer_owned_subtree":false}"#,
    false
)]
#[case::wrong_type(r#"{"newOwnerUserId":"u","transferOwnedSubtree":"yes"}"#, false)]
fn an_ownership_transfer_is_read_by_its_keys(#[case] text: &str, #[case] fits: bool) {
    use crate::remote::{api::access::OwnershipTransfer, catalogue::check};
    assert_eq!(check::<OwnershipTransfer>(text).is_ok(), fits);
}

#[rstest]
#[case::read(
    r#"{"channelId":"b","threadId":"t","readAt":"2026-10-21T10:05:45.404Z"}"#,
    true
)]
#[case::unknown_key(r#"{"channelId":"b","threadId":"t","readAt":"x","by":"u"}"#, false)]
#[case::missing_time(r#"{"channelId":"b","threadId":"t"}"#, false)]
fn a_read_mark_body_is_read_by_its_keys(#[case] text: &str, #[case] fits: bool) {
    use crate::remote::{api::comment::SetReadAt, catalogue::check};
    assert_eq!(check::<SetReadAt>(text).is_ok(), fits);
}

const FOLDER: &str = "00000000-0000-4000-8000-0000000000f9";

// 2026-10-21T10:05:45.404Z.
fn stamp(id: &str) -> CreationStamp {
    CreationStamp::at(id, Timestamp::from_millis(1_792_577_145_404))
}

fn new_channel(name: &str, kind: NodeKind, folder: Option<&str>) -> Result<NewNode> {
    Ok(NewNode {
        name: name.parse()?,
        kind,
        workspace: WORKSPACE_ID.parse::<WorkspaceId>()?,
        parent: match folder {
            Some(folder) => NewParent::Folder(folder.parse()?),
            None => NewParent::Root,
        },
    })
}

fn created(new: &NewNode, id: &str, root_access: Option<&str>) -> Result<Value> {
    Ok(serde_json::to_value(CreateChannel::new(
        new,
        "me",
        &stamp(id),
        root_access,
    )?)?)
}

#[rstest]
#[case::root(None)]
#[case::in_folder(Some(FOLDER))]
fn new_board_body_matches_observed_shape(#[case] folder: Option<&str>) -> Result<()> {
    let body = created(
        &new_channel("zz-test", NodeKind::Board, folder)?,
        "11111111-1111-1111-1111-111111111111",
        None,
    )?;
    // Without a suffix rstest cases share one snapshot name.
    insta::with_settings!({ snapshot_suffix => folder.map_or("root", |_| "folder-9") }, {
        insta::assert_json_snapshot!(body);
    });
    Ok(())
}

#[test]
fn a_node_that_is_neither_board_nor_folder_is_refused() -> Result<()> {
    let new = new_channel("Sticker", NodeKind::Other, None)?;
    assert!(CreateChannel::new(&new, "me", &stamp("id"), None).is_err());
    Ok(())
}

#[rstest]
#[case::board(NodeKind::Board)]
#[case::folder(NodeKind::Folder)]
fn a_channel_in_a_folder_inherits_its_access(#[case] kind: NodeKind) -> Result<()> {
    let body = created(
        &new_channel("Plans", kind, Some(FOLDER))?,
        "id",
        Some("editor"),
    )?;
    assert_eq!(body.get("workspace_member_access"), Some(&json!(null)));
    assert_eq!(body.get("parent_channel_access"), Some(&json!("inherit")));
    Ok(())
}

#[rstest]
#[case::private(None, json!(null))]
#[case::shared(Some("editor"), json!("editor"))]
fn a_channel_at_the_root_takes_the_workspace_default(
    #[case] default: Option<&str>,
    #[case] sent: Value,
) -> Result<()> {
    let body = created(&new_channel("Plans", NodeKind::Board, None)?, "id", default)?;
    assert_eq!(body.get("workspace_member_access"), Some(&sent));
    assert_eq!(body.get("parent_channel_access"), Some(&json!(null)));
    Ok(())
}

#[test]
fn duplicate_body_names_parent_and_copy() -> Result<()> {
    let target = CopyTarget {
        name: "Copy".into(),
        parent: Parent::Folder("f".into()),
    };
    let copy = NodeCopy {
        source_id: "src",
        target: &target,
        permissions: RightsCopy::With,
    };
    let at = CreationStamp::at("new", Timestamp::from_millis(2_500));
    assert_eq!(
        serde_json::to_value(Duplicate::new(&copy, &at))?,
        json!({
            "source_channel_id": "src",
            "channel_id": "new",
            "name": "Copy",
            "duplicate_permissions": true,
            "parent_id": "f",
            "parent_type": "channel",
            "order_index": 2.5,
        })
    );
    Ok(())
}

/// How many `PATCH channel/{id}` entries read `text` as their body.
fn patch_entries_reading(text: &str) -> usize {
    CATALOGUE
        .iter()
        .flat_map(|list| list.iter())
        .filter(|entry| {
            entry.method == HttpMethod::Patch
                && entry.template == "/gapi/channel/{channel_id}"
        })
        .filter(|entry| matches!(entry.body, Body::Typed(read) if read(text).is_ok()))
        .count()
}

#[test]
fn each_channel_patch_is_read_by_one_entry_and_a_mixture_by_none() -> Result<()> {
    let sent = [
        serde_json::to_string(&Rename::new("x"))?,
        serde_json::to_string(&SetCover::new("p"))?,
        serde_json::to_string(&ResetCover::default())?,
        serde_json::to_string(&SetPreview::new("p"))?,
        r#"{"workspace_member_access":null,"in_trash":null,"thumbnail_updated_at":null}"#
            .to_owned(),
    ];
    let mixed = [
        r#"{"name":"x","thumbnail":"p","in_trash":null,"thumbnail_updated_at":null}"#,
        r#"{"name":"x","in_trash":null,"thumbnail_updated_at":null,"by":1}"#,
        r#"{"name":"x"}"#,
    ];
    assert_eq!(
        (
            sent.map(|text| patch_entries_reading(&text)),
            mixed.map(patch_entries_reading)
        ),
        ([1; 5], [0; 3])
    );
    Ok(())
}
