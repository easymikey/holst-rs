use holst_macros::{get, patch, post};
use serde_json::{Value, json};

use crate::{
    domain::node::{CreationStamp, DeleteMode, NewNode, NodeCopy, Parent},
    error::Result,
    protocol::channel::CopyBody,
    remote::client::Client,
};

/// A board's metadata and permissions.
#[get("channel/{channel_id}")]
pub async fn get(client: &Client, channel_id: &str) -> Result<Value>;

/// Who has access.
#[get("channel_user/users", query(channel_id = channel_id))]
pub async fn users(client: &Client, channel_id: &str) -> Result<Value>;

/// Permissions inherited from the folder.
#[get("channel/{channel_id}/parent_total_permits")]
pub async fn parent_total_permits(client: &Client, channel_id: &str) -> Result<Value>;

/// The order in the sidebar.
#[get("channel_user_order_index/{workspace_id}/list")]
pub async fn order_index_list(client: &Client, workspace_id: &str) -> Result<Value>;

/// Rename a board or a folder.
#[patch(
    "channel/{channel_id}",
    body = json!({ "name": name, "in_trash": null, "thumbnail_updated_at": null }),
    request_only
)]
pub async fn rename(client: &Client, channel_id: &str, name: &str) -> Result<Value>;

/// Set the cover to an uploaded picture: the user's choice, not the preview.
#[patch(
    "channel/{channel_id}",
    body = json!({
        "thumbnail": thumbnail,
        "in_trash": null,
        "thumbnail_updated_at": null,
        "thumbnail_fixed": true,
        "thumbnail_color": "#ffffff",
    })
)]
pub async fn set_cover(
    client: &Client,
    channel_id: &str,
    thumbnail: &str,
) -> Result<Value>;

/// Give up the chosen cover: the board shows its automatic preview again.
#[patch(
    "channel/{channel_id}",
    body = json!({
        "in_trash": null,
        "thumbnail_updated_at": null,
        "thumbnail_fixed": false,
        "thumbnail_color": null,
    })
)]
pub async fn reset_cover(client: &Client, channel_id: &str) -> Result<Value>;

/// Store a freshly drawn automatic preview after a reset.
#[patch(
    "channel/{channel_id}",
    body = json!({
        "thumbnail": thumbnail,
        "in_trash": null,
        "thumbnail_updated_at": null,
        "thumbnail_color": null,
    })
)]
pub async fn set_preview(
    client: &Client,
    channel_id: &str,
    thumbnail: &str,
) -> Result<Value>;

/// Delete: to the trash, or for good.
#[post(
    "channel/{channel_id}/delete",
    body = json!({ "permanently": mode.is_permanent() }),
    request_only
)]
pub async fn delete(
    client: &Client,
    channel_id: &str,
    mode: DeleteMode,
) -> Result<Value>;

/// Bring back from the trash. Not verified live.
#[post("channel/restore/{channel_id}", body = json!({}), request_only)]
pub async fn restore(client: &Client, channel_id: &str) -> Result<Value>;

/// Move into a folder or to the root, to the sidebar place `new_index`.
#[post(
        "channel/{channel_id}/move",
        body = json!({ "newParentId": parent.new_parent_id(), "newIndex": new_index }),
        request_only
    )]
pub async fn move_to(
    client: &Client,
    channel_id: &str,
    parent: &Parent,
    new_index: f64,
) -> Result<Value>;

/// Leave a board you were invited to. Not verified live.
#[post("channel/{channel_id}/leave", request_only)]
pub async fn leave(client: &Client, channel_id: &str) -> Result<Value>;

/// Create a board or folder by `author_id`, its id and time from `stamp`.
#[post(
        "channel/create",
        body = new.body(author_id, stamp, root_access)?,
        request_only
    )]
pub async fn create_stamped(
    client: &Client,
    new: &NewNode,
    root_access: Option<&str>,
    author_id: &str,
    stamp: &CreationStamp,
) -> Result<Value>;

/// A copy of a board; the copy's id and sidebar place come from `stamp`.
#[post(
        "channel/duplicate",
        body = CopyBody {
            source_channel_id: duplication.source_id,
            name: &duplication.target.name,
            parent: &duplication.target.parent,
            duplicate_permissions: duplication.permissions.is_copied(),
        }
        .body(&stamp.id, stamp.at.sidebar_order()),
        request_only
    )]
pub async fn duplicate_stamped(
    client: &Client,
    duplication: &NodeCopy<'_>,
    stamp: &CreationStamp,
) -> Result<Value>;
