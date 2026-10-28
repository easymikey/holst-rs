use holst_macros::get;

use crate::{domain::response::LinkPreview, error::Result, remote::client::Client};

/// A link's title and picture; the parameter is `link`, as in the web
/// client.
#[get("link_preview", query(link = encode_uri(url)), response = typed)]
pub async fn link_preview(client: &Client, url: &str) -> Result<LinkPreview>;

/// `encodeURI`: the web client encodes the link, and the request encodes the
/// result once more.
fn encode_uri(url: &str) -> String {
    url.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b";,/?:@&=+$-_.!~*'()#".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
