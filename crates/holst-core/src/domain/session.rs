//! A login and its lifecycle `Missing → Active → Revoked` as typed states.

use std::{borrow::Cow, str::FromStr};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        cookie::{CookieChange, CookieJar},
        url::{BaseUrl, DEFAULT_BASE},
    },
    error::{Error, Result, login::LoginError},
};

/// A login: the Bearer token (a CSRF value), the cookies and its server.
#[derive(Debug, Clone, Default)]
pub struct Session {
    token: SecretString,
    cookies: CookieJar,
    account_id: Option<String>,
    base: Option<BaseUrl>,
}

impl Session {
    #[must_use]
    pub fn new(token: SecretString) -> Self {
        Self::with_cookies(token, CookieJar::default())
    }

    /// A login with a token and the cookies the server set.
    #[must_use]
    pub fn with_cookies(token: SecretString, cookies: CookieJar) -> Self {
        Self {
            token,
            cookies,
            account_id: None,
            base: None,
        }
    }

    /// The same login, saved for `base`.
    #[must_use]
    pub fn with_base(self, base: &BaseUrl) -> Self {
        Self {
            base: Some(base.clone()),
            ..self
        }
    }

    /// This login on `base`; one saved without a server (by 0.2.0) belongs to
    /// the production one.
    ///
    /// # Errors
    /// [`LoginError::OtherServer`] when it was saved for another server.
    pub fn on(self, base: &BaseUrl) -> Result<Self> {
        let saved = self
            .base
            .as_ref()
            .map_or(DEFAULT_BASE, BaseUrl::as_str)
            .to_owned();
        (saved == base.as_str())
            .then(|| self.with_base(base))
            .ok_or_else(|| {
                Error::Login(LoginError::OtherServer {
                    saved,
                    current: base.as_str().to_owned(),
                })
            })
    }

    /// The same login, naming its account.
    #[must_use]
    pub fn with_account_id(self, user: &str) -> Self {
        Self {
            account_id: Some(user.to_owned()),
            ..self
        }
    }

    /// The account's user id, when the login was saved with it.
    #[must_use]
    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    /// The token, still secret.
    #[must_use]
    pub fn token(&self) -> &SecretString {
        &self.token
    }

    /// Whether `other` is the same login (cookies rotate, the token does not).
    #[must_use]
    pub fn same_login(&self, other: &Self) -> bool {
        self.token.expose_secret() == other.token.expose_secret()
    }

    /// The session cookies.
    #[must_use]
    pub fn cookies(&self) -> &CookieJar {
        &self.cookies
    }

    /// The token for logs and status lines: head and tail only.
    #[must_use]
    pub fn redacted(&self) -> String {
        redact(self.token.expose_secret())
    }

    /// Stored form: a bare token if no cookies and no leading `{`, else JSON.
    #[must_use]
    pub fn to_saved_login(&self) -> SecretString {
        let token = self.token.expose_secret();
        let header = self.cookies.header();
        match (header.expose_secret(), &self.account_id, &self.base) {
            ("", None, None) if !token.starts_with('{') => SecretString::from(token),
            (line, user, base) => SecretString::from(
                serde_json::to_string(&SavedLoginJson {
                    token: token.into(),
                    cookies: line.into(),
                    user: user.as_deref().map(Cow::Borrowed),
                    base_url: base.as_ref().map(|base| Cow::Borrowed(base.as_str())),
                })
                .unwrap_or_else(|_| token.to_owned()),
            ),
        }
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.token.expose_secret() == other.token.expose_secret()
            && self.cookies == other.cookies
            && self.account_id == other.account_id
            && self.base.as_ref().map(BaseUrl::as_str)
                == other.base.as_ref().map(BaseUrl::as_str)
    }
}

impl Eq for Session {}

#[derive(Serialize, Deserialize)]
struct SavedLoginJson<'a> {
    #[serde(borrow)]
    token: Cow<'a, str>,
    #[serde(default, borrow)]
    cookies: Cow<'a, str>,
    #[serde(default, borrow, skip_serializing_if = "Option::is_none")]
    user: Option<Cow<'a, str>>,
    #[serde(default, borrow, skip_serializing_if = "Option::is_none")]
    base_url: Option<Cow<'a, str>>,
}

impl FromStr for Session {
    type Err = Error;

