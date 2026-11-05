//! The server's reply and the transport under it.

use std::fmt;

use crate::error::{Error, hint::Hint, input::InputError};

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

/// What the server put in the body of a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyBody {
    Empty,
    /// A page for people, not the API: a wrong address or a filter.
    Html,
    Text(String),
}

impl ReplyBody {
    /// `Empty` for a blank text, else the text.
    #[must_use]
    pub fn text(text: String) -> Self {
        if text.trim().is_empty() {
            ReplyBody::Empty
        } else {
            ReplyBody::Text(text)
        }
    }
}

/// The tail of a status line: `: body`, cut at 400 characters, or nothing.
impl fmt::Display for ReplyBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplyBody::Empty => Ok(()),
            ReplyBody::Html => f.write_str(": (an HTML page)"),
            ReplyBody::Text(text) => {
                let text = text.trim();
                let head = text
                    .char_indices()
                    .nth(400)
                    .and_then(|(cut, _)| text.get(..cut));
                match (text, head) {
                    ("", _) => Ok(()),
                    (_, Some(head)) => write!(f, ": {head}…"),
                    (text, None) => write!(f, ": {text}"),
                }
            }
        }
    }
}

/// A reply that is a refusal, not what was asked, or never came.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum HttpError {
    #[error("{method} {path} -> HTTP {status}{body}")]
    Status {
        method: HttpMethod,
        path: String,
        status: u16,
        body: ReplyBody,
    },

    #[error(
        "{method} {path}: no answer from the server — the change may or may not have been applied"
    )]
    NoResponse { method: HttpMethod, path: String },

    #[error("the server answered with an HTML page instead of JSON (HTTP {status})")]
    HtmlInsteadOfJson { status: u16 },

    #[error("the response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },

    #[error(
        "refused to follow a redirect to {location}: the session is sent only to its own server"
    )]
    ForeignRedirect { location: String },

    #[error(
        "{method} {path}: rate-limited by the server; try again in {} s",
        .retry_after.as_secs()
    )]
    RateLimited {
        method: HttpMethod,
        path: String,
        retry_after: std::time::Duration,
    },

    #[error("the {response} reply has no {field}")]
    MissingField {
        response: &'static str,
        field: &'static str,
    },

    #[error("the server sent JSON instead of a backup (perhaps a link to it): {head}")]
    JsonInsteadOfBackup { head: String },

    #[error("the server did not answer in time")]
    Timeout(#[source] reqwest::Error),

    #[error("could not connect to the server")]
    Connect(#[source] reqwest::Error),

    #[error("the server's reply was cut off or unreadable")]
    CutOff(#[source] reqwest::Error),

    #[error("the request could not be built")]
    Build(#[source] reqwest::Error),

    #[error("network error")]
    Network(#[source] reqwest::Error),

    #[error("the reply is not valid JSON")]
    Json(#[from] serde_json::Error),
}

impl From<reqwest::Error> for HttpError {
    fn from(error: reqwest::Error) -> Self {
        match error {
            e if e.is_timeout() => HttpError::Timeout(e),
            e if e.is_connect() => HttpError::Connect(e),
            e if e.is_decode() || e.is_body() => HttpError::CutOff(e),
            e if e.is_builder() => HttpError::Build(e),
            e => HttpError::Network(e),
        }
    }
}

impl HttpError {
    /// What to do next, decided for every variant.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            HttpError::Status { status: 401, .. } => Some(Hint::LoginAgain),
            // An HTML refusal is a wrong address or a filter, not the API.
            HttpError::Status {
                status: 400 | 403 | 404 | 405,
                body: ReplyBody::Html,
                ..
            }
            | HttpError::Timeout(_)
            | HttpError::Connect(_)
            | HttpError::CutOff(_)
            | HttpError::Build(_)
            | HttpError::Network(_)
            | HttpError::HtmlInsteadOfJson { .. } => Some(Hint::CheckNetwork),
            HttpError::Status { status: 403, .. } => Some(Hint::AskOwner),
            HttpError::Status { status: 404, .. } => Some(Hint::CheckId),
            HttpError::Status {
                status: 400 | 422, ..
            } => Some(Hint::CheckRequest),
            // Only overload and gateway failures pass; a 500 repeats.
            HttpError::Status {
                status: 429 | 502..=504,
                ..
            } => Some(Hint::RetryLater),
            HttpError::Status {
                status: 500..=599, ..
            } => Some(Hint::ServerFailed),
            HttpError::RateLimited { .. } => Some(Hint::WaitAsAsked),
            HttpError::NoResponse { .. } => Some(Hint::CheckBeforeRetry),
            HttpError::Status { .. }
            | HttpError::ResponseTooLarge { .. }
            | HttpError::ForeignRedirect { .. }
            | HttpError::MissingField { .. }
            | HttpError::JsonInsteadOfBackup { .. }
            | HttpError::Json(_) => None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::{net::TcpListener, time::Duration};

    use rstest::rstest;
    use strum::IntoEnumIterator as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use crate::{
        error::{
            hint::{Hint, Surface, decided::assert_every_variant},
            http::{HttpError, HttpErrorDiscriminants, HttpMethod, ReplyBody},
        },
        fake::http_status,
    };

    type Kind = fn(reqwest::Error) -> HttpError;

    pub(in crate::error) fn status(code: u16, body: &str) -> HttpError {
        http_status(code, ReplyBody::text(body.to_owned()))
    }

    #[rstest]
    #[case::unauthorized(status(401, ""), Some(Hint::LoginAgain))]
    #[case::forbidden(status(403, ""), Some(Hint::AskOwner))]
    #[case::gone(status(404, ""), Some(Hint::CheckId))]
    #[case::rate_limited(status(429, ""), Some(Hint::RetryLater))]
    #[case::unavailable(status(503, ""), Some(Hint::RetryLater))]
    #[case::bad_gateway(status(502, ""), Some(Hint::RetryLater))]
    #[case::gateway_timeout(status(504, ""), Some(Hint::RetryLater))]
    #[case::bad_request(status(400, ""), Some(Hint::CheckRequest))]
    #[case::internal(status(500, ""), Some(Hint::ServerFailed))]
    #[case::not_implemented(status(501, ""), Some(Hint::ServerFailed))]
    #[case::teapot(status(418, ""), None)]
    #[case::html_404_is_a_wrong_address(
        http_status(404, ReplyBody::Html),
        Some(Hint::CheckNetwork)
    )]
    #[case::html_403_is_a_filter(
        http_status(403, ReplyBody::Html),
        Some(Hint::CheckNetwork)
    )]
    #[case::html_502_is_still_a_gateway(
        http_status(502, ReplyBody::Html),
        Some(Hint::RetryLater)
    )]
    #[case::the_words_of_a_stand_in_are_only_words(
        http_status(404, ReplyBody::Text("(an HTML page)".into())),
        Some(Hint::CheckId)
    )]
    fn status_hints_point_to_the_next_step(
        #[case] err: HttpError,
        #[case] hint: Option<Hint>,
    ) {
        assert_eq!(err.hint(), hint);
    }

    /// The URL of a server that writes `reply` to its first connection, then
    /// hangs up, or keeps the connection open.
    async fn serve(reply: &'static [u8], hang_up: bool) -> String {
        let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
            return String::new();
        };
        let url = listener
            .local_addr()
            .map(|address| format!("http://{address}/"))
            .unwrap_or_default();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let _request: usize =
                    stream.read(&mut [0; 1024]).await.unwrap_or_default();
                drop(stream.write_all(reply).await);
                if !hang_up {
                    std::future::pending::<()>().await;
                }
            }
        });
        url
    }

    const QUICK: Duration = Duration::from_millis(300);

    /// Windows retries a refused connect for about 2 s before it reports it.
    const WINDOWS_RETRY: Duration = Duration::from_secs(5);

    async fn failure(url: &str, timeout: Duration) -> Option<HttpError> {
        let client = reqwest::Client::builder().timeout(timeout).build().ok()?;
        match client.get(url).send().await {
            Ok(reply) => reply.bytes().await.err().map(HttpError::from),
            Err(error) => Some(error.into()),
        }
    }

    #[tokio::test]
    async fn a_silent_server_is_a_timeout() {
        let err = failure(&serve(b"", false).await, QUICK).await;
        assert!(matches!(err, Some(HttpError::Timeout(_))), "{err:?}");
    }

    #[tokio::test]
    async fn a_closed_port_is_a_failed_connection() {
        let gone = TcpListener::bind("127.0.0.1:0")
            .ok()
            .and_then(|listener| listener.local_addr().ok())
            .map(|address| format!("http://{address}/"))
            .unwrap_or_default();
        let err = failure(&gone, WINDOWS_RETRY).await;
        assert!(matches!(err, Some(HttpError::Connect(_))), "{err:?}");
    }

    #[tokio::test]
    async fn a_body_cut_short_is_a_cut_off_reply() {
        let url =
            serve(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc", true).await;
        let err = failure(&url, QUICK).await;
        assert!(matches!(err, Some(HttpError::CutOff(_))), "{err:?}");
    }

    #[test]
    fn a_request_that_cannot_be_built_is_a_build_error() {
        let err = build_error();
        assert!(matches!(err, Some(HttpError::Build(_))), "{err:?}");
    }

    fn reqwest_error() -> Option<reqwest::Error> {
        reqwest::Client::new().get("http://[").build().err()
    }

    fn build_error() -> Option<HttpError> {
        reqwest_error().map(HttpError::from)
    }

    pub(in crate::error) fn samples() -> Vec<(HttpError, Option<Hint>)> {
        let unknown = HttpError::NoResponse {
            method: HttpMethod::Post,
            path: "/gapi/channel/create".into(),
        };
        let rate_limited = HttpError::RateLimited {
            method: HttpMethod::Get,
            path: "/x".into(),
            retry_after: Duration::from_secs(1),
        };
        let json = serde_json::from_str::<u8>("x")
            .map_err(HttpError::Json)
            .err();
        let kinds: [Kind; 5] = [
            HttpError::Timeout,
            HttpError::Connect,
            HttpError::CutOff,
            HttpError::Build,
            HttpError::Network,
        ];
        let transport = kinds
            .into_iter()
            .filter_map(|kind| reqwest_error().map(kind))
            .map(|err| (err, Some(Hint::CheckNetwork)));
        let foreign = [(json, None)]
            .into_iter()
            .filter_map(|(err, hint)| err.map(|err| (err, hint)));
        [
            (status(404, ""), Some(Hint::CheckId)),
            (unknown, Some(Hint::CheckBeforeRetry)),
            (
                HttpError::HtmlInsteadOfJson { status: 502 },
                Some(Hint::CheckNetwork),
            ),
            (HttpError::ResponseTooLarge { limit: 1 }, None),
            (
                HttpError::ForeignRedirect {
                    location: "x".into(),
                },
                None,
            ),
            (rate_limited, Some(Hint::WaitAsAsked)),
            (
                HttpError::MissingField {
                    response: "x",
                    field: "y",
                },
                None,
            ),
            (HttpError::JsonInsteadOfBackup { head: "{".into() }, None),
        ]
        .into_iter()
        .chain(foreign)
        .chain(transport)
        .collect()
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, HttpErrorDiscriminants>(samples(), HttpError::hint);
    }

    #[test]
    fn status_error_shows_trimmed_body_or_nothing() {
        assert_eq!(
            status(500, "  ").to_string(),
            "GET /gapi/current_user -> HTTP 500"
        );
        assert_eq!(
            status(500, " oops ").to_string(),
            "GET /gapi/current_user -> HTTP 500: oops"
        );
        let long = status(500, &"я".repeat(401)).to_string();
        assert!(long.ends_with('…'), "{long}");
        assert_eq!(
            http_status(404, ReplyBody::Html).to_string(),
            "GET /gapi/current_user -> HTTP 404: (an HTML page)"
        );
    }

    #[test]
    fn rate_limit_hint_does_not_contradict_the_pause() {
        let err = HttpError::RateLimited {
            method: HttpMethod::Get,
            path: "/x".into(),
            retry_after: Duration::from_mins(2),
        };
        let hints: Vec<String> = Surface::iter()
            .filter_map(|surface| err.hint().map(|hint| hint.text(surface).to_string()))
            .collect();
        assert!(err.to_string().contains("120 s"), "{err}");
        assert!(
            hints
                .iter()
                .all(|hint| !hint.contains("minute") && !hint.is_empty()),
            "{hints:?}"
        );
    }
}
