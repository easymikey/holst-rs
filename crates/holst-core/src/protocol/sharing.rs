//! The bodies that change who may see and edit a board.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, input::InputError};

/// A role on the wire: the server's word, or `null` for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum WireRole {
    None(()),
    Held(&'static str),
}

impl From<Option<&'static str>> for WireRole {
    fn from(role: Option<&'static str>) -> Self {
        role.map_or(Self::None(()), Self::Held)
    }
}

/// A patch of the sharing settings, under the server's names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SharingPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_access: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_member_access: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who_can_copy_content: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who_can_delete_content: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who_can_manage_collaboration_tools: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who_can_manage_lock_content: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who_can_manage_frame_hide_content: Option<WireRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_as_template: Option<bool>,
}

/// A patch as sent: the set keys and the two nulls of every channel patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharingBody {
    #[serde(flatten)]
    patch: SharingPatch,
    in_trash: (),
    thumbnail_updated_at: (),
}

impl SharingPatch {
    /// The patch to send.
    ///
    /// # Errors
    /// [`InputError::EmptySharingPatch`] when no setting is set.
    pub fn checked(self) -> Result<SharingBody, Error> {
        if self == Self::default() {
            Err(Error::Input(InputError::EmptySharingPatch))
        } else {
            Ok(SharingBody {
                patch: self,
                in_trash: (),
                thumbnail_updated_at: (),
            })
        }
    }
}

/// `channel_user/set`: a user's role on a board; no role revokes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RoleGrant<'a> {
    pub(crate) channel_id: &'a str,
    pub(crate) user_id: &'a str,
    pub(crate) role: Option<&'a str>,
}

/// `channel/{id}/transfer_ownership`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnershipTransfer<'a> {
    pub(crate) new_owner_user_id: &'a str,
    pub(crate) transfer_owned_subtree: bool,
}

/// The role the web client shows for a holder's permits, the top one first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeldRole {
    Viewer,
    Commentator,
    Editor,
    CoOwner,
}

/// A user and their permits (`userId` in the folder reply).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Holder {
    #[serde(alias = "userId")]
    pub(crate) user_id: String,
    #[serde(default)]
    permits: Option<Vec<String>>,
}

/// The roles by the permit that gives them, the top one first.
const ROLES: [(&str, HeldRole); 4] = [
    ("set_permits", HeldRole::CoOwner),
    ("write", HeldRole::Editor),
    ("write_comments", HeldRole::Commentator),
    ("read", HeldRole::Viewer),
];

impl Holder {
    /// The role the permits stand for, `None` when they give none.
    pub(crate) fn role(&self) -> Option<HeldRole> {
        ROLES
            .iter()
            .find(|(permit, _)| {
                self.permits.iter().flatten().any(|held| held == permit)
            })
            .map(|(_, role)| *role)
    }
}

/// The reply of `channel_user/users`: the holders under `result`.
#[derive(Deserialize)]
struct Members {
    result: Vec<Holder>,
}

/// The reply of `parent_total_permits`: the holders under `permits`.
#[derive(Deserialize)]
struct FolderRights {
    permits: Vec<Holder>,
}

/// The holders in a reply of `channel_user/users`.
pub(crate) fn members(reply: Value) -> serde_json::Result<Vec<Holder>> {
    serde_json::from_value::<Members>(reply).map(|reply| reply.result)
}

/// The holders in a reply of `parent_total_permits`.
pub(crate) fn folder_rights(reply: Value) -> serde_json::Result<Vec<Holder>> {
    serde_json::from_value::<FolderRights>(reply).map(|reply| reply.permits)
}

#[cfg(test)]
mod tests {

    use serde_json::json;

    use crate::protocol::sharing::{
        HeldRole,
        Holder,
        OwnershipTransfer,
        RoleGrant,
        SharingPatch,
        WireRole,
        folder_rights,
        members,
    };

