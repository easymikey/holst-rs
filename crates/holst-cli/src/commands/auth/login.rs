//! `holst auth login`: the email form, from the address to a saved session.

#[cfg(test)] mod tests;

use holst_core::{
    command::{
        auth::{LoginArgs, SaveLogin},
        login,
        login::{
            email as email_flow,
            email::{CodeSent, EmailLogin, PasswordChosen},
        },
    },
    domain::{
        login::{Email, LoginMethods},
        session::Session,
    },
    error::hint::Surface,
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
};
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
    output::{OutputFormat, view::display_name},
    run::{RunSettings, latest_session},
};

pub(crate) async fn login(
    login: LoginArgs,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    // The form quiets the terminal first: Ctrl-C works from here on.
    let form = Form::intro(Console::detect(run.terminal_type()), "holst auth login")?;
    match log_in(&form, login, run).await {
        Ok((done, old)) => {
            let replaced = end_previous_session(&form, old).await;
            finish(&form, &done, replaced, where_saved(&done, run).as_deref())?;
            logged_in(format, &done, replaced)
        }
        Err(err) => form.cancel(&err).and(Err(err.shown())),
    }
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
) -> Result<(Done, Option<Result<Client>>)> {
    // Read only to end it later; an unreadable one is no reason to stop.
    let previous = match login.no_save {
        SaveLogin::Keep => form
            .wait("Reading the saved login", async {
                Ok::<_, Error>(run.load().await.ok().flatten())
            })
            .await?
            .map(|(session, _)| session),
        SaveLogin::Skip => None,
    };
    let client = log_in_by_email(form, run.client(None)?, login.email).await?;
    let verified = form.wait("Checking the login", verify(client)).await?;
    let done = verified
        .keep(form, login.no_save, async |session| run.save(session).await)
        .await?;
    Ok((done, previous.map(|session| run.client(Some(session)))))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplacedLogin {
    Nothing,
    Terminated,
    Kept,
}

impl ReplacedLogin {
    const fn as_str(self) -> Option<&'static str> {
        match self {
            ReplacedLogin::Nothing => None,
            ReplacedLogin::Terminated => Some("terminated"),
            ReplacedLogin::Kept => Some("kept"),
        }
    }
}

async fn end_previous_session<T: Terminal>(
    form: &Form<T>,
    old: Option<Result<Client>>,
) -> ReplacedLogin {
    match old {
        None => ReplacedLogin::Nothing,
        Some(Err(_)) => ReplacedLogin::Kept,
        Some(Ok(client)) => form
            .wait("Terminating the previous session", async {
                Ok::<_, Error>(login::logout(&client).await)
            })
            .await
            .map_or(ReplacedLogin::Kept, |ended| {
                ended.map_or(ReplacedLogin::Kept, |_| ReplacedLogin::Terminated)
            }),
    }
}

