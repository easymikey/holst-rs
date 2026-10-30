//! Login by email: an emailed code or a permanent password.

use crate::{
    command::account::account,
    domain::{account::Account, id_source::IdSource, session::Session, url::BaseUrl},
    effect::clock::Clock,
    error::{Error, Result, login::LoginError},
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
    pub struct EmailLogin {
        client: Client,
        email: Email,
    }

    /// A flow that waits for a secret, made by `send_code` or `with_password`.
    #[derive(Debug, Clone)]
    pub struct Flow {
        client: Client,
        email: Email,
    }

    /// What a login waits for: a code cannot be entered before it was sent.
    #[derive(Debug, Clone)]
    pub enum Awaiting {
        Code(Flow),
        Password(Flow),
    }

    impl EmailLogin {
        #[must_use]
        pub fn new(client: Client, email: Email) -> Self {
            Self { client, email }
        }

        /// The address the flow logs in.
        #[must_use]
        pub fn email(&self) -> &Email {
            &self.email
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
        pub async fn send_code(self) -> Result<Awaiting> {
            send_temporary_password(&self.client, &self.email).await?;
            Ok(Awaiting::Code(self.into_flow()))
        }

        /// Log in with a permanent password instead of a code.
        #[must_use]
        pub fn with_password(self) -> Awaiting {
            Awaiting::Password(self.into_flow())
        }

        fn into_flow(self) -> Flow {
            Flow {
                client: self.client,
                email: self.email,
            }
        }
    }

    impl Awaiting {
        /// Log in with the code or password; a refused secret keeps the flow.
        ///
        /// # Errors
        /// [`Client::send_json`] errors or
        /// [`crate::error::login::LoginError::Incomplete`].
        pub async fn log_in(&self, secret: &SecretString) -> Result<Client> {
            let (Awaiting::Code(flow) | Awaiting::Password(flow)) = self;
            // The server takes a code without the spaces around it.
            let sent = match self {
                Awaiting::Code(_) => SecretString::from(secret.expose_secret().trim()),
                Awaiting::Password(_) => secret.clone(),
            };
            let token =
                login::token(&with_email(&flow.client, &flow.email, &sent).await?)?;
            Ok(flow.client.clone().with_token(token))
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

/// What became of the login a new one replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacedLogin {
    Nothing,
    Terminated,
    Kept,
}

impl ReplacedLogin {
    /// The word for scripts; `None` when there was nothing to replace.
    #[must_use]
    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            ReplacedLogin::Nothing => None,
            ReplacedLogin::Terminated => Some("terminated"),
            ReplacedLogin::Kept => Some("kept"),
        }
    }
}

/// Why a login stopped before it was done.
#[derive(Debug)]
pub enum Stopped<E> {
    /// The server did not take the new login, or it holds no session.
    Check(Error),
    /// `save` failed; the previous session is untouched.
    Save(E),
}

/// A login that works, saved by the caller, with the old one ended.
#[derive(Debug)]
pub struct LoggedIn<S> {
    pub account: Account,
    pub client: Client,
    /// What `save` returned.
    pub saved: S,
    pub replaced: ReplacedLogin,
}

/// Checks the login, hands its session to `save`, then ends `previous`.
/// If that cannot end, it is [`ReplacedLogin::Kept`].
///
/// # Errors
/// [`Stopped::Check`]: account unreadable; [`Stopped::Save`]: save failed.
pub async fn finish<S, E>(
    client: Client,
    previous: Option<Client>,
    save: impl AsyncFnOnce(Session) -> std::result::Result<S, E>,
) -> std::result::Result<LoggedIn<S>, Stopped<E>> {
    let account = account(&client).await.map_err(Stopped::Check)?;
    super::lookup::forget_trees(&client, None);
    // `account` saved it with the login: `--offline` knows whose cache to read.
    let session = client
        .rotated()
        .or_else(|| client.session())
        .ok_or(Stopped::Check(Error::Login(LoginError::NoSavedLogin)))?;
    let saved = save(session).await.map_err(Stopped::Save)?;
    let replaced = match previous {
        None => ReplacedLogin::Nothing,
        Some(old) => logout(&old)
            .await
            .map_or(ReplacedLogin::Kept, |_| ReplacedLogin::Terminated),
    };
    Ok(LoggedIn {
        account,
        client,
        saved,
        replaced,
    })
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use rstest::rstest;
    use secrecy::{ExposeSecret as _, SecretString};

    use crate::{
        command::login::{
            ReplacedLogin,
            ServerSessionEnd,
            Stopped,
            email,
            finish,
            logout,
        },
        domain::session::SessionStatus,
        effect::http::HttpResponse,
        error::{Error, Result, login::LoginError},
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

    const ME: &str = r#"{"id":"u1","name":"A"}"#;

    #[tokio::test]
    async fn finish_hands_over_the_new_session_then_ends_the_old_one() -> Result<()> {
        let (_, fresh) = on_fake([reply(200, &[], ME)], Some(stored("new", "sid=2")))?;
        let (old_server, old) =
            on_fake([reply(200, &[], "")], Some(stored("old", "sid=1")))?;
        let done = finish(fresh, Some(old), async |session| {
            Ok::<_, Error>((
                session.token().expose_secret().to_owned(),
                session.account_id().map(str::to_owned),
            ))
        })
        .await
        .ok();
        assert_eq!(
            done.as_ref().map(|done| (&done.saved, done.replaced)),
            Some((
                &("new".to_owned(), Some("u1".to_owned())),
                ReplacedLogin::Terminated
            ))
        );
        assert_eq!(old_server.requests().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn finish_keeps_the_old_session_when_saving_fails() -> Result<()> {
        let (_, fresh) = on_fake([reply(200, &[], ME)], Some(stored("new", "sid=2")))?;
        let (old_server, old) = on_fake([], Some(stored("old", "sid=1")))?;
        let done = finish(fresh, Some(old), async |_| {
            Err::<(), _>(Error::Login(LoginError::NoSavedLogin))
        })
        .await;
        assert!(matches!(done, Err(Stopped::Save(_))), "{done:?}");
        assert!(old_server.requests().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn finish_reports_an_old_session_it_could_not_end() -> Result<()> {
        let (_, fresh) = on_fake([reply(200, &[], ME)], Some(stored("new", "sid=2")))?;
        let (_, old) = on_fake([reply(500, &[], "")], Some(stored("old", "sid=1")))?;
        let done = finish(fresh, Some(old), async |_| Ok::<_, Error>(())).await;
        assert_eq!(
            done.ok().map(|done| done.replaced),
            Some(ReplacedLogin::Kept)
        );
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
