use holst_macros::{patch, post};
use serde::{Deserialize, Serialize};

use crate::{protocol::sharing::SharingPatch, remote::catalogue::Endpoint};

/// `channel_user/set`: a user's role on a board; no role revokes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleGrant {
    pub channel_id: String,
    pub user_id: String,
    pub role: Option<String>,
}

/// `channel/{id}/transfer_ownership`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipTransfer {
    pub new_owner_user_id: String,
    pub transfer_owned_subtree: bool,
}

/// Link, member and template keys go to the channel.
#[patch("channel/{channel_id}", class = Admin, request_only)]
pub async fn set_channel_access(
    client: &Client,
    channel_id: &str,
    #[body] body: &SharingPatch,
) -> Result<Done>;

/// What members may do on a board: the `who_can_*` settings only.
#[patch("channel/{channel_id}/sharing_settings", class = Admin, request_only)]
pub async fn set_sharing_settings(
    client: &Client,
    channel_id: &str,
    #[body] body: &SharingPatch,
) -> Result<Done>;

/// Grant access; `role: None` (sent as `null`, unverified) revokes it.
#[post("channel_user/set", class = Admin, request_only)]
pub async fn set_member_right(
    client: &Client,
    #[body] body: &RoleGrant,
) -> Result<Done>;

/// Give a board to another owner. Irreversible; not verified live.
#[post(
    "channel/{channel_id}/transfer_ownership",
    class = Admin,
    request_only
)]
pub async fn transfer_ownership(
    client: &Client,
    channel_id: &str,
    #[body] body: &OwnershipTransfer,
) -> Result<Done>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[
    SET_CHANNEL_ACCESS,
    SET_SHARING_SETTINGS,
    SET_MEMBER_RIGHT,
    TRANSFER_OWNERSHIP,
];
