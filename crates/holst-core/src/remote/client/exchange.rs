//! One request end to end: retries, redirects and the reply status.

use std::time::Duration;

use itertools::Itertools;
use reqwest::{Url, header};
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::{
    domain::session::SessionState,
    effect::{
        http::{BoxFuture, HttpBody, HttpMethod, HttpRequest, HttpResponse},
        request::Authorization,
    },
    error::{
        Error,
        Result,
        http::{HTML_BODY, HttpError},
        login::LoginError,
    },
    remote::client::{
        Client,
        MAX_ATTEMPTS,
        MAX_REDIRECTS,
        MAX_RETRY_WAIT,
        MIN_SCRUBBED_COOKIE,
        sign::SentSession,
    },
};

#[derive(Debug, Clone)]
pub(crate) struct ExchangeRequest {
    pub(super) method: HttpMethod,
    pub(super) url: Url,
    pub(super) body: Option<HttpBody>,
}

/// How far one exchange has got: retries of this leg and redirects followed.
#[derive(Debug, Clone, Copy)]
struct ExchangeProgress {
    attempt: u32,
    hops: u32,
}

/// The final leg of an exchange with its reply and what the request carried.
#[derive(Debug)]
struct Delivered {
    leg: ExchangeRequest,
    response: HttpResponse,
    sent_session: SentSession,
}

#[derive(Debug, Clone)]
enum ExchangeStep {
    Finish,
    Retry(Duration),
    Follow(ExchangeRequest),
}

impl Client {
    pub(super) async fn deliver(
        &self,
        leg: ExchangeRequest,
        authorization: Authorization,
    ) -> Result<HttpResponse> {
        let delivered = self
            .exchange(
                leg,
                authorization,
                ExchangeProgress {
                    attempt: 0,
                    hops: 0,
                },
            )
            .await?;
        self.settle(delivered, authorization)
    }

    fn exchange(
        &self,
        leg: ExchangeRequest,
        authorization: Authorization,
        progress: ExchangeProgress,
    ) -> BoxFuture<'_, Result<Delivered>> {
        Box::pin(async move {
            let ExchangeProgress { attempt, hops } = progress;
            let request = HttpRequest {
                method: leg.method,
                url: leg.url.clone(),
                headers: self.sign(authorization)?,
                body: leg.body.clone(),
            };
            let credentials = SentSession::of(&request.headers);
            let response = self.transport.send(&request).await?;
            // Cookies of every signed reply count, retried and redirected ones too.
            (authorization != Authorization::Unsigned)
                .then_some(&response)
                .into_iter()
                .for_each(|response| self.absorb_cookies(response));
            match next_step(&leg, &response, attempt, hops)? {
                ExchangeStep::Finish => Ok(Delivered {
                    leg,
                    response,
                    sent_session: credentials,
                }),
                ExchangeStep::Retry(wait) => {
                    tracing::warn!(
                        status = response.status,
                        wait_millis =
                            u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                        attempt = attempt + 1,
                        "retrying the request"
                    );
                    self.clock.sleep(wait).await;
                    self.exchange(
                        leg,
                        authorization,
                        ExchangeProgress {
                            attempt: attempt + 1,
                            hops,
                        },
                    )
                    .await
                }
                ExchangeStep::Follow(to) => {
                    tracing::debug!(to = %to.url.path(), "following a redirect");
                    self.exchange(
                        to,
                        authorization,
                        ExchangeProgress {
                            attempt: 0,
                            hops: hops + 1,
                        },
                    )
                    .await
                }
            }
        })
    }

    fn settle(
        &self,
        delivered: Delivered,
        authorization: Authorization,
    ) -> Result<HttpResponse> {
        let Delivered {
            leg,
            response,
            sent_session: credentials,
        } = delivered;
        let method = leg.method.as_str();
        let path = leg.url.path().to_owned();
        match (response.status, authorization) {
            (status @ 200..=299, _) => {
                if response.is_html() {
                    Err(Error::Http(HttpError::HtmlInsteadOfJson { status }))
                } else {
                    Ok(response)
                }
            }
            (401, Authorization::Session) => {
                let refused = match credentials {
                    SentSession::Bare => {
                        Error::Login(LoginError::BareToken { method, path })
                    }
                    // The words are the login's: no status number, no request path.
                    SentSession::Full => Error::Login(LoginError::SessionRevoked),
                };
                self.transition(SessionState::revoke);
                Err(refused)
            }
            _ => Err(self.status_error(method, path, &response)),
        }
    }

    fn status_error(
        &self,
        method: &'static str,
        path: String,
        response: &HttpResponse,
    ) -> Error {
        let body = if response.is_html() {
            HTML_BODY.to_owned()
        } else {
            response.text()
        };
        let state = self.state();
        let secrets = state
            .token()
            .into_iter()
            .map(ExposeSecret::expose_secret)
            .chain(
                state
                    .cookie_values()
                    .map(ExposeSecret::expose_secret)
                    .filter(|value| value.len() >= MIN_SCRUBBED_COOKIE),
            );
        Error::Http(HttpError::Status {
            method,
            path,
            status: response.status,
            detail: scrub_secrets(&body, secrets),
        })
    }
}