fn finish<T: Terminal>(
    form: &Form<T>,
    done: &Done,
    replaced: ReplacedLogin,
    saved_to: Option<&str>,
) -> Result<()> {
    form.write_box("Logged in", &done.lines(replaced, saved_to))?;
    form.outro("Done")
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

#[derive(Debug)]
enum Awaiting {
    Code(EmailLogin<CodeSent>),
    Password(EmailLogin<PasswordChosen>),
}

#[derive(Debug)]
struct Verified {
    client: Client,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Done {
    name: String,
    store: Option<SessionStoreKind>,
    stale: Option<String>,
    cookies: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginMethod {
    NoAccount,
    Code,
    Password,
    SsoOnly,
}

impl From<&LoginMethods> for LoginMethod {
    fn from(methods: &LoginMethods) -> Self {
        match (methods.has_account, methods.via_code, methods.via_password) {
            (false, _, _) => LoginMethod::NoAccount,
            (true, true, _) => LoginMethod::Code,
            (true, false, true) => LoginMethod::Password,
            (true, false, false) => LoginMethod::SsoOnly,
        }
    }
}

const ATTEMPTS: u8 = 3;

async fn prepare<T: Terminal>(
    login: EmailLogin<email_flow::LoginStart>,
    form: &Form<T>,
) -> Result<Awaiting> {
    let methods = form.wait("Checking the address", login.methods()).await?;
    let email = login.email().clone();
    match LoginMethod::from(&methods) {
        LoginMethod::NoAccount => Err(Error::NoAccount { email }),
        LoginMethod::SsoOnly => Err(Error::SsoOnly { email }),
        LoginMethod::Password => Ok(Awaiting::Password(login.with_password())),
        LoginMethod::Code => {
            let sent = form.wait("Sending the code", login.send_code()).await?;
            Ok(Awaiting::Code(sent))
        }
    }
}

impl Awaiting {
    const fn secret_kind(&self) -> SecretKind {
        match self {
            Awaiting::Code(_) => SecretKind::Code,
            Awaiting::Password(_) => SecretKind::Password,
        }
    }

    async fn submit(&self, typed: &SecretString) -> holst_core::Result<Client> {
        match self {
            Awaiting::Code(login) => login.log_in(typed).await,
            Awaiting::Password(login) => login.log_in(typed).await,
        }
    }

    async fn log_in<T: Terminal>(
        self,
        form: &Form<T>,
        refused: Option<InvalidAnswer>,
        attempts_left: u8,
    ) -> Result<Client> {
        let Question { label, kind } = question(self.secret_kind());
        let typed = refused.map_or_else(
            || form.ask(label, kind, &parse_secret),
            |invalid| form.ask_again(Question { label, kind }, &parse_secret, invalid),
        )?;
        let logged_in = form
            .wait("Signing in", async {
                Ok::<_, Error>(self.submit(&typed).await)
            })
            .await?;
        let can_ask_again =
            attempts_left > 1 && form.terminal().kind() != TerminalKind::Redirected;
        match logged_in.map_err(|err| (is_wrong_secret(&err), err)) {
            Ok(client) => Ok(client),
            Err((true, _)) if can_ask_again => {
                let problem = self.secret_kind().wrong_answer();
                Box::pin(self.log_in(form, Some(problem), attempts_left - 1)).await
            }
            Err((true, err)) => Err(Error::WrongSecret {
                secret: self.secret_kind(),
                error: err,
            }),
            Err((false, err)) => Err(Error::Holst(err)),
        }
    }
}

async fn verify(client: Client) -> Result<Verified> {
    let me = holst_core::command::account::account(&client)
        .await
        .map_err(Error::SessionNotAccepted)?;
    // Saved with the login: `--offline` then knows whose board cache to read.
    client.learn_account_id(&me.id);
    Ok(Verified {
        name: display_name(&me).to_owned(),
        client,
    })
}

impl Verified {
    async fn keep<T: Terminal>(
        self,
        form: &Form<T>,
        save: SaveLogin,
        save_session: impl AsyncFnOnce(Session) -> Result<LoginSaveReport>,
    ) -> Result<Done> {
        let session = latest_session(&self.client).ok_or(Error::Holst(
            holst_core::Error::Login(
                holst_core::error::login::LoginError::NoSavedLogin,
            ),
        ))?;
        let saved = match save {
            SaveLogin::Keep => {
                Some(form.wait("Saving the login", save_session(session)).await?)
            }
            SaveLogin::Skip => None,
        };
        let stale =
            saved
                .as_ref()
                .and_then(|saved| saved.stale.as_ref())
                .map(|stale| {
                    let hint = stale
                        .hint()
                        .map(|hint| format!("; {}", hint.on(Surface::Cli)))
                        .unwrap_or_default();
                    format!("{stale}{hint}")
                });
        Ok(Done {
            name: self.name,
            store: saved.map(|saved| saved.store),
            stale,
            cookies: self.client.cookie_count(),
        })
    }
}

impl Done {
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

fn is_wrong_secret(err: &holst_core::Error) -> bool {
    matches!(err.status(), Some(400 | 401 | 403))
}

async fn log_in_by_email<T: Terminal>(
    form: &Form<T>,
    client: Client,
    given: Option<Email>,
) -> Result<Client> {
    let email = given.map_or_else(
        || form.ask("Email", QuestionKind::Text, &parse_email),
        |email| form.step("Email", email.as_str()).map(|()| email),
    )?;
    prepare(EmailLogin::new(client, email), form)
        .await?
        .log_in(form, None, ATTEMPTS)
        .await
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
