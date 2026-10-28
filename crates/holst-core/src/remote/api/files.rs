use holst_macros::post;

use crate::{
    domain::response::Uploaded,
    effect::http::HttpBody,
    error::Result,
    protocol::upload::multipart_body,
    remote::client::Client,
};

/// A file and the boundary line that delimits it in a multipart body.
#[derive(Debug, Clone, Copy)]
pub struct FilePart<'a> {
    pub boundary: &'a str,
    pub file_name: &'a str,
    /// The MIME type the part carries, e.g. `image/png`.
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

/// What an upload is bound to; the server wants exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadBind<'a> {
    /// A board, by id.
    Channel(&'a str),
    /// A user, by id.
    User(&'a str),
    /// A workspace, by id.
    Workspace(&'a str),
}

impl<'a> UploadBind<'a> {
    const fn pair(self) -> (&'static str, &'a str) {
        match self {
            UploadBind::Channel(id) => ("channel_id", id),
            UploadBind::User(id) => ("user_id", id),
            UploadBind::Workspace(id) => ("workspace_id", id),
        }
    }
}

/// The parts of an upload besides the file and its bind.
#[derive(Debug, Clone, Copy)]
pub struct UploadOptions<'a> {
    /// The key the file is stored under; a fresh id when none is given.
    pub key: Option<&'a str>,
    /// How long the server keeps the file fresh, e.g. `0`.
    pub freshness: Option<&'a str>,
    /// The MIME type of the file.
    pub content_type: &'a str,
}

fn multipart(part: &FilePart<'_>) -> HttpBody {
    HttpBody {
        bytes: multipart_body(
            part.boundary,
            "file",
            part.file_name,
            part.content_type,
            part.bytes,
        ),
        content_type: format!("multipart/form-data; boundary={}", part.boundary),
    }
}

fn upload_query<'a>(
    bind: UploadBind<'a>,
    key: &'a str,
    freshness: Option<&'a str>,
) -> impl Iterator<Item = (&'static str, &'a str)> {
    [bind.pair(), ("key", key)]
        .into_iter()
        .chain(freshness.map(|duration| ("freshness_duration", duration)))
}

/// Upload a file between `boundary` lines, stored under `key`.
#[post(
        "upload_file",
        query = upload_query(bind, key, freshness),
        payload = multipart(part),
        response = typed
    )]
pub async fn upload_parts(
    client: &Client,
    bind: UploadBind<'_>,
    key: &str,
    freshness: Option<&str>,
    part: &FilePart<'_>,
) -> Result<Uploaded>;

/// Upload a file as `multipart/form-data` with a fresh boundary.
///
/// # Errors
/// [`Client::send_json`] errors.
pub async fn upload(
    client: &Client,
    bind: UploadBind<'_>,
    options: UploadOptions<'_>,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<Uploaded> {
    let boundary = format!("----holst-rs-{}", client.new_id());
    let key = options.key.map_or_else(|| client.new_id(), str::to_owned);
    let part = FilePart {
        boundary: &boundary,
        file_name,
        content_type: options.content_type,
        bytes: &bytes,
    };
    upload_parts(client, bind, &key, options.freshness, &part).await
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use crate::{
        Result,
        remote::api::files::{FilePart, UploadBind, upload_parts_request},
    };

    #[test]
    fn a_request_carries_the_key_and_exactly_one_bind() -> Result<()> {
        let part = FilePart {
            boundary: "B",
            file_name: "a.png",
            content_type: "image/png",
            bytes: b"x",
        };
        let pairs = |bind, freshness| -> Result<Vec<(String, String)>> {
            Ok(upload_parts_request(bind, "k", freshness, &part)?
                .query
                .into_iter()
                .map(|parameter| (parameter.name, parameter.value))
                .collect())
        };
        let pair = |name: &str, value: &str| (name.to_owned(), value.to_owned());
        assert_eq!(
            [
                pairs(UploadBind::Channel("c"), None)?,
                pairs(UploadBind::User("u"), None)?,
                pairs(UploadBind::Workspace("w"), Some("0"))?,
            ],
            [
                vec![pair("channel_id", "c"), pair("key", "k")],
                vec![pair("user_id", "u"), pair("key", "k")],
                vec![
                    pair("workspace_id", "w"),
                    pair("key", "k"),
                    pair("freshness_duration", "0")
                ],
            ]
        );
        Ok(())
    }
}
