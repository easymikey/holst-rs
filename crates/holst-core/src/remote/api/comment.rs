use holst_macros::{get, post};
use serde::{Deserialize, Serialize};

use crate::{
    domain::board::comment::ReadMark,
    error::Result,
    remote::{api::Done, catalogue::Endpoint, client::Client},
};

/// `comment/set_read_at`; `read_at` is e.g. `2026-10-21T10:05:45.404Z`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetReadAt {
    pub channel_id: String,
    pub thread_id: String,
    pub read_at: String,
}

/// When threads were read.
#[get(
    "comment/thread_list/{channel_id}",
    class = Read,
    query(channelId = channel_id),
    response = typed
)]
pub async fn thread_list(client: &Client, channel_id: &str) -> Result<Vec<ReadMark>>;

/// Mark a thread as read.
#[post("comment/set_read_at", class = Write, response = typed)]
pub async fn set_read_at_time(
    client: &Client,
    #[body] body: &SetReadAt,
) -> Result<Done>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[THREAD_LIST, SET_READ_AT_TIME];
