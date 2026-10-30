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

/// A reply that is a refusal, not what was asked, or never came.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum HttpError {
    #[error("{method} {path} -> HTTP {status}{}", display_body(.detail))]
    Status {
        method: &'static str,
        path: String,
        status: u16,
        detail: String,
    },

    #[error(
        "{method} {path}: no answer from the server — the change may or may not have been applied"
    )]
    NoResponse { method: &'static str, path: String },

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
        method: &'static str,
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

    #[error("{}", http_summary(.0))]
    Transport(#[from] reqwest::Error),

    #[error("the reply is not valid JSON")]
    Json(#[from] serde_json::Error),
}

impl HttpError {
    /// What to do next, decided for every variant.
    #[must_use]
    pub fn hint(&self) -> Option<Hint> {
        match self {
            HttpError::Status { status: 401, .. } => Some(Hint::LoginAgain),
            // An HTML refusal is a wrong address or a filter, not the API.
            HttpError::Status {
                status: 400 | 403 | 404 | 405,
                detail: body,
                ..
            } if body == HTML_BODY => Some(Hint::CheckNetwork),
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
            HttpError::Transport(_) | HttpError::HtmlInsteadOfJson { .. } => {
                Some(Hint::CheckNetwork)
            }
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

const CUT_OFF: &str = "the server's reply was cut off or unreadable";

fn http_summary(err: &reqwest::Error) -> &'static str {
    match err {
        e if e.is_timeout() => "the server did not answer in time",
        e if e.is_connect() => "could not connect to the server",
        e if e.is_decode() || e.is_body() => CUT_OFF,
        e if e.is_builder() => "the request could not be built",
        _ => "network error",
    }
}

fn display_body(body: &str) -> impl fmt::Display {
    let body = body.trim();
    let head = body
        .char_indices()
        .nth(400)
        .and_then(|(cut, _)| body.get(..cut));
    fmt::from_fn(move |f| match (body, head) {
        ("", _) => Ok(()),
        (_, Some(head)) => write!(f, ": {head}…"),
        (body, None) => write!(f, ": {body}"),
    })
}

pub(crate) const HTML_BODY: &str = "(an HTML page)";

#[cfg(test)]
pub(super) mod tests {
    use rstest::rstest;
    use strum::IntoEnumIterator as _;

    use crate::error::{
        hint::{Hint, Surface, decided::assert_every_variant},
        http::{HttpError, HttpErrorDiscriminants},
    };

    pub(in crate::error) fn status(status: u16, body: &str) -> HttpError {
        HttpError::Status {
            method: "GET",
            path: "/x".into(),
            status,
            detail: body.into(),
        }
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
        status(404, crate::error::http::HTML_BODY),
        Some(Hint::CheckNetwork)
    )]
    #[case::html_403_is_a_filter(
        status(403, crate::error::http::HTML_BODY),
        Some(Hint::CheckNetwork)
    )]
    #[case::html_502_is_still_a_gateway(
        status(502, crate::error::http::HTML_BODY),
        Some(Hint::RetryLater)
    )]
    fn status_hints_point_to_the_next_step(
        #[case] err: HttpError,
        #[case] hint: Option<Hint>,
    ) {
        assert_eq!(err.hint(), hint);
    }

    pub(in crate::error) fn samples() -> Vec<(HttpError, Option<Hint>)> {
        let unknown = HttpError::NoResponse {
            method: "POST",
            path: "/gapi/channel/create".into(),
        };
        let rate_limited = HttpError::RateLimited {
            method: "GET",
            path: "/x".into(),
            retry_after: std::time::Duration::from_secs(1),
        };
        let json = serde_json::from_str::<u8>("x")
            .map_err(HttpError::Json)
            .err();
        let transport = reqwest::Client::builder()
            .build()
            .and_then(|client| client.get("http://[").build())
            .map_err(HttpError::Transport)
            .err();
        let foreign = [(json, None), (transport, Some(Hint::CheckNetwork))]
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
        .collect()
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, HttpErrorDiscriminants>(samples(), HttpError::hint);
    }

    #[test]
    fn status_error_shows_trimmed_body_or_nothing() {
        assert_eq!(status(500, "  ").to_string(), "GET /x -> HTTP 500");
        assert_eq!(
            status(500, " oops ").to_string(),
            "GET /x -> HTTP 500: oops"
        );
        let long = status(500, &"я".repeat(401)).to_string();
        assert!(long.ends_with('…'), "{long}");
    }

    #[test]
    fn rate_limit_hint_does_not_contradict_the_pause() {
        let err = HttpError::RateLimited {
            method: "GET",
            path: "/x".into(),
            retry_after: std::time::Duration::from_mins(2),
        };
        let hints: Vec<String> = Surface::iter()
            .filter_map(|surface| err.hint().map(|hint| hint.on(surface).to_string()))
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
