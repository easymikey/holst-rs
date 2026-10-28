//! A REST request as data: what an endpoint sends, before login and network.

use std::fmt;

use reqwest::Url;
use serde::Serialize;

use crate::{
    domain::url::BaseUrl,
    effect::http::{HttpBody, HttpMethod},
    error::Result,
};

pub const DRY_RUN: &str = "dry run, nothing sent:";

/// Whether a request carries the login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    Session,
    Anonymous,
    Unsigned,
}

/// One `name=value` pair of a request query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParameter {
    pub name: String,
    pub value: String,
}

/// On the wire a pair: `["name", "value"]`.
impl Serialize for QueryParameter {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        (&self.name, &self.value).serialize(serializer)
    }
}

/// A request an endpoint sends, as data: shown by a dry run, sent by
/// [`crate::remote::client::Client::send_json`].
///
/// ```
/// use holst_core::{effect::http::HttpMethod, remote::api::channel};
///
/// let request = channel::rename_request("b1", "Plans")?;
/// assert_eq!(
///     (request.method, request.path.as_str()),
///     (HttpMethod::Patch, "/gapi/channel/b1")
/// );
/// assert_eq!(
///     request.to_string(),
///     "PATCH /gapi/channel/b1\n{\"in_trash\":null,\"name\":\"Plans\",\"thumbnail_updated_at\":null}"
/// );
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct ApiRequest {
    pub method: HttpMethod,
    pub path: String,
    pub query: Vec<QueryParameter>,
    pub body: Option<HttpBody>,
    pub authorization: Authorization,
}

impl ApiRequest {
    /// The full address on `base`, the query encoded; without a query no `?`.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotApiPath`] or
    /// [`crate::error::input::InputError::NotUrl`] as [`BaseUrl::url_for`].
    pub fn url(&self, base: &BaseUrl) -> Result<Url> {
        base.url_for(&self.path)
            .map(|url| with_query(url, &self.query))
    }

    /// Whether the body must not be shown: a login body holds the password.
    #[must_use]
    pub fn hides_body(&self) -> bool {
        self.authorization == Authorization::Anonymous
            && self.path.starts_with("/gapi/login")
    }
}

pub(super) fn with_query(url: Url, query: &[QueryParameter]) -> Url {
    Some(query)
        .filter(|query| !query.is_empty())
        .into_iter()
        .fold(url, |mut url, query| {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|pair| (&pair.name, &pair.value)));
            url
        })
}

impl fmt::Debug for ApiRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keys: Vec<&str> =
            self.query.iter().map(|pair| pair.name.as_str()).collect();
        f.debug_struct("ApiRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("query", &keys)
            .field(
                "body_bytes",
                &self.body.as_ref().map(|body| body.bytes.len()),
            )
            .field("authorization", &self.authorization)
            .finish()
    }
}

impl fmt::Display for ApiRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path)?;
        self.query.iter().enumerate().try_for_each(
            |(position, QueryParameter { name, value })| {
                let mark = match position {
                    0 => '?',
                    _ => '&',
                };
                write!(f, "{mark}{name}={value}")
            },
        )?;
        let secret = self.hides_body();
        self.body
            .as_ref()
            .map_or(Ok(()), |body| match body.content_type.as_str() {
                "application/json" if !secret => {
                    write!(f, "\n{}", String::from_utf8_lossy(&body.bytes))
                }
                other => write!(f, "\n({} bytes of {other})", body.bytes.len()),
            })
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use serde_json::json;

    use crate::{
        domain::url::{BaseUrl, DEFAULT_BASE},
        effect::{
            http::{HttpBody, HttpMethod},
            request::{ApiRequest, Authorization, QueryParameter},
        },
        error::Result,
    };

    fn request(query: &[(&str, &str)], body: Option<HttpBody>) -> ApiRequest {
        ApiRequest {
            method: HttpMethod::Post,
            path: "/gapi/x".into(),
            query: query
                .iter()
                .map(|&(name, value)| QueryParameter {
                    name: name.to_owned(),
                    value: value.to_owned(),
                })
                .collect(),
            body,
            authorization: Authorization::Session,
        }
    }

    #[test]
    fn display_shows_the_query_and_a_json_body() -> Result<()> {
        let json = HttpBody::json(&json!({ "a": 1 }))?;
        assert_eq!(
            request(&[("k", "a b"), ("n", "1")], Some(json)).to_string(),
            "POST /gapi/x?k=a b&n=1\n{\"a\":1}"
        );
        let bytes = HttpBody {
            bytes: vec![0; 3],
            content_type: "application/octet-stream".into(),
        };
        assert_eq!(
            request(&[], Some(bytes)).to_string(),
            "POST /gapi/x\n(3 bytes of application/octet-stream)"
        );
        Ok(())
    }

    #[test]
    fn only_the_login_hides_its_body() {
        let mut login = request(&[], None);
        login.path = "/gapi/login/with_email".into();
        assert!(!login.hides_body());
        login.authorization = Authorization::Anonymous;
        assert!(login.hides_body());
        login.path = "/gapi/x".into();
        assert!(!login.hides_body());
    }

    #[test]
    fn debug_shows_no_values() -> Result<()> {
        let secret = HttpBody::json(&json!({ "password": "s3cr3t" }))?;
        let shown = format!("{:?}", request(&[("key", "t0ken")], Some(secret)));
        assert!(
            !shown.contains("s3cr3t") && !shown.contains("t0ken"),
            "{shown}"
        );
        assert!(shown.contains("\"key\""), "{shown}");
        Ok(())
    }

    #[test]
    fn url_encodes_the_query_and_skips_an_empty_one() -> Result<()> {
        let base: BaseUrl = DEFAULT_BASE.parse()?;
        assert_eq!(
            request(&[("q", "a b")], None).url(&base)?.as_str(),
            "https://app.holst.so/gapi/x?q=a+b"
        );
        assert_eq!(
            request(&[], None).url(&base)?.as_str(),
            "https://app.holst.so/gapi/x"
        );
        Ok(())
    }
}
