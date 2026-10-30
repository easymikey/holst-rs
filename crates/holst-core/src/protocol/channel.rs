use serde::Deserialize;
use serde_json::Value;

use crate::{
    error::{Error, Result, board_socket::BoardSocketError},
    protocol::document::{OBJECTS, OBJECTS_V3},
};

/// The formats on hud that the document code reads and writes.
const SCHEMA_V2: u32 = 2;
const SCHEMA_V3: u32 = 3;
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
    /// Root of the board's objects; [`BoardSocketError::UnsupportedFormat`]
    /// unless v2, or v3 on hud; a missing version reads 1, provider hud.
    pub(crate) fn supported(&self) -> Result<&'static str> {
        match (self.schema(), self.provider()) {
            (SCHEMA_V2, SUPPORTED_PROVIDER) => Ok(OBJECTS),
            (SCHEMA_V3, SUPPORTED_PROVIDER) => Ok(OBJECTS_V3),
            _ => Err(self.refused()),
        }
    }

    /// # Errors
    ///
    /// As [`Self::supported`]; whole-board files of a v3 board are v2 only.
    pub(crate) fn whole_file(&self) -> Result<()> {
        self.supported().and_then(|root| {
            (root == OBJECTS)
                .then_some(())
                .ok_or_else(|| self.refused())
        })
    }

    fn schema(&self) -> u32 {
        self.schema_version.unwrap_or(1)
    }

    fn provider(&self) -> &str {
        self.provider.as_deref().unwrap_or(SUPPORTED_PROVIDER)
    }

    fn refused(&self) -> Error {
        Error::BoardSocket(BoardSocketError::UnsupportedFormat {
            schema: self.schema(),
            provider: self.provider().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::BoardFormat;
    use crate::protocol::channel::root_access;

    #[rstest::rstest]
    #[case::v2(json!({"schema_version": 2}), Some("objects"), true)]
    #[case::v3(json!({"schema_version": 3, "server_provider_type": "hud"}), Some("objects_v3"), false)]
    #[case::v1(json!({"schema_version": 1}), None, false)]
    #[case::unversioned(json!({}), None, false)]
    #[case::hocuspocus(
        json!({"schema_version": 2, "server_provider_type": "hocuspocus"}),
        None,
        false
    )]
    fn a_board_format_names_the_objects_root_or_is_refused(
        #[case] channel: serde_json::Value,
        #[case] root: Option<&str>,
        #[case] whole_file: bool,
    ) {
        let format: BoardFormat =
            serde_json::from_value(channel).unwrap_or(BoardFormat {
                schema_version: None,
                provider: None,
            });
        assert_eq!(
            (format.supported().ok(), format.whole_file().is_ok()),
            (root, whole_file)
        );
    }

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
