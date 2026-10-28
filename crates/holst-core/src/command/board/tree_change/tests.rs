#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        board::{
            CreateArgs,
            tree_change::{create, mv, rename, restore, rm},
        },
        fixture::{arguments, client, json_reply, status_of, uuid},
        table::parse,
    },
    effect::http::HttpResponse,
    remote::client::fake::Fake,
};

/// `board create` names no board, so the fixture's default board does not fit.
fn create_args(given: &Value) -> Result<CreateArgs> {
    parse(given.as_object().cloned().unwrap_or_default())
}

fn account() -> HttpResponse {
    json_reply(&json!({
        "id": "u1",
        "name": "Ann",
        "workspaces": [{ "id": uuid(1), "name": "easy" }],
    }))
}

/// Plans (a folder holding `Roadmap`), Retro and Archive (an empty folder).
fn tree() -> HttpResponse {
    json_reply(&json!({ "channels": [
        { "id": uuid(11), "type": "folder", "name": "Plans", "order_index": 1.0 },
        { "id": uuid(12), "type": "board", "name": "Roadmap", "parent_id": uuid(11), "order_index": 1.0 },
        { "id": uuid(13), "type": "board", "name": "Retro", "order_index": 2.0 },
        { "id": uuid(14), "type": "folder", "name": "Archive", "order_index": 3.0 },
    ]}))
}

fn channel(ordinal: u32, name: &str) -> HttpResponse {
    json_reply(&json!({
        "id": uuid(ordinal),
        "name": name,
        "workspace_id": uuid(1),
        "parent_id": uuid(1),
        "parent_type": "workspace",
    }))
}

fn done() -> HttpResponse {
    json_reply(&json!({ "id": uuid(99), "name": "Made" }))
}

/// The requests that were made, as `METHOD /path`.
fn asked(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.url.path()))
        .collect()
}

fn writes(asked: &[String]) -> usize {
    asked
        .iter()
        .filter(|line| !line.starts_with("GET "))
        .count()
}

#[tokio::test]
async fn a_board_is_created_in_a_folder_and_has_a_link() -> Result<()> {
    let (_, client) = client([account(), tree(), account(), done()])?;
    let args = create_args(&json!({ "name": "Notes", "folder": "Plans" }))?;
    let changed = create(&client, &args).await?;
    assert_eq!(status_of(&changed.outcome), "sent");
    assert!(
        changed
            .node
            .link
            .is_some_and(|link| link.contains(&uuid(99))),
        "{:?}",
        changed.node.id
    );
    Ok(())
}

#[tokio::test]
async fn a_folder_is_created_and_has_a_link_in_its_workspace() -> Result<()> {
    let (_, client) = client([account(), account(), account(), done()])?;
    let args = create_args(&json!({ "name": "Notes", "kind": "folder" }))?;
    let changed = create(&client, &args).await?;
    let link = changed.node.link.unwrap_or_default();
    assert!(
        link.ends_with(&format!("/w/{}/f/{}", uuid(1), uuid(99))),
        "{link}"
    );
    Ok(())
}

