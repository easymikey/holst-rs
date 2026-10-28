use holst_macros::{get, post};
use serde_json::{Value, json};

use crate::{error::Result, remote::client::Client};

/// When threads were read.
#[get("comment/thread_list/{channel_id}", query(channelId = channel_id))]
pub async fn thread_list(client: &Client, channel_id: &str) -> Result<Value>;

/// Mark a thread as read at `read_at`, e.g. `2026-10-21T10:05:45.404Z`.
#[post(
        "comment/set_read_at",
        body = json!({ "channelId": channel_id, "threadId": thread_id, "readAt": read_at })
    )]
pub async fn set_read_at_time(
    client: &Client,
    channel_id: &str,
    thread_id: &str,
    read_at: &str,
) -> Result<Value>;