/// What to do after `response`: the `attempt`-th try after `hops` redirects.
///
/// # Errors
/// [`crate::error::http::HttpError::RedirectRefused`] off-origin;
/// [`crate::error::http::HttpError::RateLimited`] for long 429.
fn next_step(
    leg: &ExchangeRequest,
    response: &HttpResponse,
    attempt: u32,
    hops: u32,
) -> Result<ExchangeStep> {
    let retry_after = response
        .header(header::RETRY_AFTER.as_str())
        .and_then(parse_retry_after);
    match (response.status, retry_after) {
        (301..=303 | 307 | 308, _) => redirect(leg, response, hops),
        (429, Some(wait)) if wait > MAX_RETRY_WAIT => {
            Err(Error::Http(HttpError::RateLimited {
                method: leg.method.as_str(),
                path: leg.url.path().to_owned(),
                retry_after: wait,
            }))
        }
        (status, _) => Ok(retry_wait(leg.method, status, retry_after, attempt)
            .map_or(ExchangeStep::Finish, ExchangeStep::Retry)),
    }
}

fn redirect(
    leg: &ExchangeRequest,
    response: &HttpResponse,
    hops: u32,
) -> Result<ExchangeStep> {
    let target = response
        .header(header::LOCATION.as_str())
        .and_then(|location| leg.url.join(location).ok())
        .filter(|_| hops < MAX_REDIRECTS);
    match target {
        None => Ok(ExchangeStep::Finish),
        Some(url) if url.origin() != leg.url.origin() => {
            Err(Error::Http(HttpError::ForeignRedirect {
                location: url.origin().ascii_serialization(),
            }))
        }
        Some(url) => Ok(ExchangeStep::Follow(match (response.status, leg.method) {
            (303, _) | (301 | 302, HttpMethod::Post) => ExchangeRequest {
                method: HttpMethod::Get,
                url,
                body: None,
            },
            _ => ExchangeRequest { url, ..leg.clone() },
        })),
    }
}

/// A successful body as JSON; empty is `Null`, other text a string.
///
/// # Errors
/// [`crate::error::http::HttpError::Json`] for a broken object or array: it is
/// never meant as text.
pub(crate) fn json_or_text(response: &HttpResponse) -> Result<Value> {
    let text = response.text();
    match text.trim_start().as_bytes().first() {
        None => Ok(Value::Null),
        Some(b'{' | b'[') => Ok(serde_json::from_str(&text)?),
        Some(_) => Ok(serde_json::from_str(&text).unwrap_or(Value::String(text))),
    }
}

