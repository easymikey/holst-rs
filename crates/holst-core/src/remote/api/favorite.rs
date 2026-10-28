use holst_macros::{delete, get, post};
use serde_json::{Value, json};

use crate::{error::Result, remote::client::Client};

/// The favorites of a workspace.
#[get("favorite_channel/channels/{workspace_id}")]
pub async fn list(client: &Client, workspace_id: &str) -> Result<Value>;

/// Add a favorite; `index: -1` — at the end.
#[post("favorite_channel/add", body = json!({ "channelId": channel_id, "index": index }), request_only)]
pub async fn add(client: &Client, channel_id: &str, index: i64) -> Result<Value>;

/// Remove a favorite. Not verified live.
#[delete("favorite_channel/delete", body = json!({ "channelId": channel_id }), request_only)]
pub async fn remove(client: &Client, channel_id: &str) -> Result<Value>;
