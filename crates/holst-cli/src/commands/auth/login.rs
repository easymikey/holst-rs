//! `holst auth login`: the email form, from the address to a saved session.

#[cfg(test)] mod tests;

use std::num::NonZeroU8;

use clacky::{
    CharFilter,
    NoteLine,
    NoteTone,
    Password,
    prelude::{Peek as _, Prefill as _, Session as Ui, password, text},
    terminal::Terminal,
};
use holst_core::{
    command::{
        auth::{LoginArgs, SaveLogin},
        login,
        login::{
            LoggedIn,
            ReplacedLogin,
            Stopped,
            email::{Awaiting, EmailLogin},
        },
    },
    domain::{
        login::{Email, LoginRoute},
        session::Session,
    },
    error::hint::Surface,
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
};
use secrecy::SecretString;
use serde_json::json;

use crate::{
    error::{Error, Reason, Result, SecretKind},
    output,
    output::{
        OutputFormat,
        color_choice::ColorChoice,
        view::{self, display_name},
    },
    run::RunSettings,
    terminal::signals,
};

pub(crate) async fn login(
    login: LoginArgs,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let ui = Ui::detect()?.abort(signals::abort());
    ui.intro("holst auth login")?;
    match log_in(&ui, login, run).await {
        Ok((done, replaced)) => {
            finish(&ui, &done, replaced, where_saved(&done, run).as_deref())?;
            logged_in(format, &done, replaced)
        }
        Err(err) => cancel(&ui, &err).and(Err(err.shown())),
    }
}

fn cancel<T: Terminal>(ui: &Ui<T>, error: &Error) -> Result<()> {
    let (message, causes) = Some(error)
        .filter(|error| !matches!(error, Error::Interrupted | Error::Cancelled))
        .map_or_else(
            || (Error::Cancelled.to_string(), Vec::new()),
            |error| {
                (
                    holst_core::domain::text::printable(&error.to_string())
                        .into_owned(),
                    view::form_cause_lines(
                        error,
                        ColorChoice::colored_if(ui.terminal().has_color()),
                    ),
                )
            },
        );
    ui.cancel_with(message, causes).map_err(Error::from)
}

fn logged_in(format: OutputFormat, done: &Done, replaced: ReplacedLogin) -> Result<()> {
    match format {
        OutputFormat::Text => Ok(()),
        OutputFormat::Pretty | OutputFormat::Json => output::value(
            format,
            &json!({
                "name": done.name,
                "saved": done.store.map(SessionStoreKind::as_str),
                "cookies": done.cookies,
                "previous": replaced.as_str(),
            }),
        ),
    }
}

async fn log_in<T: Terminal>(
    ui: &Ui<T>,
    login: LoginArgs,
    run: &RunSettings,
) -> Result<(Done, ReplacedLogin)> {
    // Read only to end it later; an unreadable one is no reason to stop.
    let previous = match login.no_save {
        SaveLogin::Keep => ui
            .wait("Reading the saved login", async {
                Ok::<_, Error>(run.load().await.ok().flatten())
            })
            .await??
            .map(|(session, _)| session),
        SaveLogin::Skip => None,
    };
    // One saved for another server is not sent there to be ended.
    let previous = previous.and_then(|session| run.client(Some(session)).ok());
    let client = log_in_by_email(ui, run.client(None)?, login.email).await?;
    ui.wait(
        "Checking the login",
        conclude(client, previous, login.no_save, async |session| {
            run.save(session).await
        }),
    )
    .await?
}

fn finish<T: Terminal>(
    ui: &Ui<T>,
    done: &Done,
    replaced: ReplacedLogin,
    saved_to: Option<&str>,
) -> Result<()> {
    ui.note("Logged in", &done.lines(replaced, saved_to))?;
    Ok(ui.outro("Done")?)
}

