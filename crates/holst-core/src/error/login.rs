//! Signing in and the shape of a login.

use holst_macros::Hint;

use crate::error::{Error, http::HttpMethod};

/// A login that is missing, refused or not a login at all.
#[derive(Debug, thiserror::Error, Hint)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum LoginError {
    #[error("no login found")]
    #[hint(LoginAgain)]
    NoSavedLogin,

    #[error("the saved login was revoked or has expired")]
    #[hint(LoginAgain)]
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
    #[hint(LoginAgain)]
    OtherServer { saved: String, current: String },

    #[error("login did not finish: the reply has no {field}")]
    Incomplete { field: &'static str },

    #[error(
        "the saved login is an old bare token, which the server refuses: it works only together with the session cookies"
    )]
    #[hint(FullLogin)]
    BareToken { method: HttpMethod, path: String },
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
        hint::decided::assert_all_listed,
        http::HttpMethod,
        login::{LoginError, LoginErrorDiscriminants},
    };

    pub(in crate::error) fn samples() -> Vec<LoginError> {
        vec![
            LoginError::NoSavedLogin,
            LoginError::SessionRevoked,
            LoginError::OtherServer {
                saved: "a".into(),
                current: "b".into(),
            },
            LoginError::NotEmail { raw: "x".into() },
            LoginError::NotSavedLogin,
            LoginError::Incomplete { field: "code" },
            LoginError::BareToken {
                method: HttpMethod::Get,
                path: "/x".into(),
            },
        ]
    }

    #[rstest::rstest]
    #[case::bad_request(400, true)]
    #[case::unauthorized(401, true)]
    #[case::forbidden(403, true)]
    #[case::server_error(500, false)]
    fn only_a_refusal_is_a_wrong_secret(#[case] status: u16, #[case] wrong: bool) {
        let err = crate::Error::Http(crate::fake::http_status(
            status,
            crate::error::http::ReplyBody::Empty,
        ));
        assert_eq!(err.is_wrong_secret(), wrong);
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_all_listed::<_, LoginErrorDiscriminants>(&samples());
    }
}
