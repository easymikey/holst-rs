use holst_macros::{get, post};
use serde_json::{Value, json};

use crate::{error::Result, remote::client::Client};

/// Mark a board as recent.
#[post("recent/set", body = json!({ "channelId": channel_id }), request_only)]
pub async fn set(client: &Client, channel_id: &str) -> Result<Value>;

/// A page of recent boards; the first one without a `cursor`.
#[get("recent/channels/page", query(pageSize = page_size, cursor? = cursor))]
pub async fn page(
    client: &Client,
    page_size: u32,
    cursor: Option<&str>,
) -> Result<Value>;