    /// A stored value: JSON with cookies, or a bare token from an old version.
    ///
    /// # Errors
    /// [`crate::error::login::LoginError::NotSavedLogin`] for empty, broken or
    /// token-less input.
    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim() {
            "" => Err(Error::Login(LoginError::NotSavedLogin)),
            json if json.starts_with('{') => {
                serde_json::from_str::<SavedLoginJson<'_>>(json)
                    .ok()
                    .filter(|stored| !stored.token.is_empty())
                    .and_then(|stored| {
                        let Ok(cookies) = stored.cookies.parse::<CookieJar>();
                        let base = stored
                            .base_url
                            .map(|text| text.parse::<BaseUrl>())
                            .transpose()
                            .ok()?;
                        Some(Session {
                            account_id: stored.user.map(Cow::into_owned),
                            base,
                            ..Session::with_cookies(
                                SecretString::from(stored.token.as_ref()),
                                cookies,
                            )
                        })
                    })
                    .ok_or(Error::Login(LoginError::NotSavedLogin))
            }
            bare => Ok(Session::new(SecretString::from(bare))),
        }
    }
}

/// Show a token in logs and messages by its length only, no characters of it.
#[must_use]
pub fn redact(token: &str) -> String {
    format!("set ({} chars)", token.chars().count())
}

/// Where a client's login stands.
#[derive(Debug, Clone)]
pub enum SessionState {
    /// No login yet; cookies still gather (login endpoints set them first).
    Missing { cookies: CookieJar },
    /// A working login; `rotated` marks a change the store has not seen.
    Active { session: Session, rotated: bool },
    /// A login refused with a 401; `cookies` are those set after it.
    Revoked {
        session: Session,
        cookies: CookieJar,
    },
}

/// The state without its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Missing,
    Active,
    Revoked,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Missing {
            cookies: CookieJar::default(),
        }
    }
}

impl From<Option<Session>> for SessionState {
    fn from(session: Option<Session>) -> Self {
        session.map_or_else(Self::default, |session| Self::Active {
            session,
            rotated: false,
        })
    }
}

impl SessionState {
    /// The state without its data.
    #[must_use]
    pub fn status(&self) -> SessionStatus {
        match self {
            Self::Missing { .. } => SessionStatus::Missing,
            Self::Active { .. } => SessionStatus::Active,
            Self::Revoked { .. } => SessionStatus::Revoked,
        }
    }

    /// Cookies to send; a revoked login sends only those set after the refusal.
    #[must_use]
    pub fn cookies(&self) -> &CookieJar {
        match self {
            Self::Missing { cookies } | Self::Revoked { cookies, .. } => cookies,
            Self::Active { session, .. } => session.cookies(),
        }
    }

    /// Every cookie value held, refused ones included, to scrub errors.
    pub fn cookie_values(&self) -> impl Iterator<Item = &SecretString> {
        let (first, second) = match self {
            Self::Missing { cookies } => (cookies, None),
            Self::Active { session, .. } => (session.cookies(), None),
            Self::Revoked { session, cookies } => (session.cookies(), Some(cookies)),
        };
        first
            .values()
            .chain(second.into_iter().flat_map(CookieJar::values))
    }

    /// The token held, active or revoked, to scrub errors after a refusal.
    #[must_use]
    pub fn token(&self) -> Option<&SecretString> {
        match self {
            Self::Active { session, .. } | Self::Revoked { session, .. } => {
                Some(session.token())
            }
            Self::Missing { .. } => None,
        }
    }

    /// Back to no login, cookies included: the end of a logout.
    #[must_use]
    pub fn forget(self) -> Self {
        drop(self);
        Self::default()
    }

    /// The active session, if there is one.
    #[must_use]
    pub fn session(&self) -> Option<&Session> {
        match self {
            Self::Active { session, .. } => Some(session),
            Self::Missing { .. } | Self::Revoked { .. } => None,
        }
    }

    /// The token for `Authorization: Bearer`.
    ///
    /// # Errors
    /// [`crate::error::login::LoginError::NoSavedLogin`] without a login;
    /// [`crate::error::login::LoginError::SessionRevoked`] after 401.
    pub fn active_token(&self) -> Result<&SecretString> {
        match self {
            Self::Active { session, .. } => Ok(session.token()),
            Self::Missing { .. } => Err(Error::Login(LoginError::NoSavedLogin)),
            Self::Revoked { .. } => Err(Error::Login(LoginError::SessionRevoked)),
        }
    }

