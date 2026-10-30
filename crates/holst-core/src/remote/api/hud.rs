use holst_macros::post;
use serde_json::Value;

use crate::{effect::http::HttpBody, error::Result, remote::client::Client};

#[cfg(test)]
pub(crate) const UPLOAD_PATH: &str = "/gapi/hud/upload";

fn octet_stream(bytes: Vec<u8>) -> HttpBody {
    HttpBody {
        bytes,
        content_type: "application/octet-stream".to_owned(),
    }
}

/// A board update, sent before its socket frame.
#[post(
    "hud/upload",
    unlisted,
    query(documentName = board, updateId = id),
    payload = octet_stream(update)
)]
pub async fn upload(
    client: &Client,
    board: &str,
    id: u64,
    update: Vec<u8>,
) -> Result<Value>;