fn where_saved(done: &Done, run: &RunSettings) -> Option<String> {
    done.store.map(|kind| match kind {
        SessionStoreKind::Keychain => kind.as_str().to_owned(),
        SessionStoreKind::File => run.location().file().map_or_else(
            || kind.as_str().to_owned(),
            |path| path.display().to_string(),
        ),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Done {
    name: String,
    store: Option<SessionStoreKind>,
    stale: Option<String>,
    cookies: usize,
}

const ATTEMPTS: NonZeroU8 = NonZeroU8::MIN.saturating_add(2);

async fn prepare<T: Terminal>(login: EmailLogin, ui: &Ui<T>) -> Result<Awaiting> {
    let methods = ui.wait("Checking the address", login.methods()).await??;
    let email = login.email().clone();
    match methods.route() {
        LoginRoute::NoAccount => Err(Error::NoAccount { email }),
        LoginRoute::SingleSignOn => Err(Error::SsoOnly { email }),
        LoginRoute::Password => Ok(login.with_password()),
        LoginRoute::Code => {
            Ok(ui.wait("Sending the code", login.send_code()).await??)
        }
    }
}

const fn secret_kind(awaiting: &Awaiting) -> SecretKind {
    match awaiting {
        Awaiting::Code(_) => SecretKind::Code,
        Awaiting::Password(_) => SecretKind::Password,
    }
}

async fn ask_and_log_in<T: Terminal>(
    awaiting: &Awaiting,
    ui: &Ui<T>,
) -> Result<Client> {
    let secret = secret_kind(awaiting);
    let logged_in = ui
        .ask_until(
            || secret_prompt(secret).validate(check_secret),
            "Signing in",
            async |typed| awaiting.log_in(&SecretString::from(typed.as_str())).await,
            |error| {
                error
                    .is_wrong_secret()
                    .then(|| secret.wrong_answer().to_string())
            },
            ATTEMPTS,
        )
        .await?;
    logged_in.map_err(|error| match (error.is_wrong_secret(), error) {
        (true, error) => Error::WrongSecret { secret, error },
        (false, error) => Error::Holst(error),
    })
}

/// Checks the login, saves it (unless `--no-save`) and ends the replaced one.
async fn conclude(
    client: Client,
    previous: Option<Client>,
    save: SaveLogin,
    save_session: impl AsyncFnOnce(Session) -> Result<LoginSaveReport>,
) -> Result<(Done, ReplacedLogin)> {
    let done = login::finish(client, previous, async |session| match save {
        SaveLogin::Keep => save_session(session).await.map(Some),
        SaveLogin::Skip => Ok(None),
    })
    .await
    .map_err(|stopped| match stopped {
        Stopped::Check(err) => Error::SessionNotAccepted(err),
        Stopped::Save(err) => err,
    })?;
    let replaced = done.replaced;
    Ok((Done::of(&done), replaced))
}

impl Done {
    fn of(done: &LoggedIn<Option<LoginSaveReport>>) -> Self {
        let stale = done
            .saved
            .as_ref()
            .and_then(|saved| saved.stale.as_ref())
            .map(|stale| {
                let hint = stale
                    .hint()
                    .map(|hint| format!("; {}", hint.text(Surface::Cli)))
                    .unwrap_or_default();
                format!("{}{hint}", holst_core::error::chained(stale))
            });
        Self {
            name: display_name(&done.account).to_owned(),
            store: done.saved.as_ref().map(|saved| saved.store),
            stale,
            cookies: done.client.cookie_count(),
        }
    }

    fn lines(&self, replaced: ReplacedLogin, saved_to: Option<&str>) -> Vec<NoteLine> {
        let cookies = match self.cookies {
            1 => "1 cookie".to_owned(),
            count => format!("{count} cookies"),
        };
        let saved = saved_to.map_or_else(
            || "not saved (--no-save)".to_owned(),
            |saved_to| format!("saved to {saved_to} · {cookies}"),
        );
        let previous = match replaced {
            ReplacedLogin::Nothing => None,
            ReplacedLogin::Terminated => Some(NoteLine {
                tone: NoteTone::Faint,
                text: "previous session terminated".to_owned(),
            }),
            ReplacedLogin::Kept => Some(NoteLine {
                tone: NoteTone::Warn,
                text: "could not terminate the previous session".to_owned(),
            }),
        };
        let stale = self.stale.clone().map(|text| NoteLine {
            tone: NoteTone::Warn,
            text,
        });
        [
            NoteLine {
                tone: NoteTone::Plain,
                text: self.name.clone(),
            },
            NoteLine {
                tone: NoteTone::Faint,
                text: saved,
            },
        ]
        .into_iter()
        .chain(stale)
        .chain(previous)
        .collect()
    }
}

const fn label_of(secret: SecretKind) -> &'static str {
    match secret {
        SecretKind::Code => "Code",
        SecretKind::Password => "Password",
    }
}

fn secret_prompt(secret: SecretKind) -> Password {
    match secret {
        SecretKind::Code => password(label_of(secret))
            .placeholder("sent to your email")
            .accept(CharFilter::NoSpaces),
        SecretKind::Password => password(label_of(secret))
            .placeholder("type the password — it stays hidden"),
    }
}

async fn log_in_by_email<T: Terminal>(
    ui: &Ui<T>,
    client: Client,
    given: Option<Email>,
) -> Result<Client> {
    let typed = ui.ask(
        text("Email")
            .placeholder("you@example.com")
            .validate(check_email)
            .given(given.map(|email| email.as_str().to_owned())),
    )?;
    let email = email_of(&typed).map_err(|reason| Error::InvalidAnswer {
        question: "Email".to_owned(),
        reason: reason.to_string(),
    })?;
    let awaiting = prepare(EmailLogin::new(client, email), ui).await?;
    ask_and_log_in(&awaiting, ui).await
}

fn email_of(text: &str) -> Result<Email, Reason> {
    text.parse().map_err(|_| Reason::NotEmail)
}

fn check_email(text: &str) -> Result<(), Reason> {
    email_of(text).map(drop)
}

fn check_secret(text: &str) -> Result<(), Reason> {
    if text.is_empty() {
        Err(Reason::Empty)
    } else {
        Ok(())
    }
}