    /// Cookies from a response; any change to an active login marks it rotated.
    #[must_use]
    pub fn absorb(self, fresh: Vec<CookieChange>) -> Self {
        match self {
            Self::Missing { cookies } => Self::Missing {
                cookies: cookies.merge(fresh),
            },
            Self::Active { session, rotated } => {
                let Session {
                    token,
                    cookies,
                    account_id,
                    base,
                } = session;
                let after = cookies.clone().merge(fresh);
                Self::Active {
                    rotated: rotated || after != cookies,
                    session: Session {
                        account_id,
                        base,
                        ..Session::with_cookies(token, after)
                    },
                }
            }
            Self::Revoked { session, cookies } => Self::Revoked {
                session,
                cookies: cookies.merge(fresh),
            },
        }
    }

    /// A token arrived (login finished) on `base`. The new session is rotated
    /// and keeps only the cookies held now (after a refusal, those set since).
    #[must_use]
    pub fn log_in(self, token: SecretString, base: &BaseUrl) -> Self {
        let cookies = match self {
            Self::Missing { cookies } | Self::Revoked { cookies, .. } => cookies,
            Self::Active { session, .. } => session.cookies,
        };
        Self::Active {
            session: Session::with_cookies(token, cookies).with_base(base),
            rotated: true,
        }
    }

    /// The server answered 401 to a signed request.
    #[must_use]
    pub fn revoke(self) -> Self {
        match self {
            Self::Active { session, .. } => Self::Revoked {
                session,
                cookies: CookieJar::default(),
            },
            other @ (Self::Missing { .. } | Self::Revoked { .. }) => other,
        }
    }

    /// The session to store if its cookies changed; only looks.
    #[must_use]
    pub fn rotated(&self) -> Option<&Session> {
        match self {
            Self::Active { session, rotated } => rotated.then_some(session),
            Self::Missing { .. } | Self::Revoked { .. } => None,
        }
    }

    /// `stored` was saved: clear the mark if it is still the current session.
    #[must_use]
    pub fn saved(self, stored: &Session) -> Self {
        match self {
            Self::Active { session, rotated } => Self::Active {
                rotated: rotated && session != *stored,
                session,
            },
            other @ (Self::Missing { .. } | Self::Revoked { .. }) => other,
        }
    }

