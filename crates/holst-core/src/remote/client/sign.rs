//! Login headers: the token and cookies a request carries and brings back.

use reqwest::{
    header,
    header::{HeaderMap, HeaderValue},
};
use secrecy::ExposeSecret;

use crate::{
    domain::{
        cookie::{self, CookieChange},
        time::Timestamp,
    },
    effect::{http::HttpResponse, request::Authorization, socket::SocketSession},
    error::{Error, Result, input::InputError},
    remote::client::Client,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SentSession {
    Full,
    Bare,
}

impl SentSession {
    pub(super) fn of(headers: &HeaderMap) -> Self {
        headers
            .get(header::COOKIE)
            .map_or(SentSession::Bare, |_| SentSession::Full)
    }
}

impl Client {
    /// The board socket address and the login it sends.
    ///
    /// # Errors
    /// [`crate::error::login::LoginError::NoSavedLogin`] or
    /// [`crate::error::login::LoginError::SessionRevoked`] without a live
    /// login, [`crate::error::input::InputError::BaseNotUrl`] for a base with
    /// no socket form.
    pub(crate) fn session_for_socket(&self) -> Result<SocketSession> {
        let state = self.state();
        Ok(SocketSession {
            url: self.base.board_socket()?,
            token: state.active_token()?.clone(),
            cookies: state.cookies().header(),
            user_agent: user_agent(),
        })
    }

    pub(super) fn absorb_cookies(&self, response: &HttpResponse) {
        let now = Timestamp::from_millis(self.unix_millis());
        let fresh: Vec<CookieChange> = response
            .header_values(header::SET_COOKIE.as_str())
            .filter_map(|raw| cookie::parse_set_cookie(raw, now))
            .collect();
        if !fresh.is_empty() {
            tracing::debug!(
                cookies = %crate::domain::text::joined(fresh.iter().map(CookieChange::name), ", "),
                "the server set cookies"
            );
            self.transition(|state| state.absorb(fresh));
        }
    }

    pub(super) fn sign(&self, authorization: Authorization) -> Result<HeaderMap> {
        let bearer = match authorization {
            Authorization::Session => Some(self.bearer()?),
            Authorization::Anonymous | Authorization::Unsigned => None,
        };
        let cookies = (authorization != Authorization::Unsigned)
            .then(|| self.state().cookies().header())
            .filter(|joined| !joined.expose_secret().is_empty())
            .and_then(|joined| HeaderValue::from_str(joined.expose_secret()).ok())
            .map(sensitive);
        Ok(bearer
            .map(|value| (header::AUTHORIZATION, value))
            .into_iter()
            .chain(cookies.map(|value| (header::COOKIE, value)))
            .collect())
    }

    fn bearer(&self) -> Result<HeaderValue> {
        let state = self.state();
        let line = zeroize::Zeroizing::new(format!(
            "Bearer {}",
            state.active_token()?.expose_secret()
        ));
        HeaderValue::from_str(&line)
            .map(sensitive)
            .map_err(|_| Error::Input(InputError::SessionNotHeaderSafe))
    }
}

fn sensitive(mut value: HeaderValue) -> HeaderValue {
    value.set_sensitive(true);
    value
}

pub(super) fn user_agent() -> String {
    format!(
        "holst-rs/{} ({})",
        env!("CARGO_PKG_VERSION"),
        platform(std::env::consts::OS)
    )
}

const fn platform(os: &str) -> &'static str {
    match os.as_bytes() {
        b"macos" => "Macintosh; Intel Mac OS X 10_15_7",
        b"windows" => "Windows NT 10.0; Win64; x64",
        _ => "X11; Linux x86_64",
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use reqwest::{header, header::HeaderValue};
    use rstest::rstest;

    use crate::{
        domain::{session::Session, url::DEFAULT_BASE},
        effect::request::Authorization,
        error::{Error, Result, login::LoginError},
        remote::client::{Client, fake::stored, sign::platform},
    };

    fn client(session: Option<Session>) -> Result<Client> {
        crate::remote::client::fake::new_client(session, DEFAULT_BASE.parse()?)
    }

    #[test]
    fn missing_token_is_reported_before_request() -> Result<()> {
        assert!(matches!(
            client(None)?.sign(Authorization::Session),
            Err(Error::Login(LoginError::NoSavedLogin))
        ));
        Ok(())
    }

    #[test]
    fn auth_headers_carry_only_bearer_marked_sensitive() -> Result<()> {
        let h =
            client(Some(Session::new("tok".into())))?.sign(Authorization::Session)?;
        let authorization = h.get(header::AUTHORIZATION);
        assert_eq!(
            authorization.and_then(|a| a.to_str().ok()),
            Some("Bearer tok")
        );
        assert!(authorization.is_some_and(HeaderValue::is_sensitive));
        assert!(h.get(header::COOKIE).is_none(), "no cookie was needed");
        Ok(())
    }

    #[rstest]
    #[case::anonymous(Authorization::Anonymous, Some("sid=1"))]
    #[case::unsigned(Authorization::Unsigned, None)]
    fn only_unsigned_drops_the_cookies(
        #[case] authorization: Authorization,
        #[case] cookie: Option<&str>,
    ) -> Result<()> {
        let h = client(Some(stored("tok", "sid=1")))?.sign(authorization)?;
        assert!(h.get(header::AUTHORIZATION).is_none());
        let sent = h.get(header::COOKIE).and_then(|c| c.to_str().ok());
        assert_eq!(sent, cookie);
        Ok(())
    }

    #[rstest]
    #[case::mac("macos", "Macintosh; Intel Mac OS X 10_15_7")]
    #[case::linux("linux", "X11; Linux x86_64")]
    fn platform_in_user_agent(#[case] os: &str, #[case] expected: &str) {
        assert_eq!(platform(os), expected);
    }
}
