use std::{sync::Arc, time::Duration};

use futures_util::{TryStreamExt as _, stream};
use reqwest::header;

use crate::{
    effect::http::{BoxFuture, HttpMethod, HttpRequest, HttpResponse, Transport},
    error::{Error, Result, http::HttpError},
};

const MAX_BODY: usize = 256 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const TIMEOUT: Duration = Duration::from_mins(1);

/// The `reqwest` transport; it never follows redirects, honors `*_PROXY`.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    http: reqwest::Client,
    timeout: Duration,
    body_limit: usize,
}

impl ReqwestTransport {
    /// A transport with the default timeouts and a body limit of `MAX_BODY`.
    ///
    /// # Errors
    /// [`HttpError::Transport`] if the HTTP client cannot be
    /// built.
    pub fn new(user_agent: &str) -> Result<Self> {
        Self::with_limits(user_agent, TIMEOUT, MAX_BODY)
    }

    /// A transport with its own timeout and body limit.
    ///
    /// # Errors
    /// As [`ReqwestTransport::new`].
    pub fn with_limits(
        user_agent: &str,
        timeout: Duration,
        body_limit: usize,
    ) -> Result<Self> {
        reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(|http| Self {
                http,
                timeout,
                body_limit,
            })
            .map_err(HttpError::from)
            .map_err(Error::from)
    }

    async fn read_body(
        &self,
        response: reqwest::Response,
        request: &HttpRequest,
    ) -> Result<Vec<u8>> {
        let limit = self.body_limit;
        response
            .content_length()
            .filter(|declared| *declared > u64::try_from(limit).unwrap_or(u64::MAX))
            .map_or(Ok(()), |_| {
                Err(Error::Http(HttpError::ResponseTooLarge { limit }))
            })?;
        // The server answered: a cut-off body may follow an applied write.
        stream::try_unfold(response, |mut response| async move {
            let chunk = response
                .chunk()
                .await
                .map_err(|err| exchange_error(request, Arrival::MaybeReceived, err))?;
            Ok::<_, Error>(chunk.map(|chunk| (chunk, response)))
        })
        .try_fold(Vec::new(), |body, chunk| async move {
            append_limited(body, &chunk, limit)
        })
        .await
    }
}

impl Transport for ReqwestTransport {
    fn send<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> BoxFuture<'a, Result<HttpResponse>> {
        Box::pin(async move {
            let builder = self
                .http
                .request(request.method.as_reqwest(), request.url.clone())
                .timeout(self.timeout)
                .headers(request.headers.clone());
            let builder = request.body.iter().fold(builder, |builder, payload| {
                builder
                    .header(header::CONTENT_TYPE, payload.content_type.as_str())
                    .body(payload.bytes.clone())
            });
            let response = builder
                .send()
                .await
                .map_err(|err| exchange_error(request, Arrival::of(&err), err))?;
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let body = self.read_body(response, request).await?;
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }

    fn limited(
        self: Arc<Self>,
        timeout: Duration,
        body_limit: usize,
    ) -> Result<Arc<dyn Transport>> {
        Ok(Arc::new(Self {
            http: self.http.clone(),
            timeout,
            body_limit,
        }))
    }
}

impl HttpMethod {
    const fn as_reqwest(self) -> reqwest::Method {
        match self {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Patch => reqwest::Method::PATCH,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
        }
    }
}

/// How far a failed exchange got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    NotSent,
    MaybeReceived,
}

impl Arrival {
    /// Only a failed connection is known not to have reached the server.
    fn of(err: &reqwest::Error) -> Self {
        match err {
            connect if connect.is_connect() => Arrival::NotSent,
            _ => Arrival::MaybeReceived,
        }
    }
}

/// A write that may have been received is never retried.
const fn is_unsure_write(method: HttpMethod, reach: Arrival) -> bool {
    !method.is_read() && matches!(reach, Arrival::MaybeReceived)
}

/// [`HttpError::NoResponse`] if it may be applied, else
/// [`HttpError::Transport`].
fn exchange_error(
    request: &HttpRequest,
    reach: Arrival,
    error: reqwest::Error,
) -> Error {
    Some(request)
        .filter(|request| is_unsure_write(request.method, reach))
        .map_or(Error::Http(HttpError::Transport(error)), |request| {
            Error::Http(HttpError::NoResponse {
                method: request.method.as_str(),
                path: request.url.path().to_owned(),
            })
        })
}

/// The body so far plus one more chunk, unless that crosses `limit`.
///
/// # Errors
/// [`HttpError::ResponseTooLarge`] when the body would grow past
/// `limit`.
fn append_limited(mut body: Vec<u8>, chunk: &[u8], limit: usize) -> Result<Vec<u8>> {
    body.len()
        .checked_add(chunk.len())
        .filter(|len| *len <= limit)
        .ok_or(Error::Http(HttpError::ResponseTooLarge { limit }))
        .map(|_| {
            body.extend_from_slice(chunk);
            body
        })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{Arrival, append_limited, is_unsure_write};
    use crate::{
        effect::http::HttpMethod,
        error::{Error, http::HttpError},
    };

    #[derive(Debug, PartialEq, Eq)]
    enum Fit {
        Within,
        Over,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Applied {
        Unknown,
        Known,
    }

    #[rstest]
    #[case::fits(3, 4, Fit::Within)]
    #[case::exactly_at_limit(4, 4, Fit::Within)]
    #[case::over(5, 4, Fit::Over)]
    fn body_is_limited(#[case] total: usize, #[case] limit: usize, #[case] fit: Fit) {
        let fits = fit == Fit::Within;
        let body = append_limited(vec![0; total - 1], &[1], limit);
        assert_eq!(body.as_ref().map(Vec::len).ok(), fits.then_some(total));
        assert!(
            fits || matches!(
                body,
                Err(Error::Http(HttpError::ResponseTooLarge { limit: 4 }))
            )
        );
    }

    #[rstest]
    #[case::read_after_send(HttpMethod::Get, Arrival::MaybeReceived, Applied::Known)]
    #[case::write_after_send(
        HttpMethod::Post,
        Arrival::MaybeReceived,
        Applied::Unknown
    )]
    #[case::delete_after_send(
        HttpMethod::Delete,
        Arrival::MaybeReceived,
        Applied::Unknown
    )]
    #[case::write_not_sent(HttpMethod::Patch, Arrival::NotSent, Applied::Known)]
    fn outcome_is_unknown_only_for_received_writes(
        #[case] method: HttpMethod,
        #[case] reach: Arrival,
        #[case] applied: Applied,
    ) {
        assert_eq!(is_unsure_write(method, reach), applied == Applied::Unknown);
    }
}
