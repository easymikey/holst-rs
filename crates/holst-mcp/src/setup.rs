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
    error::Reason,
    form::{Form, Question},
    line::QuestionKind,
    terminal::{Terminal, TerminalKind},
};
use secrecy::SecretString;

use crate::{
    error::SetupError,
    login::store::{Backend as _, SystemBackend},
};

mod connect;
pub(crate) use connect::{Clients, Program, SERVER, System, stable_binary};

/// Tries for the code or password, as the CLI allows.
pub(crate) const ATTEMPTS: u8 = 3;

/// What the second step asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Secret {
    Code,
    Password,
}

impl Secret {
    pub(crate) const fn noun(self) -> &'static str {
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
            Secret::Code => Reason::WrongCode,
            Secret::Password => Reason::WrongPassword,
        }
    }
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
    let piped = form.terminal().kind() == TerminalKind::Redirected;
    match (steps, clients) {
        (Steps::Both | Steps::ConnectOnly, Clients::Named(_)) if piped => {
            backend
                .load()
                .ok()
                .flatten()
                .ok_or(SetupError::NoTerminal)?;
            connect::connect(programs, form, binary, clients)
        }
        (Steps::Both, _) => {
            run(backend, form).await?;
            connect::connect(programs, form, binary, clients)
        }
        (Steps::LoginOnly, _) => run(backend, form).await,
        (Steps::ConnectOnly, _) => connect::connect(programs, form, binary, clients),
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
            let client = ask_until_accepted(&awaiting, form).await?;
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
                        "A login is saved already (Holst could not check it: {}).",
                        holst_core::error::chained(&err)
                    ),
                    client,
                ),
            }
        }
        Ok(None) => Ok(Saved::Replace(None)),
        Err(err) => {
            form.step(
                "The saved login could not be read",
                &holst_core::error::chained(&err).to_string(),
            )?;
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
    text.trim().parse().map_err(|_| Reason::NotEmail)
}

fn parse_secret(text: &str) -> Result<SecretString, Reason> {
    (!text.is_empty())
        .then(|| SecretString::from(text))
        .ok_or(Reason::Empty)
}

/// Asks for a code or password until Holst takes it, `ATTEMPTS` times at most.
async fn ask_until_accepted<T: Terminal>(
    awaiting: &Awaiting,
    form: &Form<T>,
) -> Result<Client, SetupError> {
    let secret = secret_of(awaiting);
    let logged_in = form
        .ask_until(
            secret.question(),
            &parse_secret,
            async |typed: &SecretString| awaiting.log_in(typed).await,
            |err| err.is_wrong_secret().then_some(secret.wrong()),
            ATTEMPTS,
        )
        .await?;
    logged_in.map_err(|err| match (err.is_wrong_secret(), err) {
        (true, _) => SetupError::Refused(secret),
        (false, err) => err.into(),
    })
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
        .map(|hint| format!("; {}", hint.text(Surface::Mcp)))
        .unwrap_or_default();
    format!("{}{hint}", holst_core::error::chained(stale))
}

#[cfg(test)] mod tests;
