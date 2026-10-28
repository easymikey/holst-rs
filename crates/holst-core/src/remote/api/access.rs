use holst_macros::{patch, post};

use crate::{
    domain::sharing::SubtreeTransfer,
    protocol::sharing::{OwnershipTransfer, RoleGrant, SharingPatch},
};

/// Link, member and template keys go to the channel.
#[patch(
        "channel/{channel_id}",
        body = patch.checked()?,
        request_only
    )]
pub async fn set_channel_access(
    client: &Client,
    channel_id: &str,
    patch: SharingPatch,
) -> Result<Value>;

/// What members may do on a board: the `who_can_*` settings only.
#[patch(
        "channel/{channel_id}/sharing_settings",
        body = patch.checked()?,
        request_only
    )]
pub async fn set_sharing_settings(
    client: &Client,
    channel_id: &str,
    patch: SharingPatch,
) -> Result<Value>;

/// Grant access; `role: None` (sent as `null`, unverified) revokes it.
#[post(
        "channel_user/set",
        body = RoleGrant { channel_id, user_id, role },
        request_only
    )]
pub async fn set_member_right(
    client: &Client,
    channel_id: &str,
    user_id: &str,
    role: Option<&str>,
) -> Result<Value>;

/// Give a board to another owner. Irreversible; not verified live.
#[post(
        "channel/{channel_id}/transfer_ownership",
        body = OwnershipTransfer {
            new_owner_user_id,
            transfer_owned_subtree: subtree.is_transferred(),
        },
        request_only
    )]
pub async fn transfer_ownership(
    client: &Client,
    channel_id: &str,
    new_owner_user_id: &str,
    subtree: SubtreeTransfer,
) -> Result<Value>;
