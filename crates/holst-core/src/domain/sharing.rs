//! Board access: the sharing fields of a channel reply and a sharing patch.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Whether the boards the owner holds below a board change hands too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubtreeTransfer {
    #[default]
    Keep,
    Transfer,
}

impl SubtreeTransfer {
    /// Whether the owned subtree changes hands.
    #[must_use]
    pub const fn is_transferred(self) -> bool {
        matches!(self, Self::Transfer)
    }
}

/// The access fields of a board, as the server names them.
///
/// ```
/// use holst_core::domain::sharing::Sharing;
/// use serde_json::json;
///
/// let channel = json!({ "id": "b1", "public": false, "thumbnail": "x.png" });
/// let settings: Sharing = serde_json::from_value(channel)?;
/// assert_eq!(serde_json::to_value(settings)?, json!({ "id": "b1", "public": false }));
///
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Map<String, Value>", into = "Map<String, Value>")]
pub struct Sharing(Map<String, Value>);

impl From<Map<String, Value>> for Sharing {
    fn from(channel: Map<String, Value>) -> Self {
        Self(
            channel
                .into_iter()
                .filter(|(key, _)| is_sharing_key(key))
                .collect(),
        )
    }
}

impl From<Sharing> for Map<String, Value> {
    fn from(settings: Sharing) -> Self {
        settings.0
    }
}

fn is_sharing_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "name"
            | "public"
            | "private"
            | "permits"
            | "free_restricted"
            | "share_as_template"
    ) || key.starts_with("public_")
        || key.starts_with("who_can_")
        || key.starts_with("sharing")
        || key.ends_with("_access")
}

#[cfg(test)]
mod tests {

    use serde_json::json;

    use crate::domain::sharing::Sharing;

    #[test]
    fn sharing_keys_keep_access_fields_and_drop_noise() {
        let channel = json!({
            "id": "b1",
            "name": "Board",
            "public": false,
            "public_access": null,
            "who_can_copy_content": "viewer",
            "workspace_member_access": "co_owner",
            "thumbnail": "x.png",
            "updated_at": "2026-10-21T10:00:00Z",
        });
        let expected = json!({
            "id": "b1",
            "name": "Board",
            "public": false,
            "public_access": null,
            "who_can_copy_content": "viewer",
            "workspace_member_access": "co_owner",
        });
        let settings = serde_json::from_value::<Sharing>(channel)
            .ok()
            .and_then(|settings| serde_json::to_value(settings).ok());
        assert_eq!(settings, Some(expected));
    }

    #[test]
    fn non_object_reply_is_refused() {
        assert!(serde_json::from_value::<Sharing>(json!("text")).is_err());
    }
}