    #[test]
    fn each_setting_serializes_to_its_wire_key_and_value() {
        let wire = |patch: SharingPatch| serde_json::to_value(patch).ok();
        let held = |role| Some(WireRole::Held(role));
        let cases = [
            (
                SharingPatch {
                    public: Some(true),
                    public_access: held("viewer"),
                    ..SharingPatch::default()
                },
                json!({ "public": true, "public_access": "viewer" }),
            ),
            (
                SharingPatch {
                    public: Some(false),
                    public_access: Some(None.into()),
                    ..SharingPatch::default()
                },
                json!({ "public": false, "public_access": null }),
            ),
            (
                SharingPatch {
                    workspace_member_access: held("co_owner"),
                    who_can_copy_content: held("viewer"),
                    who_can_delete_content: held("editor"),
                    who_can_manage_collaboration_tools: held("editor"),
                    who_can_manage_lock_content: held("editor"),
                    who_can_manage_frame_hide_content: held("editor"),
                    share_as_template: Some(true),
                    ..SharingPatch::default()
                },
                json!({
                    "workspace_member_access": "co_owner",
                    "who_can_copy_content": "viewer",
                    "who_can_delete_content": "editor",
                    "who_can_manage_collaboration_tools": "editor",
                    "who_can_manage_lock_content": "editor",
                    "who_can_manage_frame_hide_content": "editor",
                    "share_as_template": true,
                }),
            ),
        ];
        let wired = cases.clone().map(|(patch, _)| wire(patch));
        let expected = cases.map(|(_, expected)| Some(expected));
        assert_eq!(wired, expected);
    }

    #[test]
    fn an_empty_patch_is_refused_and_says_what_is_expected() {
        let error = SharingPatch::default()
            .checked()
            .err()
            .map(|e| e.to_string());
        assert!(error.is_some_and(|e| {
            e.starts_with("sharing settings must be a non-empty JSON object")
        }));
        let set = SharingPatch {
            public: Some(true),
            ..SharingPatch::default()
        };
        assert!(set.checked().is_ok());
    }

    #[test]
    fn a_sent_patch_carries_the_two_nulls_of_every_channel_patch() {
        let sent = SharingPatch {
            public: Some(true),
            ..SharingPatch::default()
        }
        .checked()
        .ok()
        .and_then(|body| serde_json::to_value(body).ok());
        assert_eq!(
            sent,
            Some(json!({
                "public": true,
                "in_trash": null,
                "thumbnail_updated_at": null,
            }))
        );
    }

    #[test]
    fn role_grant_has_the_wire_names_and_a_null_role_revokes() {
        let wire = |role| {
            serde_json::to_value(RoleGrant {
                channel_id: "c",
                user_id: "u",
                role,
            })
            .ok()
        };
        assert_eq!(
            wire(Some("editor")),
            Some(json!({ "channel_id": "c", "user_id": "u", "role": "editor" }))
        );
        assert_eq!(
            wire(None),
            Some(json!({ "channel_id": "c", "user_id": "u", "role": null }))
        );
    }

    #[test]
    fn ownership_transfer_has_the_wire_names() {
        let wire = serde_json::to_value(OwnershipTransfer {
            new_owner_user_id: "u",
            transfer_owned_subtree: true,
        })
        .ok();
        assert_eq!(
            wire,
            Some(json!({ "newOwnerUserId": "u", "transferOwnedSubtree": true }))
        );
    }

    #[test]
    fn holders_are_read_as_the_server_replies_and_the_role_follows_the_permits() {
        let editor = json!(["read", "write", "create_channel", "read_comments"]);
        let board = members(json!({ "result": [
            { "channel_id": "c", "user_id": "u", "permits": editor, "created_at": "t" },
            { "channel_id": "c", "user_id": "v", "permits": ["read", "read_comments"] },
        ] }));
        let folder = folder_rights(json!({ "permits": [
            { "userId": "w", "permits": ["set_permits", "read"] },
            { "userId": "x", "permits": null },
        ] }));
        let roles = |read: Option<Vec<Holder>>| {
            read.map(|holders| {
                holders
                    .into_iter()
                    .map(|holder| (holder.user_id.clone(), holder.role()))
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(
            (
                roles(board.ok()),
                roles(folder.ok()),
                roles(members(json!({ "result": [] })).ok()),
                roles(folder_rights(json!({ "permits": [] })).ok()),
                members(json!({ "users": [] })).is_err(),
            ),
            (
                Some(vec![
                    ("u".to_owned(), Some(HeldRole::Editor)),
                    ("v".to_owned(), Some(HeldRole::Viewer)),
                ]),
                Some(vec![
                    ("w".to_owned(), Some(HeldRole::CoOwner)),
                    ("x".to_owned(), None),
                ]),
                Some(vec![]),
                Some(vec![]),
                true
            )
        );
    }
}