#[tokio::test]
async fn a_rename_reports_the_old_and_the_new_name() -> Result<()> {
    let (fake, client) = client([channel(13, "Retro"), done()])?;
    let args = arguments(json!({ "board": uuid(13), "name": " Retro 2 " }))?;
    let changed = rename(&client, &args).await?;
    assert_eq!(
        (
            status_of(&changed.outcome),
            changed.node.previous_name,
            changed.node.name,
            asked(&fake).last().cloned()
        ),
        (
            String::from("sent"),
            Some(String::from("Retro")),
            Some(String::from("Retro 2")),
            Some(format!("PATCH /gapi/channel/{}", uuid(13)))
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_blank_name_is_refused_before_any_request() -> Result<()> {
    let (fake, client) = client([])?;
    let args = arguments(json!({ "board": "Retro", "name": "  " }))?;
    let refused = rename(&client, &args).await.is_err();
    assert_eq!((refused, asked(&fake).len()), (true, 0));
    Ok(())
}

#[tokio::test]
async fn a_confirmed_or_dry_move_of_a_folder_goes_on() -> Result<()> {
    let (fake, client) = client([
        channel(11, "Plans"),
        tree(),
        channel(11, "Plans"),
        tree(),
        done(),
    ])?;
    let dry = arguments(json!({ "board": uuid(11), "folder": "/", "dry_run": true }))?;
    let confirmed =
        arguments(json!({ "board": uuid(11), "folder": "/", "confirm": true }))?;
    let shown = mv(&client, &dry).await?;
    let moved = mv(&client, &confirmed).await?;
    assert_eq!(
        (
            status_of(&shown.outcome),
            status_of(&moved.outcome),
            writes(&asked(&fake))
        ),
        (String::from("dry_run"), String::from("sent"), 1)
    );
    Ok(())
}

#[tokio::test]
async fn a_folder_deleted_for_good_names_what_it_leaves_and_where() -> Result<()> {
    let folder = || {
        json_reply(&json!({
            "id": uuid(11),
            "type": "folder",
            "name": "Plans",
            "workspace_id": uuid(1),
        }))
    };
    let moved = json_reply(&json!({ "updatedChannelIds": [uuid(12)] }));
    let (fake, client) = client([folder(), tree(), moved, folder(), tree()])?;
    let sent = rm(
        &client,
        &arguments(json!({ "board": uuid(11), "permanently": true, "confirm": true }))?,
    )
    .await?;
    let planned = rm(
        &client,
        &arguments(json!({ "board": uuid(11), "permanently": true, "dry_run": true }))?,
    )
    .await?;
    assert_eq!(
        (
            sent.node.phrase(false),
            planned.node.phrase(true),
            writes(&asked(&fake))
        ),
        (
            String::from(
                "deleted Plans for good; Roadmap (board) is now at the workspace root"
            ),
            String::from(
                "would delete Plans for good; Roadmap (board) would be at the workspace root"
            ),
            1
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_trashed_board_is_restored_by_id_or_by_name() -> Result<()> {
    let trash = json_reply(&json!({ "channels": [
        { "id": uuid(15), "type": "board", "name": "Old" },
    ]}));
    let trashed =
        json_reply(&json!({ "id": uuid(15), "name": "Old", "in_trash": true }));
    let (fake, client) = client([trashed, done(), account(), trash, done()])?;
    let by_id = arguments(json!({ "board": uuid(15) }))?;
    let by_name = arguments(json!({ "board": "Old" }))?;
    let first = restore(&client, &by_id).await?;
    let second = restore(&client, &by_name).await?;
    let sent = asked(&fake);
    assert_eq!(
        (
            status_of(&first.outcome),
            first.node.id,
            status_of(&second.outcome),
            second.node.id,
            writes(&sent)
        ),
        (
            String::from("sent"),
            Some(uuid(15)),
            String::from("sent"),
            Some(uuid(15)),
            2
        ),
        "{sent:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_id_that_is_not_in_the_trash_is_refused_unsent() -> Result<()> {
    let (fake, client) = client([channel(13, "Retro")])?;
    let args = arguments(json!({ "board": uuid(13) }))?;
    let refused = restore(&client, &args)
        .await
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(refused.contains("not found in the trash"), "{refused}");
    assert_eq!(writes(&asked(&fake)), 0);
    Ok(())
}

#[tokio::test]
async fn a_dry_run_request_is_structured_data() -> Result<()> {
    let (_, client) = client([channel(13, "Retro")])?;
    let args =
        arguments(json!({ "board": uuid(13), "name": "Retro 2", "dry_run": true }))?;
    let changed = rename(&client, &args).await?;
    let outcome = serde_json::to_value(&changed.outcome)?;
    assert_eq!(
        outcome,
        json!({
            "status": "dry_run",
            "request": {
                "method": "PATCH",
                "path": format!("/gapi/channel/{}", uuid(13)),
                "query": [],
                "body": {
                    "name": "Retro 2",
                    "in_trash": null,
                    "thumbnail_updated_at": null,
                },
            },
        })
    );
    Ok(())
}
