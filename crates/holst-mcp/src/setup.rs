//! `holst-mcp setup`: log in from a terminal and connect the MCP clients.

use std::io::{self, BufRead, Write};

use holst_core::{
    command::{
        account::SessionCheck,
        login::email::{CodeSent, EmailLogin, PasswordChosen},
    },
    domain::login::Email,
    remote::client::Client,
};
use secrecy::{ExposeSecret as _, SecretString};

use crate::login::store::{Backend as _, SystemBackend};

mod connect;
pub(crate) use connect::{Program, System};

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
            Ok(_) => Ok(line.trim_end_matches(['\r', '\n']).to_owned()),
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

/// The login steps; `Awaiting` holds the flow for what is asked next.
enum Awaiting {
    Code(EmailLogin<CodeSent>),
    Password(EmailLogin<PasswordChosen>),
}

impl Awaiting {
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
    if !matches!(steps, Steps::ConnectOnly) {
        run(backend, talk).await?;
    }
    if steps.connects() {
        connect::connect(programs, talk, binary)?;
    }
    Ok(())
}

/// Logs in and saves the login for the server; asks first when one is saved.
pub(crate) async fn run<R: BufRead, W: Write>(
    backend: &SystemBackend,
    talk: &mut Talk<R, W>,
) -> Result<(), SetupError> {
    if wants_new_login(backend, talk).await? {
        let (email, awaiting) = begin(backend, talk).await?;
        let client = ask_until_accepted(&awaiting, talk, ATTEMPTS).await?;
        keep(backend, &client, &email, talk).await
    } else {
        Ok(())
    }
}

async fn wants_new_login<R: BufRead, W: Write>(
    backend: &SystemBackend,
    talk: &mut Talk<R, W>,
) -> Result<bool, SetupError> {
    match backend.load() {
        Ok(Some((session, _))) => {
            let client = backend.connect(session)?;
            match holst_core::command::account::check(&client).await {
                SessionCheck::Active(me) => {
                    let known = me.name.map_or_else(
                        || "Logged in already.".to_owned(),
                        |name| format!("Logged in as {name} already."),
                    );
                    confirm(&known, talk)
                }
                SessionCheck::Revoked(_) => {
                    talk.say("The saved login is not accepted any more.")?;
                    Ok(true)
                }
                SessionCheck::Unknown(err) => confirm(
                    &format!(
                        "A login is saved already (Holst could not check it: {err})."
                    ),
                    talk,
                ),
            }
        }
        Ok(None) => Ok(true),
        Err(err) => {
            talk.say(&format!("The saved login could not be read: {err}"))?;
            Ok(true)
        }
    }
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
    match (methods.has_account, methods.via_code, methods.via_password) {
        (false, _, _) => Err(SetupError::NoAccount),
        (true, true, _) => {
            let sent = start.send_code().await?;
            talk.say(&format!("A code was sent to {}.", email.as_str()))?;
            Ok(Awaiting::Code(sent))
        }
        (true, false, true) => Ok(Awaiting::Password(start.with_password())),
        (true, false, false) => Err(SetupError::SsoOnly),
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
    let noun = awaiting.secret().noun();
    if typed.expose_secret().trim().is_empty() {
        Ok(Tried::Refused(format!("Enter the {noun}.")))
    } else {
        match awaiting.log_in(typed).await {
            Ok(client) => Ok(Tried::Accepted(client)),
            Err(err) if matches!(err.status(), Some(400 | 401 | 403)) => {
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
    let typed = talk.ask_secret(awaiting.secret())?;
    match try_secret(awaiting, &typed).await? {
        Tried::Accepted(client) => Ok(client),
        Tried::Refused(words) if left > 1 => {
            talk.say(&format!("{words} Try again."))?;
            Box::pin(ask_until_accepted(awaiting, talk, left - 1)).await
        }
        Tried::Refused(words) => {
            talk.say(&words)?;
            Err(SetupError::Refused(awaiting.secret()))
        }
    }
}

/// Checks the login works, then saves it for the server.
async fn keep<R: BufRead, W: Write>(
    backend: &SystemBackend,
    client: &Client,
    email: &Email,
    talk: &mut Talk<R, W>,
) -> Result<(), SetupError> {
    let me = holst_core::command::account::account(client).await?;
    // Saved with the login: the board cache of the account is found by it.
    client.learn_account_id(&me.id);
    let session = client.rotated().or_else(|| client.session()).ok_or(
        holst_core::Error::Login(holst_core::error::login::LoginError::NoSavedLogin),
    )?;
    backend.save_new(&session)?;
    talk.say(&format!(
        "Logged in as {}. The MCP server picks the login up on its next call.",
        email.as_str()
    ))
}

/// Echo off while a secret is typed.
#[cfg(unix)]
mod echo {
    use std::{io, mem::MaybeUninit};

    /// Puts the terminal settings back when dropped.
    // GUARD: the terminal must not stay silent after the question.
    pub(super) struct Quiet(libc::termios);

    pub(super) fn off() -> io::Result<Quiet> {
        let mut settings = MaybeUninit::<libc::termios>::zeroed();
        // SAFETY: `tcgetattr` only writes a `termios` into a sized, aligned local.
        let status =
            unsafe { libc::tcgetattr(libc::STDIN_FILENO, settings.as_mut_ptr()) };
        let original = (status == 0)
            // SAFETY: the call succeeded, so the value is initialised.
            .then(|| unsafe { settings.assume_init() })
            .ok_or_else(io::Error::last_os_error)?;
        let mut quiet = original;
        quiet.c_lflag &= !libc::ECHO;
        set(&quiet)?;
        Ok(Quiet(original))
    }

    fn set(settings: &libc::termios) -> io::Result<()> {
        // SAFETY: `tcsetattr` only reads the `termios` behind the reference.
        let status =
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, settings) };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    impl Drop for Quiet {
        fn drop(&mut self) {
            // Nothing to tell: the shell resets a terminal it finds silent.
            drop(set(&self.0));
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
