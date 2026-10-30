//! Login values: email, options, token.

use std::str::FromStr;

use derive_more::{AsRef, Display};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result, login::LoginError};

/// An address like `you@example.com`; only parsed, the server checks the rest.
///
/// ```
/// use holst_core::domain::login::Email;
///
/// let email: Email = " you@example.com ".parse()?;
/// assert_eq!(email.as_str(), "you@example.com");
/// assert!("you@localhost".parse::<Email>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Display, AsRef, Deserialize)]
#[as_ref(str)]
#[serde(try_from = "String")]
pub struct Email(String);

impl Email {
    /// The address as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Email {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self> {
        raw.parse()
    }
}

impl FromStr for Email {
    type Err = Error;

    /// # Errors
    ///
    /// [`crate::error::login::LoginError::NotEmail`] when it is not like
    /// `you@example.com`.
    fn from_str(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        // A copied address may carry a zero-width space the server would keep.
        let visible: String = raw.chars().filter(|c| !is_invisible(*c)).collect();
        if is_email(&visible) {
            Ok(Self(visible))
        } else {
            Err(Error::Login(LoginError::NotEmail {
                raw: raw.to_owned(),
            }))
        }
    }
}

const fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{ad}' | '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'
    )
}

fn is_email(raw: &str) -> bool {
    let clean = |part: &str| {
        !part.is_empty() && !part.chars().any(|c| c.is_whitespace() || c.is_control())
    };
    raw.split_once('@').is_some_and(|(local, domain)| {
        clean(local)
            && clean(domain)
            && !domain.contains('@')
            && domain.contains('.')
            && domain.split('.').all(|label| !label.is_empty())
    })
}

/// Ways to log in (`login/get_public_options`); a missing field is `false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LoginMethods {
    pub has_account: bool,
    pub via_password: bool,
    #[serde(rename = "viaTemporaryPassword")]
    pub via_code: bool,
}

/// Where the address leads: the code is preferred to the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRoute {
    NoAccount,
    Code,
    Password,
    SingleSignOn,
}

impl LoginMethods {
    #[must_use]
    pub const fn route(self) -> LoginRoute {
        match (self.has_account, self.via_code, self.via_password) {
            (false, _, _) => LoginRoute::NoAccount,
            (true, true, _) => LoginRoute::Code,
            (true, false, true) => LoginRoute::Password,
            (true, false, false) => LoginRoute::SingleSignOn,
        }
    }
}

/// The token from a login reply; the reply never goes into the error.
///
/// # Errors
///
/// [`crate::error::login::LoginError::Incomplete`] when the reply has no
/// `token`.
///
/// ```
/// use holst_core::domain::login::token;
/// use secrecy::ExposeSecret as _;
/// use serde_json::json;
///
/// assert_eq!(token(&json!({ "token": "csrf-1" }))?.expose_secret(), "csrf-1");
/// assert!(token(&json!({})).is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
pub fn token(value: &Value) -> Result<SecretString> {
    value
        .get("token")
        .and_then(Value::as_str)
        .map(SecretString::from)
        .ok_or(Error::Login(LoginError::Incomplete { field: "token" }))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use crate::domain::login::{Email, LoginMethods, LoginRoute, token};

    #[test]
    fn token_extraction_reports_missing_field_without_leaking_body() {
        let err = token(&json!({ "secretish": "value" }))
            .err()
            .map(|e| format!("{e} {e:?}"));
        assert_eq!(
            err.as_deref()
                .map(|e| e.starts_with("login did not finish: the reply has no token")),
            Some(true)
        );
        assert!(!err.unwrap_or_default().contains("value"));
    }

    #[derive(Debug)]
    enum Verdict {
        Accepted,
        Rejected,
    }

    #[rstest]
    #[case::plain("you@example.com", Verdict::Accepted)]
    #[case::padded("  you@example.com ", Verdict::Accepted)]
    #[case::subdomain("a.b+tag@mail.example.co.uk", Verdict::Accepted)]
    #[case::no_at("you.example.com", Verdict::Rejected)]
    #[case::no_user("@example.com", Verdict::Rejected)]
    #[case::no_dot("you@localhost", Verdict::Rejected)]
    #[case::two_ats("a@b@example.com", Verdict::Rejected)]
    #[case::display_name("You <you@example.com>", Verdict::Rejected)]
    #[case::trailing_dot("you@example.", Verdict::Rejected)]
    #[case::space_inside("y ou@example.com", Verdict::Rejected)]
    #[case::empty("", Verdict::Rejected)]
    fn email_is_parsed_once(#[case] raw: &str, #[case] verdict: Verdict) {
        let email = raw.parse::<Email>().map(|e| e.to_string());
        let expected = matches!(verdict, Verdict::Accepted).then(|| raw.trim());
        assert_eq!(email.as_ref().ok().map(String::as_str), expected);
    }

    #[rstest]
    #[case::zero_width_space("mock@example.com\u{200b}")]
    #[case::byte_order_mark("\u{feff}mock@example.com")]
    #[case::soft_hyphen_inside("mo\u{ad}ck@example.com")]
    fn invisible_characters_of_a_copied_address_are_dropped(#[case] raw: &str) {
        let email = raw.parse::<Email>().map(|e| e.to_string());
        assert_eq!(email.ok().as_deref(), Some("mock@example.com"));
    }

    #[test]
    fn not_email_names_the_input() {
        assert_eq!(
            "you".parse::<Email>().err().map(|e| e.to_string()),
            Some(
                "\"you\" is not an email address: expected something like you@example.com"
                    .to_owned()
            )
        );
    }

    #[rstest]
    #[case::no_account(json!({}), LoginRoute::NoAccount)]
    #[case::code_preferred(
        json!({ "hasAccount": true, "viaTemporaryPassword": true, "viaPassword": true }),
        LoginRoute::Code
    )]
    #[case::password(json!({ "hasAccount": true, "viaPassword": true }), LoginRoute::Password)]
    #[case::sso_only(json!({ "hasAccount": true }), LoginRoute::SingleSignOn)]
    fn the_route_follows_the_login_options(
        #[case] offered: Value,
        #[case] expected: LoginRoute,
    ) {
        let methods: LoginMethods = serde_json::from_value(offered).unwrap_or_default();
        assert_eq!(methods.route(), expected);
    }

    #[test]
    fn login_options_tolerates_missing_fields() {
        let opts =
            serde_json::from_value::<LoginMethods>(json!({ "hasAccount": true }));
        assert_eq!(
            opts.ok(),
            Some(LoginMethods {
                has_account: true,
                ..LoginMethods::default()
            })
        );
    }
}
