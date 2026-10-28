//! The HTTP transport port: one exchange per call, no retries or redirects.

use std::{fmt, pin::Pin, sync::Arc, time::Duration};

use reqwest::{
    Url,
    header,
    header::{HeaderMap, HeaderValue},
};

use crate::error::{Error, Result, input::InputError};

/// An HTTP method the API uses; parsing ignores case.
///
/// ```
/// use holst_core::effect::http::HttpMethod;
///
/// assert_eq!("patch".parse::<HttpMethod>().ok(), Some(HttpMethod::Patch));
/// assert_eq!(HttpMethod::Delete.to_string(), "DELETE");
/// assert!("FETCH".parse::<HttpMethod>().is_err());
/// ```
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumIter,
    strum::EnumString,
)]
#[strum(
    serialize_all = "UPPERCASE",
    ascii_case_insensitive,
    parse_err_ty = Error,
    parse_err_fn = not_http_method
)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
    Put,
    Delete,
}

impl HttpMethod {
    /// The method on the wire: `GET`, `POST`, …
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Only a read may be repeated after a failure: no double write.
    #[must_use]
    pub const fn is_read(self) -> bool {
        matches!(self, HttpMethod::Get)
    }
}

fn not_http_method(raw: &str) -> Error {
    Error::Input(InputError::NotHttpMethod {
        raw: raw.to_owned(),
    })
}

/// A request body: bytes and their media type.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpBody {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl HttpBody {
    /// A JSON body.
    ///
    /// # Errors
    /// [`HttpError::Json`](crate::error::http::HttpError::Json) if the value
    /// does not serialize.
    pub fn json<T: serde::Serialize + ?Sized>(value: &T) -> Result<Self> {
        Ok(Self {
            bytes: serde_json::to_vec(value)?,
            content_type: "application/json".to_owned(),
        })
    }
}

// By hand: the bytes may hold a password.
impl fmt::Debug for HttpBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpBody")
            .field("content_type", &self.content_type)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// A request ready to go: a checked address and the login headers.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Option<HttpBody>,
}

/// A whole response.
#[derive(Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

// By hand: the body may hold a token and the headers a session cookie.
impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body", &self.body.len())
            .finish_non_exhaustive()
    }
}

impl HttpResponse {
    /// The body as text; invalid UTF-8 is replaced, as a browser does.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Every value of a header that reads as text.
    pub fn header_values<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .get_all(name)
            .iter()
            .filter_map(|value: &HeaderValue| value.to_str().ok())
    }

    /// The first value of a header that reads as text.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// A non-empty body that looks like a page, or non-JSON `text/html`.
    #[must_use]
    pub fn is_html(&self) -> bool {
        let body = self.body.trim_ascii_start();
        let declared = self
            .header(header::CONTENT_TYPE.as_str())
            .is_some_and(|kind| {
                kind.trim_start()
                    .to_ascii_lowercase()
                    .starts_with("text/html")
            });
        !body.is_empty()
            && (starts_like_html(body) || (declared && !starts_like_json(body)))
    }
}

fn starts_like_json(body: &[u8]) -> bool {
    matches!(body.first(), Some(b'{' | b'[' | b'"'))
}

fn starts_like_html(body: &[u8]) -> bool {
    let head = body.trim_ascii_start();
    [b"<!doctype html".as_slice(), b"<html".as_slice()]
        .iter()
        .any(|mark| {
            head.get(..mark.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(mark))
        })
}

/// A future that can be stored behind `dyn`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Sending a request: one exchange.
pub trait Transport: Send + Sync + 'static {
    /// Send the request and read the whole response, whatever the status.
    ///
    /// # Errors
    /// [`HttpError::Transport`](crate::error::http::HttpError::Transport);
    /// [`HttpError::NoResponse`](crate::error::http::HttpError::NoResponse) for
    /// a write that broke after it was sent;
    /// [`HttpError::ResponseTooLarge`](crate::error::http::HttpError::ResponseTooLarge)
    /// over the limit.
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, Result<HttpResponse>>;

    /// The same transport reading a reply within `timeout` and at most `limit`
    /// bytes: a larger reply stops early. A transport that does not touch the
    /// network may keep the default, which returns itself unchanged.
    ///
    /// # Errors
    /// [`HttpError::Transport`](crate::error::http::HttpError::Transport) if
    /// the limited transport cannot be built.
    fn limited(
        self: Arc<Self>,
        _timeout: Duration,
        _limit: usize,
    ) -> Result<Arc<dyn Transport>> {
        Ok(Arc::new(self))
    }
}

impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, Result<HttpResponse>> {
        (**self).send(request)
    }

    fn limited(
        self: Arc<Self>,
        timeout: Duration,
        limit: usize,
    ) -> Result<Arc<dyn Transport>> {
        Arc::clone(&*self).limited(timeout, limit)
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderMap, HeaderValue};
    use rstest::rstest;

    use crate::{
        effect::http::{HttpBody, HttpMethod, HttpResponse, header},
        error::{Error, input::InputError},
    };

    #[test]
    fn debug_of_a_body_and_a_response_shows_no_content() {
        let body = HttpBody {
            bytes: b"s3cr3t".to_vec(),
            content_type: "application/json".into(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            "set-cookie",
            HeaderValue::from_static("sessionToken=c00k1e"),
        );
        let response = HttpResponse {
            status: 200,
            headers,
            body: b"t0ken".to_vec(),
        };
        assert_eq!(
            format!("{body:?} {response:?}"),
            r#"HttpBody { content_type: "application/json", bytes: 6 } HttpResponse { status: 200, body: 5, .. }"#
        );
    }

    #[rstest]
    #[case::lower("patch", Some(HttpMethod::Patch))]
    #[case::upper("GET", Some(HttpMethod::Get))]
    #[case::unknown("options", None)]
    fn method_parse_is_case_insensitive(
        #[case] raw: &str,
        #[case] expected: Option<HttpMethod>,
    ) {
        assert_eq!(raw.parse::<HttpMethod>().ok(), expected);
    }

    #[test]
    fn unknown_method_error_names_it() {
        let err = "FETCH".parse::<HttpMethod>().err();
        assert!(
            matches!(&err, Some(Error::Input(InputError::NotHttpMethod { .. }))),
            "{err:?}"
        );
        assert!(format!("{err:?}").contains(r#"raw: "FETCH""#), "{err:?}");
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Kind {
        Html,
        Data,
    }

    fn page(content_type: Option<&str>, body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: content_type
                .and_then(|kind| HeaderValue::from_str(kind).ok())
                .map(|kind| (header::CONTENT_TYPE, kind))
                .into_iter()
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[rstest]
    #[case::html_type(Some("text/html; charset=utf-8"), "<h1>Down</h1>", Kind::Html)]
    // Empty replies often carry a default `text/html`: not a page.
    #[case::empty_with_html_type(Some("text/html; charset=utf-8"), "", Kind::Data)]
    #[case::blank_with_html_type(Some("text/html"), " \n", Kind::Data)]
    // The header lies; the body is data.
    #[case::json_with_html_type(Some("text/html"), r#"{"ok":true}"#, Kind::Data)]
    #[case::json_type(Some("application/json"), r#"{"a":1}"#, Kind::Data)]
    // `<html>` is never JSON: the page wins over a lying header.
    #[case::page_typed_as_json(Some("application/json"), "<html>", Kind::Html)]
    #[case::sniffed_doctype(None, "  <!DOCTYPE html><title>down</title>", Kind::Html)]
    #[case::sniffed_html(None, "<HTML>", Kind::Html)]
    #[case::json_body(None, r#"{"a":1}"#, Kind::Data)]
    #[case::empty(None, "", Kind::Data)]
    fn html_pages_are_recognised(
        #[case] content_type: Option<&str>,
        #[case] body: &str,
        #[case] kind: Kind,
    ) {
        assert_eq!(page(content_type, body).is_html(), kind == Kind::Html);
    }
}
