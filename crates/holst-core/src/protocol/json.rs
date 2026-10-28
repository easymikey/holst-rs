//! Fields of JSON replies that the next step relies on, and blank replies.

const NOISE: &[&str] = &[
    "thumbnail",
    "thumbnail_updated_at",
    "thumbnail_fixed",
    "thumbnail_color",
    "thumbnailUrl",
    "schema_version",
    "is_imported_from_miro",
    "order_index",
    "avatar",
    "avatar_url",
    "avatarUrl",
];

/// Tells whether the reply field `key` is web-client noise for the model.
#[must_use]
pub fn is_noise(key: &str) -> bool {
    NOISE.contains(&key)
}

/// Keys under which a change reply wraps the node it changed.
const WRAPPERS: [&str; 4] = ["updatedChannel", "channel", "result", "workspace"];

/// What a Tree change reply says about the node it changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangedNode {
    pub id: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
}

/// The id, name and kind of the node a Tree change `reply` names, unwrapped or
/// not.
#[must_use]
pub fn changed_node(reply: &serde_json::Value) -> ChangedNode {
    let about = WRAPPERS
        .into_iter()
        .find_map(|key| reply.get(key).filter(|inner| inner.is_object()))
        .unwrap_or(reply);
    let field = |key| {
        about
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    ChangedNode {
        id: field("id"),
        name: field("name"),
        kind: field("type"),
    }
}
