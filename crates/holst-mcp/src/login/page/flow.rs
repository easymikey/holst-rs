//! The login behind the page: the address, then the code or the password.

use std::{fmt::Display, sync::Arc};

use holst_core::{
    command::login::email::{CodeSent, EmailLogin, PasswordChosen},
    domain::login::Email,
    remote::client::Client,
};
use secrecy::SecretString;

use crate::{
    error::Error,
    login::{NewLogin, SharedLogin, page::http::Form},
};

/// What the second step asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Secret {
    Code,
    Password,
}

impl Secret {
    const fn noun(self) -> &'static str {
        match self {
            Secret::Code => "code",
            Secret::Password => "password",
        }
    }
}

/// Why the page asks again.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Notice {
    BadEmail,
    NoAccount,
    SsoOnly,
    Empty(Secret),
    Refused(Secret),
    Failed(String),
}

impl Notice {
    fn failed(err: impl Display) -> Self {
        Notice::Failed(err.to_string())
    }

    pub(super) fn words(&self) -> String {
        match self {
            Notice::BadEmail => "That does not look like an email address.".to_owned(),
            Notice::NoAccount => "No Holst account uses that address.".to_owned(),
            Notice::SsoOnly => {
                "That account signs in through single sign-on, which this page does not do."
                    .to_owned()
            }
            Notice::Empty(secret) => format!("Enter the {}.", secret.noun()),
            Notice::Refused(secret) => {
                format!("That {} was not accepted. Try again.", secret.noun())
            }
            Notice::Failed(words) => format!("Could not log in: {words}"),
        }
    }
}

/// What the visitor is to see.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Shown {
    Email(Option<Notice>),
    Secret {
        email: String,
        secret: Secret,
        notice: Option<Notice>,
    },
    LoggedIn {
        email: String,
    },
}

enum Awaiting {
    Code(EmailLogin<CodeSent>),
    Password(EmailLogin<PasswordChosen>),
}

impl Awaiting {
    fn email(&self) -> &Email {
        match self {
            Awaiting::Code(login) => login.email(),
            Awaiting::Password(login) => login.email(),
        }
    }

    const fn secret(&self) -> Secret {
        match self {
            Awaiting::Code(_) => Secret::Code,
            Awaiting::Password(_) => Secret::Password,
        }
    }

    async fn log_in(&self, typed: &SecretString) -> holst_core::Result<Client> {
        match self {
            Awaiting::Code(login) => login.log_in(typed).await,
            Awaiting::Password(login) => login.log_in(typed).await,
        }
    }
}

enum Step {
    Email,
    Secret(Awaiting),
    Done(String),
}

/// Where the login stands.
pub(super) struct Flow(Step);

impl Flow {
    pub(super) const fn new() -> Self {
        Self(Step::Email)
    }

    pub(super) fn shown(&self, notice: Option<Notice>) -> Shown {
        match &self.0 {
            Step::Email => Shown::Email(notice),
            Step::Secret(awaiting) => Shown::Secret {
                email: awaiting.email().as_str().to_owned(),
                secret: awaiting.secret(),
                notice,
            },
            Step::Done(email) => Shown::LoggedIn {
                email: email.clone(),
            },
        }
    }

    /// Takes what the visitor sent for the step the login is at.
    pub(super) async fn submit(&mut self, form: &Form, login: &SharedLogin) -> Shown {
        let next = match &self.0 {
            Step::Email => begin(form, login).await,
            Step::Secret(awaiting) => answer(awaiting, form, login).await,
            Step::Done(email) => Ok(Step::Done(email.clone())),
        };
        match next {
            Ok(step) => {
                self.0 = step;
                self.shown(None)
            }
            Err(notice) => self.shown(Some(notice)),
        }
    }
}

async fn begin(form: &Form, login: &SharedLogin) -> Result<Step, Notice> {
    let email = form
        .get("email")
        .and_then(|typed| typed.trim().parse::<Email>().ok())
        .ok_or(Notice::BadEmail)?;
    let client = login.backend.anonymous().map_err(Notice::failed)?;
    let start = EmailLogin::new(client, email);
    let methods = start.methods().await.map_err(Notice::failed)?;
    match (methods.has_account, methods.via_code, methods.via_password) {
        (false, _, _) => Err(Notice::NoAccount),
        (true, true, _) => start
            .send_code()
            .await
            .map(|sent| Step::Secret(Awaiting::Code(sent)))
            .map_err(Notice::failed),
        (true, false, true) => {
            Ok(Step::Secret(Awaiting::Password(start.with_password())))
        }
        (true, false, false) => Err(Notice::SsoOnly),
    }
}

async fn answer(
    awaiting: &Awaiting,
    form: &Form,
    login: &SharedLogin,
) -> Result<Step, Notice> {
    let typed = form
        .get("secret")
        .filter(|typed| !typed.trim().is_empty())
        .map(SecretString::from)
        .ok_or(Notice::Empty(awaiting.secret()))?;
    match awaiting.log_in(&typed).await {
        Ok(client) => finish(&client, awaiting.email(), login).await,
        Err(err) if matches!(err.status(), Some(400 | 401 | 403)) => {
            Err(Notice::Refused(awaiting.secret()))
        }
        Err(err) => Err(Notice::failed(err)),
    }
}

/// Checks the login works, saves it and leaves it for the server to take.
async fn finish(
    client: &Client,
    email: &Email,
    login: &SharedLogin,
) -> Result<Step, Notice> {
    let me = holst_core::command::account::account(client)
        .await
        .map_err(Notice::failed)?;
    // Saved with the login: the board cache of the account is found by it.
    client.learn_account_id(&me.id);
    let session = client
        .rotated()
        .or_else(|| client.session())
        .ok_or_else(|| {
            Notice::failed(Error::Holst(holst_core::Error::Login(
                holst_core::error::login::LoginError::NoSavedLogin,
            )))
        })?;
    let saved = login
        .save_new(session.clone())
        .await
        .map_err(Notice::failed)?;
    login.arrived.store(Some(Arc::new(NewLogin {
        session,
        store: saved.store,
    })));
    Ok(Step::Done(email.as_str().to_owned()))
}
