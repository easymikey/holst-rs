//! `holst-mcp setup`: log in from a terminal and connect the MCP clients.

use holst_core::{
    command::{
        account::SessionCheck,
        login::{
            self,
            Stopped,
            email::{Awaiting, EmailLogin},
        },
    },
    domain::login::{Email, LoginRoute},
    error::hint::Surface,
    remote::client::Client,
};
use holst_form::{
    draw::NoteTone,
    error::{FormError, Reason},
    form::{Form, Question},
    line::QuestionKind,
    terminal::Terminal,
};
use secrecy::SecretString;

use crate::login::store::{Backend as _, SystemBackend};

mod connect;
pub(crate) use connect::{Clients, Program, SERVER, System};

/// Tries for the code or password, as the CLI allows.
const ATTEMPTS: u8 = 3;

const NOT_EMAIL: Reason = Reason("enter an address like you@example.com");
const EMPTY: Reason = Reason("this cannot be empty");

/// What the second step asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Secret {
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

    const fn question(self) -> Question {
        match self {
            Secret::Code => Question {
                label: "Code",
                kind: QuestionKind::LoginCode,
            },
            Secret::Password => Question {
                label: "Password",
                kind: QuestionKind::Password,
            },
        }
    }

    const fn wrong(self) -> Reason {
        match self {
            Secret::Code => Reason(
                "that code did not match — check the latest email and type it again",
            ),
            Secret::Password => Reason("that password did not match — try again"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SetupError {
    #[error("No Holst account uses that address.")]
    NoAccount,

    #[error(
        "That account signs in through single sign-on, which holst-mcp setup does not do."
    )]
    SsoOnly,

    #[error(
        "The {} was not accepted {ATTEMPTS} times. Run holst-mcp setup to try again.",
        .0.noun()
    )]
    Refused(Secret),

    #[error("unknown client {0}: --clients takes claude or codex, comma-separated")]
    UnknownClient(String),

    #[error(transparent)]
    Form(#[from] FormError),

    #[error("Could not log in")]
    Holst(#[from] holst_core::Error),
}

const fn secret_of(awaiting: &Awaiting) -> Secret {
    match awaiting {
        Awaiting::Code(_) => Secret::Code,
        Awaiting::Password(_) => Secret::Password,
    }
}

/// Which steps `holst-mcp setup` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Steps {
    Both,
    /// `--no-connect`
    LoginOnly,
    /// `--connect-only`
    ConnectOnly,
}

impl Steps {
    pub(crate) const fn connects(self) -> bool {
        !matches!(self, Steps::LoginOnly)
    }
}

/// The steps in order: the login, then the connection of the `clients`.
pub(crate) async fn set_up<T: Terminal>(
    backend: &SystemBackend,
    form: &Form<T>,
    programs: &impl Program,
    binary: &str,
    steps: Steps,
    clients: &Clients,
) -> Result<(), SetupError> {
    match steps {
        Steps::Both => {
            run(backend, form).await?;
            connect::connect(programs, form, binary, clients)
        }
        Steps::LoginOnly => run(backend, form).await,
        Steps::ConnectOnly => connect::connect(programs, form, binary, clients),
    }?;
    Ok(form.outro("Done")?)
}

/// Logs in and saves the login for the server; asks first when one is saved.
pub(crate) async fn run<T: Terminal>(
    backend: &SystemBackend,
    form: &Form<T>,
) -> Result<(), SetupError> {
    match saved_login(backend, form).await? {
        Saved::Keep => Ok(()),
        Saved::Replace(old) => {
            let (email, awaiting) = begin(backend, form).await?;
            let client = ask_until_accepted(&awaiting, form, None, ATTEMPTS).await?;
            keep(backend, client, old, &email, form).await
        }
    }
}

/// What to do about the login saved already.
enum Saved {
    Keep,
    /// Log in again, then end this login (none when it is unusable).
    Replace(Option<Client>),
}

