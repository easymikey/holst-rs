//! The boards the account opened last, across all workspaces.

use serde_json::{Value, json};

use super::{Node, Tree};
use crate::fake::{
    Fake,
    http::{Parts, Response, found, refused},
};

/// How many boards a page holds when the request does not say.
const PAGE_SIZE: usize = 20;

/// The ids of the boards opened, the last one first.
#[derive(Debug, Default)]
pub(super) struct Recents {
    ids: Vec<String>,
}

impl Recents {
    fn touch(&mut self, id: &str) {
        self.ids.retain(|known| known != id);
        self.ids.insert(0, id.to_owned());
    }
}

impl Tree {
    /// Marks the board the body names as opened just now.
    pub(super) fn visit(&mut self, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let id = body.get("channelId").and_then(Value::as_str);
        match id.filter(|id| self.find(id).is_some()) {
            Some(id) => {
                self.recent.touch(id);
                found(&json!({}))
            }
            None => refused(404, "not-found"),
        }
    }

    /// A page of `pageSize` recent boards from the `cursor` on.
    pub(super) fn recent_page(&self, parts: &Parts<'_>) -> Response {
        let number = |name: &str| {
            parts
                .query(name)
                .and_then(|text| text.parse::<usize>().ok())
        };
        let size = number("pageSize").unwrap_or(PAGE_SIZE).max(1);
        let start = number("cursor").unwrap_or(0);
        let live = self.recent_nodes();
        let channels: Vec<Value> = live
            .iter()
            .enumerate()
            .skip(start)
            .take(size)
            .map(|(place, node)| {
                json!({ "channelId": node.id, "date": opened_at(live.len() - place) })
            })
            .collect();
        found(&json!({
            "channels": channels,
            "cursor": (live.len() > start + size).then(|| (start + size).to_string()),
        }))
    }

    /// The names of the boards opened, the last one first.
    pub(super) fn recent_names(&self) -> Vec<String> {
        self.recent_nodes()
            .iter()
            .map(|node| node.name.clone())
            .collect()
    }

    /// The opened nodes still there and not in the trash.
    fn recent_nodes(&self) -> Vec<&Node> {
        self.recent
            .ids
            .iter()
            .filter_map(|id| self.find(id))
            .filter(|node| !node.trashed)
            .collect()
    }
}

/// The time of the `minutes`th minute of the fake's calendar.
fn opened_at(minutes: usize) -> String {
    format!(
        "2026-10-01T{:02}:{:02}:00Z",
        minutes / 60 % 24,
        minutes % 60
    )
}

impl Fake {
    /// Returns the names of the boards the account opened, the last one first.
    #[must_use]
    pub fn recent_of(&self) -> Vec<String> {
        self.locked().tree.recent_names()
    }
}