    /// The active login's account became known; a new one marks it for saving.
    #[must_use]
    pub fn learn_account_id(self, user: &str) -> Self {
        match self {
            Self::Active { session, rotated } => Self::Active {
                rotated: rotated || session.account_id() != Some(user),
                session: session.with_account_id(user),
            },
            other @ (Self::Missing { .. } | Self::Revoked { .. }) => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use secrecy::{ExposeSecret, SecretString};

    use crate::{
        domain::{
            cookie::CookieChange,
            session::{Session, SessionState, SessionStatus, redact},
            url::{BaseUrl, DEFAULT_BASE},
        },
        error::{Error, Result, hint::Hint, login::LoginError},
        remote::client::fake::stored as session,
    };

    fn pair(s: &Session) -> (String, String) {
        (
            s.token().expose_secret().to_owned(),
            s.cookies().header().expose_secret().to_owned(),
        )
    }

    fn sid(value: &str) -> CookieChange {
        CookieChange::Set {
            name: "sid".into(),
            value: value.into(),
        }
    }

    fn saved(state: SessionState) -> (SessionState, Option<Session>) {
        let rotated = state.rotated().cloned();
        (rotated.iter().fold(state, SessionState::saved), rotated)
    }

    #[rstest]
    #[case::bare_token("abc", Some(("abc", "")))]
    #[case::bare_token_with_spaces("  abc\n", Some(("abc", "")))]
    #[case::json(r#"{"token":"abc","cookies":"s=1; c=2"}"#, Some(("abc", "c=2; s=1")))]
    #[case::json_without_cookies(r#"{"token":"abc"}"#, Some(("abc", "")))]
    #[case::json_without_token(r#"{"cookies":"s=1"}"#, None)]
    #[case::broken_json("{nope", None)]
    #[case::json_with_a_broken_server(r#"{"token":"abc","base_url":"nope"}"#, None)]
    #[case::empty("   ", None)]
    fn parse_cases(#[case] raw: &str, #[case] expected: Option<(&str, &str)>) {
        let parsed = raw.parse::<Session>().ok().map(|s| pair(&s));
        assert_eq!(parsed, expected.map(|(t, c)| (t.to_owned(), c.to_owned())));
    }

    #[test]
    fn the_account_round_trips_and_marks_the_session_for_saving() {
        let named = session("abc", "").with_account_id("u1");
        let stored = named.to_saved_login();
        assert_eq!(
            stored.expose_secret(),
            r#"{"token":"abc","cookies":"","user":"u1"}"#
        );
        let back = stored.expose_secret().parse::<Session>().ok();
        assert_eq!(back.as_ref().and_then(Session::account_id), Some("u1"));
        assert_eq!(session("abc", "").account_id(), None);
        let loaded = SessionState::from(Some(session("tok", "sid=1")));
        let (learned, rotated) = saved(loaded.learn_account_id("u1"));
        assert_eq!(rotated.as_ref().and_then(Session::account_id), Some("u1"));
        let (again, none) = saved(learned.learn_account_id("u1"));
        assert!(none.is_none(), "a known account is saved again");
        let (_, kept) = saved(again.absorb(vec![sid("2")]));
        assert_eq!(kept.as_ref().and_then(Session::account_id), Some("u1"));
    }

    #[rstest]
    #[case::bare(session("abc", ""), "abc")]
    // Stored bare, this token would read back as broken JSON.
    #[case::brace_token(session("{x", ""), r#"{"token":"{x","cookies":""}"#)]
    #[case::with_cookies(session("abc", "s=1"), r#"{"token":"abc","cookies":"s=1"}"#)]
    fn session_round_trips_through_storage(
        #[case] session: Session,
        #[case] stored: &str,
    ) {
        assert_eq!(session.to_saved_login().expose_secret(), stored);
        assert_eq!(stored.parse::<Session>().ok(), Some(session));
    }

    #[rstest]
    #[case::shows_only_the_length(
        "11111111-2222-3333-4444-555555555555",
        "set (36 chars)"
    )]
    #[case::short("abcd", "set (4 chars)")]
    fn redact_cases(#[case] token: &str, #[case] expected: &str) {
        assert_eq!(redact(token), expected);
    }

