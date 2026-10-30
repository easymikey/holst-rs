//! Signing in and the shape of a login.

use crate::error::{Error, hint::Hint};

/// A login that is missing, refused or not a login at all.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum LoginError {
    #[error("no login found")]
    NoSavedLogin,

    #[error("the saved login was revoked or has expired")]
    SessionRevoked,

    #[error(
        "\"{raw}\" is not an email address: expected something like you@example.com"
    )]
    NotEmail { raw: String },

    #[error(
        "the saved login is unreadable: expected a token or JSON with a \"token\" field"
    )]
    NotSavedLogin,

    #[error("the saved login is for {saved}, not {current}")]
    OtherServer { saved: String, current: String },

    #[error("login did not finish: the reply has no {field}")]
    Incomplete { field: &'static str },

    #[error(
        "the saved login is an old bare token, which the server refuses: it works only together with the session cookies"
    )]
    BareToken { method: &'static str, path: String },
}

impl LoginError {
    /// What to do next, decided for every variant.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            LoginError::NoSavedLogin
            | LoginError::SessionRevoked
            | LoginError::OtherServer { .. } => Some(Hint::LoginAgain),
            LoginError::BareToken { .. } => Some(Hint::FullLogin),
            LoginError::NotEmail { .. }
            | LoginError::NotSavedLogin
            | LoginError::Incomplete { .. } => None,
        }
    }
}

impl Error {
    /// 400, 401 or 403 on a login: the code or password was not accepted.
    #[must_use]
    pub fn is_wrong_secret(&self) -> bool {
        matches!(self.status(), Some(400 | 401 | 403))
    }
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        hint::{Hint, decided::assert_every_variant},
        login::{LoginError, LoginErrorDiscriminants},
    };

    pub(in crate::error) fn samples() -> Vec<(LoginError, Option<Hint>)> {
        vec![
            (LoginError::NoSavedLogin, Some(Hint::LoginAgain)),
            (LoginError::SessionRevoked, Some(Hint::LoginAgain)),
            (
                LoginError::OtherServer {
                    saved: "a".into(),
                    current: "b".into(),
                },
                Some(Hint::LoginAgain),
            ),
            (LoginError::NotEmail { raw: "x".into() }, None),
            (LoginError::NotSavedLogin, None),
            (LoginError::Incomplete { field: "code" }, None),
            (
                LoginError::BareToken {
                    method: "GET",
                    path: "/x".into(),
                },
                Some(Hint::FullLogin),
            ),
        ]
    }

    #[rstest::rstest]
    #[case::bad_request(400, true)]
    #[case::unauthorized(401, true)]
    #[case::forbidden(403, true)]
    #[case::server_error(500, false)]
    fn only_a_refusal_is_a_wrong_secret(#[case] status: u16, #[case] wrong: bool) {
        let err = crate::Error::Http(crate::error::http::HttpError::Status {
            method: "POST",
            path: "/gapi/login/with_email".into(),
            status,
            detail: String::new(),
        });
        assert_eq!(err.is_wrong_secret(), wrong);
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, LoginErrorDiscriminants>(samples(), LoginError::hint);
    }
}
