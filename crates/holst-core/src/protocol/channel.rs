use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result, board_socket::BoardSocketError};

/// The format v2 on hud, the only one the document code reads and writes.
const SUPPORTED_SCHEMA: u32 = 2;
const SUPPORTED_PROVIDER: &str = "hud";
/// A workspace's default access for new root channels; `null` is private.
const ROOT_ACCESS_KEY: &str = "default_new_root_channel_workspace_member_access";

/// The root access when the workspace record has no default (not seen live).
const UNKNOWN_ROOT_ACCESS: &str = "co_owner";

/// The access the workspace gives a new channel at its root, from the reply of
/// `current_user`; a `null` default is private, a missing one is co-owner.
pub(crate) fn root_access<'a>(account: &'a Value, workspace: &str) -> Option<&'a str> {
    let default = account
        .get("workspaces")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(workspace))
        })
        .and_then(|item| item.get(ROOT_ACCESS_KEY));
    default.map_or(Some(UNKNOWN_ROOT_ACCESS), Value::as_str)
}

/// How a board is stored, from its `channel/{id}` reply.
#[derive(Debug, Deserialize)]
pub(crate) struct BoardFormat {
    schema_version: Option<u32>,
    #[serde(rename = "server_provider_type")]
    provider: Option<String>,
}

impl BoardFormat {
    /// # Errors
    ///
    /// [`BoardSocketError::UnsupportedFormat`] unless the board is in format v2
    /// on hud; the web client reads a missing version as 1 and a missing
    /// provider as hud.
    pub(crate) fn supported(&self) -> Result<()> {
        let schema = self.schema_version.unwrap_or(1);
        let provider = self.provider.as_deref().unwrap_or(SUPPORTED_PROVIDER);
        match (schema, provider) {
            (SUPPORTED_SCHEMA, SUPPORTED_PROVIDER) => Ok(()),
            _ => Err(Error::BoardSocket(BoardSocketError::UnsupportedFormat {
                schema,
                provider: provider.to_owned(),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::protocol::channel::root_access;

    #[test]
    fn the_root_default_is_read_from_the_workspace_in_the_account() {
        let account = json!({ "workspaces": [
            { "id": "other", "default_new_root_channel_workspace_member_access": "viewer" },
            { "id": "mine", "default_new_root_channel_workspace_member_access": "editor" },
            { "id": "private", "default_new_root_channel_workspace_member_access": null },
            { "id": "bare" },
        ] });
        assert_eq!(
            [
                root_access(&account, "mine"),
                root_access(&account, "private"),
                root_access(&account, "bare"),
                root_access(&account, "absent"),
            ],
            [Some("editor"), None, Some("co_owner"), Some("co_owner")]
        );
    }
}
