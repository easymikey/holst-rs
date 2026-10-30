use holst_macros::{get, post};
use serde::{Deserialize, Serialize};

use crate::{
    domain::id::BoardId,
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

/// The body of `recent/set`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRecent {
    pub channel_id: BoardId,
}

/// One page of recent boards, newest first.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentPage {
    #[serde(default)]
    pub channels: Vec<RecentEntry>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub channel_id: String,
}

/// Mark a board as recent.
#[post("recent/set", class = Write, request_only)]
pub async fn set(client: &Client, #[body] body: &SetRecent) -> Result<Done>;

/// A page of recent boards; the first one without a `cursor`.
#[get("recent/channels/page", class = Read, query(pageSize = page_size, cursor? = cursor), response = typed)]
pub async fn page(
    client: &Client,
    page_size: u32,
    cursor: Option<&str>,
) -> Result<RecentPage>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[SET, PAGE];
