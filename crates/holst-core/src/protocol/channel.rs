use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    domain::node::{CreationStamp, NewNode, NewParent, NodeKind, Parent},
    error::{Error, Result, board_socket::BoardSocketError},
};

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
        (schema == SUPPORTED_SCHEMA && provider == SUPPORTED_PROVIDER)
            .then_some(())
            .ok_or_else(|| {
                Error::BoardSocket(BoardSocketError::UnsupportedFormat {
                    schema,
                    provider: provider.to_owned(),
                })
            })
    }
}

impl NewNode {
    /// The body of `channel/create` by `author_id`, its id and time from
    /// `stamp`. In a folder the channel inherits the folder's access; at the
    /// root the workspace members get `root_access`, none for private, as the
    /// web client sends.
    ///
    /// # Errors
    ///
    /// [`crate::error::input::InputError::UnknownKind`] when the kind is
    /// neither board nor folder.
    pub(crate) fn body(
        &self,
        author_id: &str,
        stamp: &CreationStamp,
        root_access: Option<&str>,
    ) -> Result<Value> {
        let kind = NodeKind::creatable(self.kind.as_str())?;
        let parent = self.parent.parent_in(&self.workspace);
        let now = stamp.at.iso8601();
        let (member_access, parent_access) = match self.parent {
            NewParent::Root => {
                (root_access.map_or(Value::Null, Value::from), Value::Null)
            }
            NewParent::Folder(_) => (Value::Null, Value::from("inherit")),
        };
        Ok(json!({
            "id": stamp.id,
            "name": self.name.as_str(),
            "type": kind.as_str(),
            "permits": ["read", "read_comments"],
            "public": false,
            "thumbnail": Value::Null,
            "author_id": author_id,
            "in_trash": Value::Null,
            "parent_type": parent.api_type(),
            "parent_id": parent.id(),
            "public_access": Value::Null,
            "workspace_id": self.workspace.as_str(),
            "created_at": now,
            "updated_at": now,
            "update_author_id": author_id,
            "thumbnail_updated_at": Value::Null,
            "thumbnail_fixed": false,
            "thumbnail_color": Value::Null,
            "schema_version": 2,
            "order_index": stamp.at.sidebar_order(),
            "workspace_member_access": member_access,
            "parent_channel_access": parent_access,
        }))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CopyBody<'a> {
    pub source_channel_id: &'a str,
    pub name: &'a str,
    pub parent: &'a Parent,
    pub duplicate_permissions: bool,
}

impl CopyBody<'_> {
    #[must_use]
    pub(crate) fn body(&self, channel_id: &str, order_index: f64) -> Value {
        json!({
            "source_channel_id": self.source_channel_id,
            "channel_id": channel_id,
            "name": self.name,
            "duplicate_permissions": self.duplicate_permissions,
            "parent_id": self.parent.id(),
            "parent_type": self.parent.api_type(),
            "order_index": order_index,
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use rstest::rstest;
    use serde_json::json;

    use crate::{
        Result,
        domain::{
            id::WorkspaceId,
            node::{CreationStamp, NewNode, NewParent, NodeKind, Parent},
            time::Timestamp,
        },
        protocol::channel::{CopyBody, root_access},
    };

    const WORKSPACE: &str = "00000000-0000-4000-8000-000000000001";
    const FOLDER: &str = "00000000-0000-4000-8000-0000000000f9";

    // 2026-10-21T10:05:45.404Z.
    fn stamp(id: &str) -> CreationStamp {
        CreationStamp::at(id, Timestamp::from_millis(1_792_577_145_404))
    }

    fn new_channel(
        name: &str,
        kind: NodeKind,
        folder: Option<&str>,
    ) -> Result<NewNode> {
        Ok(NewNode {
            name: name.parse()?,
            kind,
            workspace: WORKSPACE.parse::<WorkspaceId>()?,
            parent: match folder {
                Some(folder) => NewParent::Folder(folder.parse()?),
                None => NewParent::Root,
            },
        })
    }

    #[rstest]
    #[case::root(None)]
    #[case::in_folder(Some(FOLDER))]
    fn new_board_body_matches_observed_shape(
        #[case] folder: Option<&str>,
    ) -> Result<()> {
        let body = new_channel("zz-test", NodeKind::Board, folder)?.body(
            "me",
            &stamp("11111111-1111-1111-1111-111111111111"),
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
        assert!(new.body("me", &stamp("id"), None).is_err());
        Ok(())
    }

    #[rstest]
    #[case::board(NodeKind::Board)]
    #[case::folder(NodeKind::Folder)]
    fn a_channel_in_a_folder_inherits_its_access(#[case] kind: NodeKind) -> Result<()> {
        let body = new_channel("Plans", kind, Some(FOLDER))?.body(
            "me",
            &stamp("id"),
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
        #[case] sent: serde_json::Value,
    ) -> Result<()> {
        let body = new_channel("Plans", NodeKind::Board, None)?.body(
            "me",
            &stamp("id"),
            default,
        )?;
        assert_eq!(body.get("workspace_member_access"), Some(&sent));
        assert_eq!(body.get("parent_channel_access"), Some(&json!(null)));
        Ok(())
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

    #[test]
    fn duplicate_body_names_parent_and_copy() {
        let parent = Parent::Folder("f".into());
        let body = CopyBody {
            source_channel_id: "src",
            name: "Copy",
            parent: &parent,
            duplicate_permissions: true,
        }
        .body("new", 2.5);
        assert_eq!(
            body,
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
    }
}
