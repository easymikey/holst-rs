//! Login by email: an emailed code or a permanent password.

use crate::{
    domain::{id_source::IdSource, session::Session, url::BaseUrl},
    effect::clock::Clock,
    error::Result,
    remote::{api::login::end_server_session, client::Client},
};

/// A client for a login (or none) on `base` over the real network, telling the
/// time by `clock` and numbering new objects by `ids`.
///
/// # Errors
/// [`crate::error::http::HttpError::Transport`] if the HTTP client cannot be
/// built.
pub fn connect(
    session: Option<Session>,
    base: BaseUrl,
    clock: impl Clock + 'static,
    ids: impl IdSource + 'static,
) -> Result<Client> {
    Client::new(session, base, clock, ids)
}

/// The `email` flow: an emailed code or a permanent password.
pub mod email {
    use std::marker::PhantomData;

    use secrecy::{ExposeSecret as _, SecretString};

    use crate::{
        domain::{
            login,
            login::{Email, LoginMethods},
        },
        error::Result,
        remote::{
            api::login::{public_options, send_temporary_password, with_email},
            client::Client,
        },
    };

    /// The address is known, nothing is sent yet.
    #[derive(Debug, Clone, Copy)]
    pub enum LoginStart {}

    /// A code went out by email.
    #[derive(Debug, Clone, Copy)]
    pub enum CodeSent {}

    /// A permanent password instead of a code.
    #[derive(Debug, Clone, Copy)]
    pub enum PasswordChosen {}

    /// A state where a secret can be entered (sealed by private fields).
    pub trait ReadyForSecret {
        /// Returns the secret as the server takes it: a code without the
        /// spaces around it, a password as typed.
        fn sent(secret: &SecretString) -> SecretString;
    }
    impl ReadyForSecret for CodeSent {
        fn sent(secret: &SecretString) -> SecretString {
            SecretString::from(secret.expose_secret().trim())
        }
    }
    impl ReadyForSecret for PasswordChosen {
        fn sent(secret: &SecretString) -> SecretString {
            secret.clone()
        }
    }

    /// The email login as typed states: a code cannot be entered before it was
    /// sent.
    ///
    /// ```no_run
    /// # async fn demo() -> holst_core::Result<()> {
    /// use holst_core::{
    ///     command::login::{connect, email::EmailLogin},
    ///     domain::id_source::SequentialIds,
    ///     effect::{clock::FixedClock, settings::Settings},
    /// };
    /// use secrecy::SecretString;
    ///
    /// let settings = Settings::from_variables(|name| std::env::var_os(name))?;
    /// let (clock, ids) = (FixedClock { unix_millis: 0 }, SequentialIds::new("id"));
    /// let anonymous = connect(None, settings.base_url, clock, ids)?;
    /// let flow = EmailLogin::new(anonymous, "you@example.com".parse()?);
    /// let sent = flow.send_code().await?;
    /// let client = sent.log_in(&SecretString::from("123456")).await?;
    ///
    /// # drop(client);
    /// # Ok(())
    /// # }
    /// ```
    #[derive(Debug, Clone)]
    pub struct EmailLogin<S> {
        client: Client,
        email: Email,
        step: PhantomData<S>,
    }

    impl EmailLogin<LoginStart> {
        #[must_use]
        pub fn new(client: Client, email: Email) -> Self {
            Self {
                client,
                email,
                step: PhantomData,
            }
        }

        /// The ways to log in for the address. No login needed.
        ///
        /// # Errors
        /// [`Client::send_json`] errors or
        /// [`crate::error::http::HttpError::Json`] (another shape).
        pub async fn methods(&self) -> Result<LoginMethods> {
            Ok(serde_json::from_value(
                public_options(&self.client, &self.email).await?,
            )?)
        }

        /// Send a code by email.
        ///
        /// # Errors
        /// [`Client::send_json`] errors.
        pub async fn send_code(self) -> Result<EmailLogin<CodeSent>> {
            send_temporary_password(&self.client, &self.email).await?;
            Ok(self.step_to())
        }

        /// Log in with a permanent password instead of a code.
        #[must_use]
        pub fn with_password(self) -> EmailLogin<PasswordChosen> {
            self.step_to()
        }
    }

    impl<S> EmailLogin<S> {
        /// The address the flow logs in.
        #[must_use]
        pub fn email(&self) -> &Email {
            &self.email
        }

        fn step_to<T>(self) -> EmailLogin<T> {
            EmailLogin {
                client: self.client,
                email: self.email,
                step: PhantomData,
            }
        }
    }

