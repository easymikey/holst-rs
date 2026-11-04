//! `holst-mcp setup`: log in from a terminal and connect the MCP clients.

use std::num::NonZeroU8;

use clacky::{
    CharFilter,
    NoteLine,
    NoteTone,
    Password,
    Tone,
    prelude::{Ask as _, Peek as _, Reply, Session, confirm, password, text},
    terminal::{Terminal, TerminalKind},
};
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

use crate::{
    error::SetupError,
    login::store::{Backend as _, SystemBackend},
};

mod connect;
pub(crate) use connect::{Clients, Program, SERVER, System, stable_binary};

/// Tries for the code or password, as the CLI allows.
pub(crate) const ATTEMPTS: NonZeroU8 = NonZeroU8::MIN.saturating_add(2);

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

    fn question(self) -> Password {
        match self {
            Secret::Code => password("Code")
                .placeholder("sent to your email")
                .accept(CharFilter::NoSpaces),
            Secret::Password => {
                password("Password").placeholder("type the password — it stays hidden")
            }
        }
    }

    const fn wrong(self) -> &'static str {
        match self {
            Secret::Code => {
                "that code did not match — check the latest email and type it again"
            }
            Secret::Password => "that password did not match — try again",
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

/// A line on the rail: what happened, and under it what came of it when
/// there is anything to say.
fn say<T: Terminal>(
    ui: &Session<T>,
    tone: Tone,
    label: &str,
    detail: &str,
) -> clacky::Result<()> {
    if detail.is_empty() {
        ui.log(tone, label)
    } else {
        ui.log(tone, format_args!("{label}\n{detail}"))
    }
}

/// The steps in order: the login, then the connection of the `clients`.
pub(crate) async fn set_up<T: Terminal>(
    backend: &SystemBackend,
    ui: &Session<T>,
    programs: &impl Program,
    binary: &str,
    steps: Steps,
    clients: &Clients,
) -> Result<(), SetupError> {
    let piped = ui.terminal().kind() == TerminalKind::Redirected;
    match (steps, clients) {
        (Steps::Both | Steps::ConnectOnly, Clients::Named(_)) if piped => {
            backend
                .load()
                .ok()
                .flatten()
                .ok_or(SetupError::NoTerminal)?;
            connect::connect(programs, ui, binary, clients)
        }
        (Steps::Both, _) => {
            run(backend, ui).await?;
            connect::connect(programs, ui, binary, clients)
        }
        (Steps::LoginOnly, _) => run(backend, ui).await,
        (Steps::ConnectOnly, _) => connect::connect(programs, ui, binary, clients),
    }?;
    Ok(ui.outro("Done")?)
}

/// Logs in and saves the login for the server; asks first when one is saved.
pub(crate) async fn run<T: Terminal>(
    backend: &SystemBackend,
    ui: &Session<T>,
) -> Result<(), SetupError> {
    match saved_login(backend, ui).await? {
        Saved::Keep => Ok(()),
        Saved::Replace(old) => {
            let (email, awaiting) = begin(backend, ui).await?;
            let client = ask_until_accepted(&awaiting, ui).await?;
            keep(backend, client, old, &email, ui).await
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
    ui: &Session<T>,
) -> Result<Saved, SetupError> {
    match backend.load() {
        Ok(Some((session, _))) => {
            let client = backend.connect(session)?;
            let check = ui
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
                    replace(ui, &known, client)
                }
                SessionCheck::Revoked(_) => {
                    say(
                        ui,
                        Tone::Warn,
                        "The saved login is not accepted any more",
                        "",
                    )?;
                    Ok(Saved::Replace(None))
                }
                SessionCheck::Unknown(err) => replace(
                    ui,
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
            say(
                ui,
                Tone::Warn,
                "The saved login could not be read",
                &holst_core::error::chained(&err).to_string(),
            )?;
            Ok(Saved::Replace(None))
        }
    }
}

fn replace<T: Terminal>(
    ui: &Session<T>,
    known: &str,
    old: Client,
) -> Result<Saved, SetupError> {
    Ok(ui
        .ask(confirm(format!("{known} Log in again?")).default(Reply::No))?
        .then_some(old)
        .map_or(Saved::Keep, |old| Saved::Replace(Some(old))))
}

async fn begin<T: Terminal>(
    backend: &SystemBackend,
    ui: &Session<T>,
) -> Result<(Email, Awaiting), SetupError> {
    let email: Email = ui
        .ask(
            text("Email")
                .placeholder("you@example.com")
                .validate(check_email),
        )?
        .parse()?;
    let start = EmailLogin::new(backend.anonymous()?, email.clone());
    let methods = ui.wait("Checking the address", start.methods()).await??;
    match methods.route() {
        LoginRoute::NoAccount => Err(SetupError::NoAccount),
        LoginRoute::Code => {
            let sent = ui.wait("Sending the code", start.send_code()).await??;
            say(ui, Tone::Success, "A code was sent to", email.as_str())?;
            Ok(sent)
        }
        LoginRoute::Password => Ok(start.with_password()),
        LoginRoute::SingleSignOn => Err(SetupError::SsoOnly),
    }
    .map(|awaiting| (email, awaiting))
}

fn check_email(typed: &str) -> Result<(), &'static str> {
    typed
        .parse::<Email>()
        .map(drop)
        .map_err(|_| "enter an address like you@example.com")
}

fn check_secret(typed: &str) -> Result<(), &'static str> {
    if typed.is_empty() {
        Err("this cannot be empty")
    } else {
        Ok(())
    }
}

/// Asks for a code or password until Holst takes it, `ATTEMPTS` times at most.
async fn ask_until_accepted<T: Terminal>(
    awaiting: &Awaiting,
    ui: &Session<T>,
) -> Result<Client, SetupError> {
    let secret = secret_of(awaiting);
    let logged_in = ui
        .ask_until(
            || secret.question().validate(check_secret).secret(),
            "Signing in",
            async |typed: &_| awaiting.log_in(typed).await,
            |err: &holst_core::Error| {
                err.is_wrong_secret().then(|| secret.wrong().to_owned())
            },
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
    ui: &Session<T>,
) -> Result<(), SetupError> {
    let done = ui
        .wait(
            "Checking the login",
            login::finish(client, old, async |session| backend.save_new(&session)),
        )
        .await?
        .map_err(|stopped| match stopped {
            Stopped::Check(err) | Stopped::Save(err) => err,
        })?;
    let stale = done.saved.stale.as_ref().map(|stale| NoteLine {
        tone: NoteTone::Warn,
        text: stale_warning(stale),
    });
    let lines: Vec<_> = [
        NoteLine {
            tone: NoteTone::Plain,
            text: email.as_str().to_owned(),
        },
        NoteLine {
            tone: NoteTone::Faint,
            text: "The MCP server picks the login up on its next call.".to_owned(),
        },
    ]
    .into_iter()
    .chain(stale)
    .collect();
    Ok(ui.note("Logged in", &lines)?)
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