pub(crate) fn retry_wait(
    method: HttpMethod,
    status: u16,
    retry_after: Option<Duration>,
    attempt: u32,
) -> Option<Duration> {
    let retryable = status == 429 || (method.is_read() && matches!(status, 502..=504));
    // Lazy: `500 << attempt` would overflow on a large attempt.
    (retryable && attempt + 1 < MAX_ATTEMPTS)
        .then(|| retry_after.unwrap_or(Duration::from_millis(500 << attempt)))
        .filter(|wait| *wait <= MAX_RETRY_WAIT)
}

fn parse_retry_after(raw: &str) -> Option<Duration> {
    raw.trim().parse().ok().map(Duration::from_secs)
}

pub(crate) fn scrub_secrets<'a>(
    body: &str,
    secrets: impl IntoIterator<Item = &'a str>,
) -> String {
    let body = secrets
        .into_iter()
        .filter(|secret| !secret.is_empty())
        .sorted_unstable_by_key(|secret| std::cmp::Reverse(secret.len()))
        .fold(body.to_owned(), |body, secret| body.replace(secret, "***"));
    match body.get(..body.floor_char_boundary(crate::remote::client::ERROR_BODY_LIMIT))
    {
        Some(head) if head.len() < body.len() => format!("{head}…"),
        _ => body,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rstest::rstest;
    use serde_json::Value;

    use crate::{
        effect::http::HttpMethod,
        remote::client::{
            MAX_ATTEMPTS,
            exchange::{json_or_text, parse_retry_after, retry_wait},
            fake::reply,
        },
    };

    #[rstest]
    #[case::empty("", Some(Value::Null))]
    #[case::blank(" \n", Some(Value::Null))]
    #[case::json(r#"{"a":1}"#, Some(serde_json::json!({ "a": 1 })))]
    #[case::text("ok", Some(Value::String("ok".into())))]
    // A cut object would read as a string, and then as "no workspaces".
    #[case::cut_object(r#"{"workspaces":[{"id":"w"#, None)]
    #[case::cut_array(" [1, 2", None)]
    fn success_body_is_json_or_text(
        #[case] body: &str,
        #[case] expected: Option<Value>,
    ) {
        assert_eq!(json_or_text(&reply(200, &[], body)).ok(), expected);
    }

    struct Failure {
        method: HttpMethod,
        status: u16,
        retry_after: Option<Duration>,
        attempt: u32,
    }

    const fn failure(
        method: HttpMethod,
        status: u16,
        retry_after: Option<Duration>,
        attempt: u32,
    ) -> Failure {
        Failure {
            method,
            status,
            retry_after,
            attempt,
        }
    }

    const TWO_SECS: Option<Duration> = Some(Duration::from_secs(2));

    #[rstest]
    #[case::any_method_on_429(failure(HttpMethod::Post, 429, TWO_SECS, 0), TWO_SECS)]
    #[case::post_not_on_503(failure(HttpMethod::Post, 503, None, 0), None)]
    #[case::get_backoff_on_503(
        failure(HttpMethod::Get, 503, None, 1),
        Some(Duration::from_secs(1))
    )]
    #[case::attempts_exhausted(failure(HttpMethod::Get, 429, None, MAX_ATTEMPTS - 1), None)]
    #[case::wait_too_long(
        failure(HttpMethod::Get, 429, Some(Duration::from_hours(1)), 0),
        None
    )]
    #[case::not_retryable_500(failure(HttpMethod::Get, 500, None, 0), None)]
    #[case::not_retryable_404(failure(HttpMethod::Get, 404, None, 0), None)]
    fn retry_policy_cases(#[case] failed: Failure, #[case] expected: Option<Duration>) {
        let Failure {
            method,
            status,
            retry_after,
            attempt,
        } = failed;
        assert_eq!(retry_wait(method, status, retry_after, attempt), expected);
    }

    #[rstest]
    #[case::seconds(" 7 ", Some(Duration::from_secs(7)))]
    #[case::http_date("Wed, 21 Oct 2026 07:28:00 GMT", None)]
    fn retry_after_seconds(#[case] raw: &str, #[case] expected: Option<Duration>) {
        assert_eq!(parse_retry_after(raw), expected);
    }
}