    impl<S: ReadyForSecret> EmailLogin<S> {
        /// Log in with the code or password; a refused secret keeps the flow.
        ///
        /// # Errors
        /// [`Client::send_json`] errors or
        /// [`crate::error::login::LoginError::Incomplete`].
        pub async fn log_in(&self, secret: &SecretString) -> Result<Client> {
            let token = login::token(
                &with_email(&self.client, &self.email, &S::sent(secret)).await?,
            )?;
            Ok(self.client.clone().with_token(token))
        }
    }
}

/// What logging out did on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerSessionEnd {
    Ended,
    /// There was no login, or the server had already refused it (401 or 403).
    AlreadyEnded,
}

/// End the session on the server and forget it; no or refused login is ok.
///
/// # Errors
/// [`Client::send_json`] errors other than a refused login.
pub async fn logout(client: &Client) -> Result<ServerSessionEnd> {
    end_server_session(client)
        .await
        .map(|_| ServerSessionEnd::Ended)
        .or_else(|err| {
            if err.is_fatal() || err.is_unauthorized() {
                Ok(ServerSessionEnd::AlreadyEnded)
            } else {
                Err(err)
            }
        })
        .inspect(|_| client.forget())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use rstest::rstest;
    use secrecy::SecretString;

    use crate::{
        command::login::{ServerSessionEnd, email, logout},
        domain::session::SessionStatus,
        effect::http::HttpResponse,
        error::{Error, Result},
        remote::{
            api::login::with_email_request,
            client::fake::{on_fake, reply, stored},
        },
    };

    #[test]
    fn the_login_request_shows_no_password() -> Result<()> {
        let request = with_email_request(
            &"me@example.com".parse()?,
            &SecretString::from("s3cr3t"),
        )?;
        assert!(!format!("{request}|{request:?}").contains("s3cr3t"));
        assert_eq!(
            request.to_string(),
            "POST /gapi/login/with_email\n(46 bytes of application/json)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn email_flow_signs_in_after_a_wrong_code() -> Result<()> {
        let (fake, anonymous) = on_fake(
            [
                reply(200, &[], "{}"),
                reply(400, &[], r#"{"errCode":"bad-request"}"#),
                reply(
                    200,
                    &[("set-cookie", "sessionToken=srv")],
                    r#"{"token":"csrf-1"}"#,
                ),
            ],
            None,
        )?;
        let flow = email::EmailLogin::new(anonymous, "me@example.com".parse()?);
        let sent = flow.send_code().await?;
        let wrong = sent.log_in(&SecretString::from("000000")).await.err();
        assert_eq!(
            wrong.as_ref().and_then(Error::status),
            Some(400),
            "{wrong:?}"
        );
        let client = sent.log_in(&SecretString::from(" 123456 ")).await?;
        assert_eq!(client.status(), SessionStatus::Active);
        let rotated = client
            .rotated()
            .inspect(|session| client.confirm_saved(session));
        assert!(rotated.is_some(), "a fresh login is a session to save");
        assert!(client.rotated().is_none(), "saved, so nothing is pending");
        let last_body = fake
            .requests()
            .iter()
            .filter_map(|r| r.body.as_ref())
            .map(|b| String::from_utf8_lossy(&b.bytes).into_owned())
            .next_back();
        assert_eq!(
            last_body.as_deref(),
            Some(r#"{"email":"me@example.com","password":"123456"}"#)
        );
        Ok(())
    }

    #[derive(Debug, PartialEq, Eq)]
    enum LogoutReport {
        Ended(ServerSessionEnd),
        NotEnded,
    }

    #[rstest]
    #[case::ended(reply(200, &[], ""), LogoutReport::Ended(ServerSessionEnd::Ended), SessionStatus::Missing)]
    #[case::already_revoked(reply(401, &[], ""), LogoutReport::Ended(ServerSessionEnd::AlreadyEnded), SessionStatus::Missing)]
    #[case::refused(reply(403, &[], ""), LogoutReport::Ended(ServerSessionEnd::AlreadyEnded), SessionStatus::Missing)]
    #[case::server_down(reply(500, &[], ""), LogoutReport::NotEnded, SessionStatus::Active)]
    #[tokio::test]
    async fn logout_outcomes(
        #[case] answer: HttpResponse,
        #[case] expected: LogoutReport,
        #[case] after: SessionStatus,
    ) -> Result<()> {
        let (_, client) = on_fake([answer], Some(stored("t", "sid=1")))?;
        let outcome = logout(&client)
            .await
            .map_or(LogoutReport::NotEnded, LogoutReport::Ended);
        assert_eq!(outcome, expected);
        assert_eq!(client.status(), after);
        Ok(())
    }

    #[tokio::test]
    async fn logout_without_login_sends_nothing() -> Result<()> {
        let (fake, client) = on_fake([], None)?;
        logout(&client).await?;
        assert!(fake.requests().is_empty());
        Ok(())
    }
}
