//! `holst-mcp setup`: log in from a terminal and connect the MCP clients.

use std::io::{self, BufRead, Write};

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
use secrecy::{ExposeSecret as _, SecretString};

use crate::login::store::{Backend as _, SystemBackend};

mod connect;
pub(crate) use connect::{Program, SERVER, System};

/// Tries for the address and for the code or password, as the CLI allows.
const ATTEMPTS: u8 = 3;

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

    const fn label(self) -> &'static str {
        match self {
            Secret::Code => "Code",
            Secret::Password => "Password",
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

    #[error("That does not look like an email address ({ATTEMPTS} tries used).")]
    BadEmail,

    #[error(
        "The {} was not accepted {ATTEMPTS} times. Run holst-mcp setup to try again.",
        .0.noun()
    )]
    Refused(Secret),

    #[error("The input ended before the login was finished.")]
    EndOfInput,

    #[error("Could not read from or write to the terminal.")]
    Terminal(#[source] io::Error),

    #[error("Could not log in")]
    Holst(#[from] holst_core::Error),
}

/// The person at the terminal: questions go out, one line comes back.
pub(crate) struct Talk<R, W> {
    input: R,
    out: W,
    /// Turn the echo off for a code or password (the real terminal only).
    hide_secrets: bool,
}

impl<R: BufRead, W: Write> Talk<R, W> {
    pub(crate) const fn new(input: R, out: W, hide_secrets: bool) -> Self {
        Self {
            input,
            out,
            hide_secrets,
        }
    }

    fn say(&mut self, text: &str) -> Result<(), SetupError> {
        writeln!(self.out, "{text}").map_err(SetupError::Terminal)
    }

    fn ask(&mut self, label: &str) -> Result<String, SetupError> {
        write!(self.out, "{label}: ")
            .and_then(|()| self.out.flush())
            .map_err(SetupError::Terminal)?;
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) => Err(SetupError::EndOfInput),
            Ok(_) => {
                let end = line.trim_end_matches(['\r', '\n']).len();
                line.truncate(end);
                Ok(line)
            }
            Err(err) => Err(SetupError::Terminal(err)),
        }
    }

    fn ask_secret(&mut self, secret: Secret) -> Result<SecretString, SetupError> {
        let quiet = self
            .hide_secrets
            .then(echo::off)
            .transpose()
            .map_err(SetupError::Terminal)?;
        let line = self.ask(secret.label());
        // The Enter of the person was not echoed: end their line.
        if quiet.is_some() {
            self.say("")?;
        }
        line.map(SecretString::from)
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

/// The steps in order: the login, then the connection of the clients.
pub(crate) async fn set_up<R: BufRead, W: Write>(
    backend: &SystemBackend,
    talk: &mut Talk<R, W>,
    programs: &impl Program,
    binary: &str,
    steps: Steps,
) -> Result<(), SetupError> {
    match steps {
        Steps::Both => {
            run(backend, talk).await?;
            connect::connect(programs, talk, binary)
        }
        Steps::LoginOnly => run(backend, talk).await,
        Steps::ConnectOnly => connect::connect(programs, talk, binary),
    }
}

/// Logs in and saves the login for the server; asks first when one is saved.
pub(crate) async fn run<R: BufRead, W: Write>(
    backend: &SystemBackend,
    talk: &mut Talk<R, W>,
) -> Result<(), SetupError> {
    match saved_login(backend, talk).await? {
        Saved::Keep => Ok(()),
        Saved::Replace(old) => {
            let (email, awaiting) = begin(backend, talk).await?;
            let client = ask_until_accepted(&awaiting, talk, ATTEMPTS).await?;
            keep(backend, client, old, &email, talk).await
        }
    }
}

/// What to do about the login saved already.
enum Saved {
    Keep,
    /// Log in again, then end this login (none when it is unusable).
    Replace(Option<Client>),
}

async fn saved_login<R: BufRead, W: Write>(
    backend: &SystemBackend,
    talk: &mut Talk<R, W>,
) -> Result<Saved, SetupError> {
    match backend.load() {
        Ok(Some((session, _))) => {
            let client = backend.connect(session)?;
            match holst_core::command::account::check(&client).await {
                SessionCheck::Active(me) => {
                    let known = me.name.map_or_else(
                        || "Logged in already.".to_owned(),
                        |name| format!("Logged in as {name} already."),
                    );
                    Ok(replace(confirm(&known, talk)?, client))
                }
                SessionCheck::Revoked(_) => {
                    talk.say("The saved login is not accepted any more.")?;
                    Ok(Saved::Replace(None))
                }
                SessionCheck::Unknown(err) => Ok(replace(
                    confirm(
                        &format!(
                            "A login is saved already (Holst could not check it: {err})."
                        ),
                        talk,
                    )?,
                    client,
                )),
            }
        }
        Ok(None) => Ok(Saved::Replace(None)),
        Err(err) => {
            talk.say(&format!("The saved login could not be read: {err}"))?;
            Ok(Saved::Replace(None))
        }
    }
}

fn replace(yes: bool, old: Client) -> Saved {
    yes.then_some(old)
        .map_or(Saved::Keep, |old| Saved::Replace(Some(old)))
}

fn confirm<R: BufRead, W: Write>(
    known: &str,
    talk: &mut Talk<R, W>,
) -> Result<bool, SetupError> {
    let answer = talk.ask(&format!("{known} Log in again? [y/N]"))?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

async fn begin<R: BufRead, W: Write>(
    backend: &SystemBackend,
    talk: &mut Talk<R, W>,
) -> Result<(Email, Awaiting), SetupError> {
    let email = ask_email(talk, ATTEMPTS)?;
    let start = EmailLogin::new(backend.anonymous()?, email.clone());
    let methods = start.methods().await?;
    match methods.route() {
        LoginRoute::NoAccount => Err(SetupError::NoAccount),
        LoginRoute::Code => {
            let sent = start.send_code().await?;
            talk.say(&format!("A code was sent to {}.", email.as_str()))?;
            Ok(sent)
        }
        LoginRoute::Password => Ok(start.with_password()),
        LoginRoute::SingleSignOn => Err(SetupError::SsoOnly),
    }
    .map(|awaiting| (email, awaiting))
}

fn ask_email<R: BufRead, W: Write>(
    talk: &mut Talk<R, W>,
    left: u8,
) -> Result<Email, SetupError> {
    talk.ask("Email")?.trim().parse::<Email>().or_else(|_| {
        talk.say("That does not look like an email address.")?;
        if left > 1 {
            ask_email(talk, left - 1)
        } else {
            Err(SetupError::BadEmail)
        }
    })
}

/// What the server made of a typed code or password.
enum Tried {
    Accepted(Client),
    Refused(String),
}

async fn try_secret(
    awaiting: &Awaiting,
    typed: &SecretString,
) -> Result<Tried, SetupError> {
    let noun = secret_of(awaiting).noun();
    if typed.expose_secret().trim().is_empty() {
        Ok(Tried::Refused(format!("Enter the {noun}.")))
    } else {
        match awaiting.log_in(typed).await {
            Ok(client) => Ok(Tried::Accepted(client)),
            Err(err) if err.is_wrong_secret() => {
                Ok(Tried::Refused(format!("That {noun} was not accepted.")))
            }
            Err(err) => Err(err.into()),
        }
    }
}

async fn ask_until_accepted<R: BufRead, W: Write>(
    awaiting: &Awaiting,
    talk: &mut Talk<R, W>,
    left: u8,
) -> Result<Client, SetupError> {
    let typed = talk.ask_secret(secret_of(awaiting))?;
    match try_secret(awaiting, &typed).await? {
        Tried::Accepted(client) => Ok(client),
        Tried::Refused(words) if left > 1 => {
            talk.say(&format!("{words} Try again."))?;
            Box::pin(ask_until_accepted(awaiting, talk, left - 1)).await
        }
        Tried::Refused(words) => {
            talk.say(&words)?;
            Err(SetupError::Refused(secret_of(awaiting)))
        }
    }
}

/// Checks the login works, saves it for the server and ends the replaced one.
async fn keep<R: BufRead, W: Write>(
    backend: &SystemBackend,
    client: Client,
    old: Option<Client>,
    email: &Email,
    talk: &mut Talk<R, W>,
) -> Result<(), SetupError> {
    let done = login::finish(client, old, async |session| backend.save_new(&session))
        .await
        .map_err(|stopped| match stopped {
            Stopped::Check(err) | Stopped::Save(err) => err,
        })?;
    let saved = done.saved;
    talk.say(&format!(
        "Logged in as {}. The MCP server picks the login up on its next call.",
        email.as_str()
    ))?;
    saved
        .stale
        .as_ref()
        .map_or(Ok(()), |stale| talk.say(&stale_warning(stale)))
}

/// An older login that the server still reads first, with what to do.
fn stale_warning(stale: &holst_core::Error) -> String {
    let hint = stale
        .hint()
        .map(|hint| format!("; {}", hint.on(Surface::Mcp)))
        .unwrap_or_default();
    format!("Warning: {stale}{hint}")
}

/// Echo off while a secret is typed.
#[cfg(unix)]
mod echo {
    use std::{io, mem::MaybeUninit};

    use tokio::signal::unix::{SignalKind, signal};

    /// Puts the terminal settings back when dropped or on a stop signal.
    // GUARD: the terminal must not stay silent after the question.
    pub(super) struct Quiet {
        original: libc::termios,
        on_signal: tokio::task::JoinHandle<()>,
    }

    #[expect(
        clippy::exit,
        clippy::disallowed_methods,
        reason = "the signal handler ends the process once the terminal is back"
    )]
    pub(super) fn off() -> io::Result<Quiet> {
        let mut settings = MaybeUninit::<libc::termios>::zeroed();
        // SAFETY: `tcgetattr` only writes a `termios` into a sized, aligned local.
        let status =
            unsafe { libc::tcgetattr(libc::STDIN_FILENO, settings.as_mut_ptr()) };
        let original = (status == 0)
            // SAFETY: the call succeeded, so the value is initialised.
            .then(|| unsafe { settings.assume_init() })
            .ok_or_else(io::Error::last_os_error)?;
        // The listeners exist before the echo goes off: no signal slips by.
        let (mut interrupt, mut terminate, mut hangup) = (
            signal(SignalKind::interrupt())?,
            signal(SignalKind::terminate())?,
            signal(SignalKind::hangup())?,
        );
        let mut quiet = original;
        quiet.c_lflag &= !libc::ECHO;
        set(&quiet)?;
        // A stop signal would end the process with the terminal silent.
        let on_signal = tokio::spawn(async move {
            let code = tokio::select! {
                _ = interrupt.recv() => 130,
                _ = terminate.recv() => 143,
                _ = hangup.recv() => 129,
            };
            drop(set(&original));
            std::process::exit(code);
        });
        Ok(Quiet {
            original,
            on_signal,
        })
    }

    /// `TCSANOW` as in the CLI: typed-ahead input stays.
    fn set(settings: &libc::termios) -> io::Result<()> {
        // SAFETY: `tcsetattr` only reads the `termios` behind the reference.
        let status =
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, settings) };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    impl Drop for Quiet {
        fn drop(&mut self) {
            self.on_signal.abort();
            // Nothing to tell: the shell resets a terminal it finds silent.
            drop(set(&self.original));
        }
    }
}

/// Windows: the secret is read as a plain line, so it shows while typed.
#[cfg(not(unix))]
mod echo {
    use std::io;

    pub(super) struct Quiet;

    #[expect(clippy::unnecessary_wraps, reason = "the signature of the unix one")]
    pub(super) fn off() -> io::Result<Quiet> {
        Ok(Quiet)
    }
}

#[cfg(test)] mod tests;
