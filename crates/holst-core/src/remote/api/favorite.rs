use holst_macros::{delete, get, post};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

/// The favorites of a workspace.
#[get("favorite_channel/channels/{workspace_id}", class = Read)]
pub async fn list(client: &Client, workspace_id: &str) -> Result<Value>;

/// The body of `favorite_channel/add`; `index: -1` — at the end.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddFavorite {
    pub channel_id: String,
    pub index: i64,
}

/// The body of `favorite_channel/delete`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveFavorite {
    pub channel_id: String,
}

/// Add a favorite.
#[post("favorite_channel/add", class = Write, request_only)]
pub async fn add(client: &Client, #[body] body: &AddFavorite) -> Result<Value>;

/// Remove a favorite. Not verified live.
#[delete("favorite_channel/delete", class = Write, request_only)]
pub async fn remove(client: &Client, #[body] body: &RemoveFavorite) -> Result<Value>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[LIST, ADD, REMOVE];