async fn saved_login<T: Terminal>(
    backend: &SystemBackend,
    form: &Form<T>,
) -> Result<Saved, SetupError> {
    match backend.load() {
        Ok(Some((session, _))) => {
            let client = backend.connect(session)?;
            let check = form
                .wait(
                    "Checking the saved login",
                    holst_core::command::account::check(&client),
                )
                .await?;
            match check {
                SessionCheck::Active(me) => {
                    let known = me.name.map_or_else(
                        || "Logged in already.".to_owned(),
                        |name| format!("Logged in as {name} already."),
                    );
                    replace(form, &known, client)
                }
                SessionCheck::Revoked(_) => {
                    form.step("The saved login is not accepted any more", "")?;
                    Ok(Saved::Replace(None))
                }
                SessionCheck::Unknown(err) => replace(
                    form,
                    &format!(
                        "A login is saved already (Holst could not check it: {err})."
                    ),
                    client,
                ),
            }
        }
        Ok(None) => Ok(Saved::Replace(None)),
        Err(err) => {
            form.step("The saved login could not be read", &err.to_string())?;
            Ok(Saved::Replace(None))
        }
    }
}

fn replace<T: Terminal>(
    form: &Form<T>,
    known: &str,
    old: Client,
) -> Result<Saved, SetupError> {
    Ok(form
        .confirm(&format!("{known} Log in again?"), false)?
        .then_some(old)
        .map_or(Saved::Keep, |old| Saved::Replace(Some(old))))
}

async fn begin<T: Terminal>(
    backend: &SystemBackend,
    form: &Form<T>,
) -> Result<(Email, Awaiting), SetupError> {
    let email = form.ask("Email", QuestionKind::Text, &parse_email)?;
    let start = EmailLogin::new(backend.anonymous()?, email.clone());
    let methods = form.wait("Checking the address", start.methods()).await??;
    match methods.route() {
        LoginRoute::NoAccount => Err(SetupError::NoAccount),
        LoginRoute::Code => {
            let sent = form.wait("Sending the code", start.send_code()).await??;
            form.step("A code was sent to", email.as_str())?;
            Ok(sent)
        }
        LoginRoute::Password => Ok(start.with_password()),
        LoginRoute::SingleSignOn => Err(SetupError::SsoOnly),
    }
    .map(|awaiting| (email, awaiting))
}

fn parse_email(text: &str) -> Result<Email, Reason> {
    text.trim().parse().map_err(|_| NOT_EMAIL)
}

fn parse_secret(text: &str) -> Result<SecretString, Reason> {
    (!text.is_empty())
        .then(|| SecretString::from(text))
        .ok_or(EMPTY)
}

/// Asks for the code or password; `refused` is why the last one was not taken.
async fn ask_until_accepted<T: Terminal>(
    awaiting: &Awaiting,
    form: &Form<T>,
    refused: Option<Reason>,
    left: u8,
) -> Result<Client, SetupError> {
    let secret = secret_of(awaiting);
    let typed = refused.map_or_else(
        || {
            let Question { label, kind } = secret.question();
            form.ask(label, kind, &parse_secret)
        },
        |reason| form.ask_again(secret.question(), &parse_secret, reason),
    )?;
    let logged_in = form
        .wait("Signing in", async { awaiting.log_in(&typed).await })
        .await?;
    match logged_in {
        Ok(client) => Ok(client),
        Err(err) if err.is_wrong_secret() && left > 1 => {
            Box::pin(ask_until_accepted(
                awaiting,
                form,
                Some(secret.wrong()),
                left - 1,
            ))
            .await
        }
        Err(err) if err.is_wrong_secret() => Err(SetupError::Refused(secret)),
        Err(err) => Err(err.into()),
    }
}

/// Checks the login works, saves it for the server and ends the replaced one.
async fn keep<T: Terminal>(
    backend: &SystemBackend,
    client: Client,
    old: Option<Client>,
    email: &Email,
    form: &Form<T>,
) -> Result<(), SetupError> {
    let done = form
        .wait(
            "Checking the login",
            login::finish(client, old, async |session| backend.save_new(&session)),
        )
        .await?
        .map_err(|stopped| match stopped {
            Stopped::Check(err) | Stopped::Save(err) => err,
        })?;
    let stale = done
        .saved
        .stale
        .as_ref()
        .map(|stale| (NoteTone::Warn, stale_warning(stale)));
    let lines: Vec<_> = [
        (NoteTone::Plain, email.as_str().to_owned()),
        (
            NoteTone::Faint,
            "The MCP server picks the login up on its next call.".to_owned(),
        ),
    ]
    .into_iter()
    .chain(stale)
    .collect();
    Ok(form.write_box("Logged in", &lines)?)
}

/// An older login that the server still reads first, with what to do.
fn stale_warning(stale: &holst_core::Error) -> String {
    let hint = stale
        .hint()
        .map(|hint| format!("; {}", hint.on(Surface::Mcp)))
        .unwrap_or_default();
    format!("{stale}{hint}")
}

#[cfg(test)] mod tests;
