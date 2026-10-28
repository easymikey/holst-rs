#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        fixture::{arguments, channel, channel_with, client, json_reply, status_of},
        template::{Changed, CreateArgs, ShowArgs, create::created, ls, rm, show},
    },
    domain::board::EditRight,
    effect::http::HttpResponse,
    protocol::hud::{DocumentResponse, UpdatePayload},
    remote::{
        client::{
            Client,
            fake::{Fake, reply},
        },
        socket::fake::{
            BOARD,
            Scripted,
            answer,
            empty_board,
            script,
            sticker_by,
            update,
        },
    },
};

/// The type of the drawn thumbnail: PNG when the build renders one.
#[cfg(feature = "png")]
const THUMBNAIL_TYPE: &str = "image/png";
#[cfg(not(feature = "png"))]
const THUMBNAIL_TYPE: &str = "image/svg+xml";

const TEMPLATE: &str = "00000001-0000-4000-8000-0000000000a1";

fn account() -> Value {
    json!({
        "id": "u1",
        "name": "Ann",
        "workspaces": [{ "id": "w1", "name": "easy" }, { "id": "w2", "name": "team" }],
    })
}

fn one_workspace() -> Value {
    json!({ "id": "u1", "workspaces": [{ "id": "w1", "name": "easy" }] })
}

/// A listing whose ids start with `list`, so two lists never share an id.
fn templates(list: &str, names: &[&str]) -> Value {
    json!({
        "board_templates": names
            .iter()
            .map(|name| json!({ "id": format!("{list}-{name}"), "name": name }))
            .collect::<Vec<_>>(),
    })
}

fn categories() -> Value {
    json!({ "categories": [
        { "id": "c1", "slug": "retro", "name": "Retrospectives" },
    ]})
}

/// The requests that were made, as `METHOD /path`.
fn asked(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.url.path()))
        .collect()
}

/// The query of the one upload request as `name=value` pairs, and the type of
/// the file part.
fn upload_of(fake: &Fake) -> (Vec<(String, String)>, String) {
    fake.requests()
        .iter()
        .find(|request| request.url.path() == "/gapi/upload_file")
        .map(|request| {
            let body = request
                .body
                .as_ref()
                .map(|body| String::from_utf8_lossy(&body.bytes).into_owned())
                .unwrap_or_default();
            let part_type = body
                .lines()
                .find_map(|line| line.strip_prefix("Content-Type: "))
                .unwrap_or_default()
                .to_owned();
            (
                request
                    .url
                    .query_pairs()
                    .map(|(name, value)| (name.into_owned(), value.into_owned()))
                    .collect(),
                part_type,
            )
        })
        .unwrap_or_default()
}

fn bodies(fake: &Fake) -> Vec<Value> {
    fake.requests()
        .iter()
        .filter_map(|request| request.body.as_ref())
        .filter_map(|body| serde_json::from_slice(&body.bytes).ok())
        .collect()
}

/// A board holding one sticker, `a`.
fn board() -> Scripted {
    let (state, _) = sticker_by(4, "a");
    script([Some(vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        update(0, UpdatePayload::Inline(state)),
    ])])
}

/// The `channel/{id}` reply of the board in workspace `w1`.
fn board_workspace() -> HttpResponse {
    channel_with(&json!({
        "schema_version": 2,
        "server_provider_type": "hud",
        "workspace_id": "w1",
    }))
}

async fn make(client: &Client, socket: &Scripted, given: &Value) -> Result<Changed> {
    created(client, socket, &arguments::<CreateArgs>(given.clone())?).await
}

