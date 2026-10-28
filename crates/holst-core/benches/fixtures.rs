//! Synthetic API replies shaped like the real ones, no captured traffic.

use serde_json::{Value, json};

pub(crate) fn uuid(kind: u32, i: usize) -> String {
    format!("{kind:08x}-0000-4000-8000-{i:012x}")
}

pub(crate) fn tree(ws: usize, n: usize, target: Option<&str>) -> Value {
    let ws_id = uuid(0, ws);
    let order = std::iter::successors(Some(1_663_857_879.099_312), |o| Some(o + 1.0));
    let channels: Vec<Value> = (0..n)
        .zip(order)
        .map(|(i, order_index)| {
            let folder = i % 10 == 0;
            let (parent_id, parent_type, kind) = match i % 10 {
                0 => (ws_id.clone(), "workspace", "folder"),
                _ => (uuid(1, ws * 1_000_000 + i - i % 10), "channel", "board"),
            };
            let name = match target {
                Some(t) if i == n - 1 => t.to_string(),
                _ if folder => format!("Папка {i}"),
                _ => format!("Доска {ws}-{i} планирование"),
            };
            json!({
                "id": uuid(1, ws * 1_000_000 + i),
                "workspace_id": ws_id,
                "name": name,
                "type": kind,
                "parent_id": parent_id,
                "parent_type": parent_type,
                "order_index": order_index,
                "public": false,
                "public_access": null,
                "private": false,
                "in_trash": null,
                "author_id": uuid(2, 0),
                "created_at": "2026-10-21T12:00:00.000Z",
                "permits": ["read", "read_comments", "write"],
                "parent_channel_access": "inherit",
                "server_provider_type": "hud",
                "schema_version": 2,
                "share_as_template": false,
                "free_restricted": false,
                "is_imported_from_miro": false,
                "channel_user": null,
            })
        })
        .collect();
    json!({
        "workspace": { "id": ws_id, "name": format!("ws {ws}") },
        "spaces": [],
        "channels": channels,
    })
}

pub(crate) fn current_user(workspaces: usize) -> Value {
    let list: Vec<Value> = (0..workspaces)
        .map(|w| json!({ "id": uuid(0, w), "name": format!("ws {w}") }))
        .collect();
    json!({ "id": uuid(2, 0), "name": "bench", "workspaces": list })
}
