//! `holst auth login`: the email form, from the address to a saved session.

#[cfg(test)] mod tests;

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
use holst_form::error::Reason;
use secrecy::SecretString;
use serde_json::json;

use crate::{
    error::{Error, InvalidAnswer, Result, SecretKind},
    form::{
        Form,
        Question,
        draw::NoteTone,
        line::QuestionKind,
        terminal::{Console, Terminal, TerminalKind},
    },
    output,
    output::{
        OutputFormat,
        coloring::Coloring,
        view::{self, display_name},
    },
    run::RunSettings,
};

pub(crate) async fn login(
    login: LoginArgs,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    // The form quiets the terminal first: Ctrl-C works from here on.
    let form = Form::intro(Console::detect(run.terminal_type()), "holst auth login")?;
    match log_in(&form, login, run).await {
        Ok((done, replaced)) => {
            finish(&form, &done, replaced, where_saved(&done, run).as_deref())?;
            logged_in(format, &done, replaced)
        }
        Err(err) => cancel(&form, &err).and(Err(err.shown())),
    }
}

fn cancel<T: Terminal>(form: &Form<T>, error: &Error) -> Result<()> {
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
                        Coloring::colored_if(form.terminal().has_color()),
                    ),
                )
            },
        );
    form.cancel(&message, &causes).map_err(Error::from)
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
    form: &Form<T>,
    login: LoginArgs,
    run: &RunSettings,
) -> Result<(Done, ReplacedLogin)> {
    // Read only to end it later; an unreadable one is no reason to stop.
    let previous = match login.no_save {
        SaveLogin::Keep => form
            .wait("Reading the saved login", async {
                Ok::<_, Error>(run.load().await.ok().flatten())
            })
            .await??
            .map(|(session, _)| session),
        SaveLogin::Skip => None,
    };
    let previous = previous
        .map(|session| run.client(Some(session)))
        .transpose()?;
    let client = log_in_by_email(form, run.client(None)?, login.email).await?;
    form.wait(
        "Checking the login",
        conclude(client, previous, login.no_save, async |session| {
            run.save(session).await
        }),
    )
    .await?
}

fn finish<T: Terminal>(
    form: &Form<T>,
    done: &Done,
    replaced: ReplacedLogin,
    saved_to: Option<&str>,
) -> Result<()> {
    form.write_box("Logged in", &done.lines(replaced, saved_to))?;
    Ok(form.outro("Done")?)
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

const ATTEMPTS: u8 = 3;

async fn prepare<T: Terminal>(login: EmailLogin, form: &Form<T>) -> Result<Awaiting> {
    let methods = form.wait("Checking the address", login.methods()).await??;
    let email = login.email().clone();
    match methods.route() {
        LoginRoute::NoAccount => Err(Error::NoAccount { email }),
        LoginRoute::SingleSignOn => Err(Error::SsoOnly { email }),
        LoginRoute::Password => Ok(login.with_password()),
        LoginRoute::Code => {
            Ok(form.wait("Sending the code", login.send_code()).await??)
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
    awaiting: Awaiting,
    form: &Form<T>,
    refused: Option<InvalidAnswer>,
    attempts_left: u8,
) -> Result<Client> {
    let secret = secret_kind(&awaiting);
    let Question { label, kind } = question(secret);
    let typed = refused.map_or_else(
        || form.ask(label, kind, &reason_of(parse_secret)),
        |invalid| {
            form.ask_again(
                Question { label, kind },
                &reason_of(parse_secret),
                invalid.reason(),
            )
        },
    )?;
    let logged_in = form
        .wait("Signing in", async {
            Ok::<_, Error>(awaiting.log_in(&typed).await)
        })
        .await??;
    let can_ask_again =
        attempts_left > 1 && form.terminal().kind() != TerminalKind::Redirected;
    match logged_in.map_err(|err| (err.is_wrong_secret(), err)) {
        Ok(client) => Ok(client),
        Err((true, _)) if can_ask_again => {
            let problem = secret.wrong_answer();
            Box::pin(ask_and_log_in(
                awaiting,
                form,
                Some(problem),
                attempts_left - 1,
            ))
            .await
        }
        Err((true, err)) => Err(Error::WrongSecret { secret, error: err }),
        Err((false, err)) => Err(Error::Holst(err)),
    }
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
                    .map(|hint| format!("; {}", hint.on(Surface::Cli)))
                    .unwrap_or_default();
                format!("{stale}{hint}")
            });
        Self {
            name: display_name(&done.account).to_owned(),
            store: done.saved.as_ref().map(|saved| saved.store),
            stale,
            cookies: done.client.cookie_count(),
        }
    }

    fn lines(
        &self,
        replaced: ReplacedLogin,
        saved_to: Option<&str>,
    ) -> Vec<(NoteTone, String)> {
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
            ReplacedLogin::Terminated => {
                Some((NoteTone::Faint, "previous session terminated".to_owned()))
            }
            ReplacedLogin::Kept => Some((
                NoteTone::Warn,
                "could not terminate the previous session".to_owned(),
            )),
        };
        let stale = self.stale.clone().map(|stale| (NoteTone::Warn, stale));
        [
            (NoteTone::Plain, self.name.clone()),
            (NoteTone::Faint, saved),
        ]
        .into_iter()
        .chain(stale)
        .chain(previous)
        .collect()
    }
}

const fn question(secret: SecretKind) -> Question {
    match secret {
        SecretKind::Code => Question {
            label: "Code",
            kind: QuestionKind::LoginCode,
        },
        SecretKind::Password => Question {
            label: "Password",
            kind: QuestionKind::Password,
        },
    }
}

async fn log_in_by_email<T: Terminal>(
    form: &Form<T>,
    client: Client,
    given: Option<Email>,
) -> Result<Client> {
    let email = given.map_or_else(
        || form.ask("Email", QuestionKind::Text, &reason_of(parse_email)),
        |email| form.step("Email", email.as_str()).map(|()| email),
    )?;
    let awaiting = prepare(EmailLogin::new(client, email), form).await?;
    ask_and_log_in(awaiting, form, None, ATTEMPTS).await
}

// The form shows the reason as text; the command keeps the enum.
fn reason_of<V>(
    parse: impl Fn(&str) -> Result<V, InvalidAnswer>,
) -> impl Fn(&str) -> Result<V, Reason> {
    move |text| parse(text).map_err(InvalidAnswer::reason)
}

fn parse_email(text: &str) -> Result<Email, InvalidAnswer> {
    text.parse().map_err(|_| InvalidAnswer::NotEmail)
}

fn parse_secret(text: &str) -> Result<SecretString, InvalidAnswer> {
    if text.is_empty() {
        Err(InvalidAnswer::Empty)
    } else {
        Ok(SecretString::from(text))
    }
}