#[tokio::test]
async fn a_category_outside_the_standard_view_is_refused() -> Result<()> {
    let (fake, client) = client([])?;
    let error = ls(&client, &arguments(json!({ "category": "retro" }))?)
        .await
        .err();
    assert_eq!(
        (error.map(|error| error.to_string()), asked(&fake).len()),
        (
            Some(
                "category of `template ls` must be left out unless --view is standard"
                    .to_owned()
            ),
            0
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_template_is_shown_by_id() -> Result<()> {
    let (fake, client) = client([json_reply(
        &json!({ "id": TEMPLATE, "name": "Retro", "cells": 3 }),
    )])?;
    let shown = show(
        &client,
        &arguments::<ShowArgs>(json!({ "template": TEMPLATE }))?,
    )
    .await?;
    assert_eq!(
        (shown.name.as_deref(), asked(&fake)),
        (
            Some("Retro"),
            vec!["GET /gapi/board_template/get".to_owned()]
        )
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_template_id_is_not_found() -> Result<()> {
    let (_, client) =
        client([reply(404, &[("content-type", "application/json")], "{}")])?;
    let error = show(
        &client,
        &arguments::<ShowArgs>(json!({ "template": TEMPLATE }))?,
    )
    .await
    .err();
    assert_eq!(
        error.map(|error| error.to_string()),
        Some(format!("template \"{TEMPLATE}\" not found"))
    );
    Ok(())
}

#[tokio::test]
async fn a_template_name_is_looked_up_among_all_the_lists() -> Result<()> {
    let replies = |personal: &[&str]| {
        [
            json_reply(&one_workspace()),
            json_reply(&templates("p", personal)),
            json_reply(&templates("w", &["Plan"])),
            json_reply(&categories()),
            json_reply(&templates("s", &["Retro"])),
            json_reply(&json!({ "id": "t0", "name": "Plan" })),
        ]
    };
    let (_, first) = client(replies(&["Mine"]))?;
    let one = show(
        &first,
        &arguments::<ShowArgs>(json!({ "template": "plan" }))?,
    )
    .await?;
    let (_, second) = client(replies(&["Retro"]))?;
    let several = show(
        &second,
        &arguments::<ShowArgs>(json!({ "template": "retro" }))?,
    )
    .await
    .err();
    let (_, third) = client(replies(&["Mine"]))?;
    let none = show(
        &third,
        &arguments::<ShowArgs>(json!({ "template": "nothing" }))?,
    )
    .await
    .err();
    assert_eq!(
        (
            one.name.as_deref(),
            several.map(|error| error.to_string()),
            none.map(|error| error.to_string())
        ),
        (
            Some("Plan"),
            Some("\"retro\" matches several templates: Retro (p-Retro), retro/Retro (s-Retro)".to_owned()),
            Some("template \"nothing\" not found. Available: Mine (p-Mine), easy/Plan (w-Plan), retro/Retro (s-Retro)".to_owned())
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_held_delete_and_a_dry_run_show_the_same_request() -> Result<()> {
    let request = async |given: Value| -> Result<Value> {
        let (_fake, client) = client([
            json_reply(&json!({ "id": TEMPLATE, "name": "Retro" })),
            json_reply(&json!({})),
        ])?;
        let changed = rm(&client, &arguments(given)?).await?;
        Ok(serde_json::to_value(&changed.outcome)?
            .pointer("/request")
            .cloned()
            .unwrap_or_default())
    };
    let held = request(json!({ "template": TEMPLATE })).await?;
    let dry = request(json!({ "template": TEMPLATE, "dry_run": true })).await?;
    let planned = json!({
        "method": "DELETE",
        "path": "/gapi/board_template/delete",
        "query": [["board_template_id", "00000001-0000-4000-8000-0000000000a1"]],
        "body": null
    });
    assert_eq!((held, dry), (planned.clone(), planned));
    Ok(())
}

#[tokio::test]
async fn a_template_is_made_from_a_whole_board_after_its_thumbnail() -> Result<()> {
    let (fake, client) = client([
        channel(),
        board_workspace(),
        json_reply(&account()),
        json_reply(&json!({ "name": "thumb.png" })),
        json_reply(&json!({ "id": "t9", "name": "Retro" })),
    ])?;
    let made = make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": " Retro ", "description": "Notes" }),
    )
    .await?;
    let body = bodies(&fake).pop().unwrap_or_default();
    let objects: Vec<Value> = body
        .pointer("/data/data")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    assert_eq!(
        (
            status_of(&made.outcome),
            made.id.as_deref(),
            made.objects,
            asked(&fake),
            body.get("name").cloned(),
            body.get("scope").cloned(),
            body.get("standard").cloned(),
            body.get("description").cloned(),
            body.get("thumbnail").cloned(),
            [
                body.get("thumbnail_fixed").cloned(),
                body.get("thumbnail_color").cloned(),
                body.get("category_ids").cloned(),
                body.get("workspace_id").cloned(),
            ],
            objects
                .iter()
                .filter_map(|object| object.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
        ),
        (
            "sent".to_owned(),
            Some("t9"),
            Some(1),
            vec![
                format!("GET /gapi/channel/{BOARD}"),
                format!("GET /gapi/channel/{BOARD}"),
                "GET /gapi/current_user".to_owned(),
                "POST /gapi/upload_file".to_owned(),
                "POST /gapi/board_template/create".to_owned()
            ],
            Some(json!("Retro")),
            Some(json!("personal")),
            Some(json!(false)),
            Some(json!("Notes")),
            Some(json!("thumb.png")),
            [
                Some(json!(false)),
                Some(json!(null)),
                Some(json!([])),
                Some(json!("w1"))
            ],
            vec!["a"],
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_personal_thumbnail_is_bound_to_the_user_with_its_type() -> Result<()> {
    let (fake, client) = client([
        channel(),
        board_workspace(),
        json_reply(&account()),
        json_reply(&json!({ "name": "thumb.png" })),
        json_reply(&json!({})),
    ])?;
    make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": "Mine" }),
    )
    .await?;
    let (query, part_type) = upload_of(&fake);
    let names: Vec<&str> = query.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        (
            names,
            query.first().map(|(_, id)| id.as_str()),
            part_type.trim_end()
        ),
        (vec!["user_id", "key"], Some("u1"), THUMBNAIL_TYPE)
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_template_uploads_nothing() -> Result<()> {
    let (fake, client) = client([json_reply(&json!({
        "id": "u1",
        "workspaces": [{ "id": "w1", "name": "easy", "role": "guest" }],
    }))])?;
    let refused = make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": "Retro", "for": "workspace" }),
    )
    .await;
    assert_eq!(
        (refused.is_err(), asked(&fake)),
        (true, vec!["GET /gapi/current_user".to_owned()])
    );
    Ok(())
}

#[tokio::test]
async fn a_member_makes_a_workspace_template_and_a_personal_one_asks_no_role()
-> Result<()> {
    let (fake, client) = client([
        json_reply(&json!({
            "id": "u1",
            "workspaces": [{ "id": "w1", "name": "easy", "role": "member" }],
        })),
        channel(),
        json_reply(&json!({ "name": "thumb.png" })),
        json_reply(&json!({})),
        channel(),
        board_workspace(),
        json_reply(&account()),
        json_reply(&json!({ "name": "thumb.png" })),
        json_reply(&json!({})),
    ])?;
    make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": "Retro", "for": "workspace" }),
    )
    .await?;
    make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": "Mine" }),
    )
    .await?;
    let check = format!("GET /gapi/channel/{BOARD}");
    assert_eq!(
        asked(&fake),
        [
            "GET /gapi/current_user",
            &check,
            "POST /gapi/upload_file",
            "POST /gapi/board_template/create",
            &check,
            &check,
            "GET /gapi/current_user",
            "POST /gapi/upload_file",
            "POST /gapi/board_template/create",
        ]
    );
    Ok(())
}

#[tokio::test]
async fn an_empty_board_or_a_blank_name_makes_no_template() -> Result<()> {
    // The one request is the empty board's format check.
    let (fake, client) = client([channel()])?;
    let empty = make(
        &client,
        &script([Some(empty_board(EditRight::Editable))]),
        &json!({ "board": BOARD, "name": "Retro" }),
    )
    .await
    .err();
    let blank = make(&client, &board(), &json!({ "board": BOARD, "name": "  " }))
        .await
        .err();
    assert_eq!(
        (
            empty.map(|error| error.to_string()),
            blank.map(|error| error.to_string()),
            asked(&fake).len()
        ),
        (
            Some("the board is empty: there is no area to show".to_owned()),
            Some("name of `template create` must be text that is not blank".to_owned()),
            1
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_part_of_a_board_is_made_a_template_by_object() -> Result<()> {
    let (_, client) = client([channel(), board_workspace(), channel()])?;
    let made = make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": "Retro", "object": ["a"], "dry_run": true }),
    )
    .await?;
    let both = make(
        &client,
        &board(),
        &json!({ "board": BOARD, "name": "Retro", "object": ["a"], "frame": "Q4", "dry_run": true }),
    )
    .await
    .err();
    assert_eq!(
        (made.objects, both.map(|error| error.to_string())),
        (
            Some(1),
            Some("give one of an area, a frame or objects, not several".to_owned())
        )
    );
    Ok(())
}