    #[test]
    fn debug_never_shows_token_or_cookie_values() {
        let shown = format!("{:?}", session("tok-secret", "sid=cookie-secret"));
        assert!(!shown.contains("tok-secret"), "{shown}");
        assert!(!shown.contains("cookie-secret"), "{shown}");
        assert!(shown.contains("sid"), "{shown}");
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn login_keeps_cookies_collected_before_the_token() -> Result<()> {
        let production: BaseUrl = DEFAULT_BASE.parse()?;
        let state = SessionState::default()
            .absorb(vec![sid("1")])
            .log_in(SecretString::from("tok"), &production);
        let (state, rotated) = saved(state);
        assert_eq!(state.status(), SessionStatus::Active);
        assert_eq!(
            rotated.map(|s| pair(&s)),
            Some(("tok".into(), "sid=1".into()))
        );
        assert!(
            saved(state).1.is_none(),
            "a saved rotation is offered again"
        );
        Ok(())
    }

    #[test]
    fn only_a_changed_cookie_marks_rotation() {
        let loaded = SessionState::from(Some(session("tok", "sid=1")));
        let (same, none) = saved(loaded.absorb(vec![sid("1")]));
        assert!(none.is_none());
        let (_, rotated) = saved(same.absorb(vec![sid("2")]));
        assert_eq!(
            rotated.map(|s| pair(&s)),
            Some(("tok".into(), "sid=2".into()))
        );
    }

    #[test]
    fn rotation_is_cleared_only_by_a_save_of_the_current_session() {
        let rotated =
            SessionState::from(Some(session("tok", "sid=1"))).absorb(vec![sid("2")]);
        let first = rotated.rotated().cloned();
        assert_eq!(
            first.as_ref().map(pair),
            Some(("tok".into(), "sid=2".into()))
        );
        // The save failed: nothing confirmed, still marked.
        assert!(rotated.rotated().is_some());
        // The server rotated again while the old copy was being saved.
        let newer = rotated.absorb(vec![sid("3")]);
        let after_old_save = first.iter().fold(newer, SessionState::saved);
        let pending = after_old_save.rotated().cloned();
        assert_eq!(
            pending.as_ref().map(pair),
            Some(("tok".into(), "sid=3".into()))
        );
        let done = pending.iter().fold(after_old_save, SessionState::saved);
        assert!(done.rotated().is_none());
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn login_after_revocation_drops_the_dead_cookies() -> Result<()> {
        let production: BaseUrl = DEFAULT_BASE.parse()?;
        let revoked = SessionState::from(Some(session("old", "dead=1; sid=old")))
            .revoke()
            .absorb(vec![sid("new")]);
        assert_eq!(revoked.cookies().header().expose_secret(), "sid=new");
        let scrubbed: Vec<_> = revoked
            .cookie_values()
            .map(|v| v.expose_secret().to_owned())
            .collect();
        assert_eq!(scrubbed, ["1", "old", "new"]);
        let (_, stored) = saved(revoked.log_in(SecretString::from("tok"), &production));
        assert_eq!(
            stored.map(|s| pair(&s)),
            Some(("tok".into(), "sid=new".into()))
        );
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn revoked_session_signs_nothing_until_a_new_login() -> Result<()> {
        let production: BaseUrl = DEFAULT_BASE.parse()?;
        let revoked = SessionState::from(Some(session("tok", ""))).revoke();
        assert_eq!(revoked.status(), SessionStatus::Revoked);
        assert!(matches!(
            revoked.active_token(),
            Err(Error::Login(LoginError::SessionRevoked))
        ));
        assert!(revoked.session().is_none());
        let again = revoked.log_in(SecretString::from("new"), &production);
        assert_eq!(
            again
                .active_token()
                .map(|t| t.expose_secret().to_owned())
                .ok(),
            Some("new".into())
        );
        assert!(matches!(
            SessionState::default().active_token(),
            Err(Error::Login(LoginError::NoSavedLogin))
        ));
        Ok(())
    }

    #[rstest]
    #[case::bare_on_production("t", DEFAULT_BASE, true)]
    #[case::bare_elsewhere("t", "https://staging.example", false)]
    #[case::legacy_on_production(
        r#"{"token":"t","cookies":"s=1"}"#,
        DEFAULT_BASE,
        true
    )]
    #[case::legacy_elsewhere(
        r#"{"token":"t","cookies":"s=1"}"#,
        "https://staging.example",
        false
    )]
    #[case::saved_here(
        r#"{"token":"t","base_url":"https://staging.example/"}"#,
        "https://staging.example",
        true
    )]
    #[case::saved_elsewhere(
        r#"{"token":"t","base_url":"https://staging.example"}"#,
        DEFAULT_BASE,
        false
    )]
    fn a_login_belongs_to_the_server_it_was_saved_for(
        #[case] raw: &str,
        #[case] base: &str,
        #[case] fits: bool,
    ) {
        let fitting = raw
            .parse::<Session>()
            .ok()
            .zip(base.parse::<BaseUrl>().ok())
            .map(|(login, base)| login.on(&base).is_ok());
        assert_eq!(fitting, Some(fits));
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn the_refusal_names_both_servers_and_says_to_log_in() -> Result<()> {
        let staging: BaseUrl = "https://staging.example".parse()?;
        let refused = session("t", "s=1").on(&staging).err();
        let hint = refused.as_ref().and_then(Error::hint);
        assert_eq!(
            refused.map(|err| err.to_string()),
            Some("the saved login is for https://app.holst.so, not https://staging.example".to_owned())
        );
        assert_eq!(hint, Some(Hint::LoginAgain));
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn the_server_is_saved_with_the_login_and_read_back() -> Result<()> {
        let staging: BaseUrl = "https://staging.example".parse()?;
        let stamped = session("abc", "").with_base(&staging);
        let stored = stamped.to_saved_login();
        assert_eq!(
            stored.expose_secret(),
            r#"{"token":"abc","cookies":"","base_url":"https://staging.example"}"#
        );
        assert_eq!(stored.expose_secret().parse::<Session>()?, stamped);
        Ok(())
    }
}
