//! `holst auth status`: the saved login, the account and its workspaces.

use std::fmt;

use holst_core::{
    command::auth::{
        self,
        AccountStatus,
        NodeCounts,
        StatusArgs,
        StatusReport,
        WorkspaceStatus,
    },
    domain::{
        access::{Session, Token},
        text::counted,
    },
    error::{
        hint::{LOGIN, Surface},
        login::LoginError,
    },
    storage::session_store::SessionStoreKind,
};
use serde::Serialize;

use crate::{
    error::{Error, Result},
    output,
    output::{OutputFormat, view::display_name},
    run::{RunSettings, save_rotated, warn_unsaved},
};

pub(crate) async fn status(
    args: StatusArgs,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match run.load().await? {
        Some((session, kind)) => saved_status(&args, run, format, session, kind).await,
        // The report says why; the exit code lets a script know.
        None => output::report(
            format,
            &MissingLogin {
                logged_in: false,
                file: run
                    .location()
                    .file()
                    .map(|path| path.to_string_lossy().into_owned()),
            },
        )
        .and(Err(Error::from(holst_core::Error::Login(
            LoginError::NoSavedLogin,
        ))
        .shown())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MissingLogin {
    logged_in: bool,
    file: Option<String>,
}

impl fmt::Display for MissingLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "not logged in. Run `{LOGIN}`")?;
        match &self.file {
            Some(path) => write!(f, "login file: {path}"),
            None => write!(f, "no login file: the home directory was not found"),
        }
    }
}

async fn saved_status(
    args: &StatusArgs,
    run: &RunSettings,
    format: OutputFormat,
    session: holst_core::domain::session::Session,
    kind: SessionStoreKind,
) -> Result<()> {
    let (store, cookies) = (kind.as_str(), session.cookies().len());
    let token = session.redacted();
    let client = run.client(Some(session))?;
    let checked = auth::status(&client, args).await;
    output::report(format, &SavedLogin::of(store, token, cookies, &checked))?;
    // A cookie may have been rotated during the check: keep it where it was.
    let saved = save_rotated(&client, kind, async |rotated, saved_in| {
        run.resave(rotated, saved_in).await
    })
    .await;
    warn_unsaved(saved);
    // A failed check fails the command after its report, which is all it says.
    match checked {
        StatusReport::Active(_) => Ok(()),
        StatusReport::Revoked(err) | StatusReport::Unknown(err) => {
            Err(Error::from(err).shown())
        }
    }
}

/// A workspace with its counts; `None` when its tree could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ListedWorkspace {
    id: String,
    name: String,
    boards: Option<usize>,
    folders: Option<usize>,
}

impl ListedWorkspace {
    fn of(status: &WorkspaceStatus) -> Self {
        let counts = status.counts.as_ref().ok().copied();
        Self {
            id: status.workspace.id.clone(),
            name: status.workspace.name.clone(),
            boards: counts.map(|NodeCounts { boards, .. }| boards),
            folders: counts.map(|NodeCounts { folders, .. }| folders),
        }
    }

    fn counted(&self) -> String {
        match (self.boards, self.folders) {
            (Some(boards), Some(folders)) => {
                format!(
                    "{}, {}",
                    counted(boards, "board"),
                    counted(folders, "folder")
                )
            }
            (None, _) | (_, None) => String::from("could not be counted"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    Active,
    Revoked,
    Unknown,
}

/// The JSON of `auth status -o json`; `sessions` and `tokens` only on request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SavedLogin {
    logged_in: bool,
    #[serde(rename = "source")]
    store: &'static str,
    cookies: usize,
    status: CheckStatus,
    name: Option<String>,
    workspaces: Vec<ListedWorkspace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sessions: Option<Vec<Session>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sessions_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<Vec<Token>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens_error: Option<String>,
    error: Option<String>,
    hint: Option<String>,
    #[serde(skip)]
    token: String,
}

impl SavedLogin {
    fn of(
        store: &'static str,
        token: String,
        cookies: usize,
        checked: &StatusReport,
    ) -> Self {
        let failed = |status, err: &holst_core::Error| Self {
            logged_in: true,
            store,
            cookies,
            status,
            name: None,
            workspaces: Vec::new(),
            sessions: None,
            sessions_error: None,
            tokens: None,
            tokens_error: None,
            error: Some(err.to_string()),
            hint: err.hint().map(|hint| hint.on(Surface::Cli).to_string()),
            token: token.clone(),
        };
        match checked {
            StatusReport::Active(AccountStatus {
                account,
                workspaces,
                sessions,
                tokens,
            }) => {
                let (sessions, sessions_error) = read(sessions.as_ref());
                let (tokens, tokens_error) = read(tokens.as_ref());
                Self {
                    logged_in: true,
                    store,
                    cookies,
                    status: CheckStatus::Active,
                    name: Some(display_name(account).to_owned()),
                    workspaces: workspaces.iter().map(ListedWorkspace::of).collect(),
                    sessions,
                    sessions_error,
                    tokens,
                    tokens_error,
                    error: None,
                    hint: None,
                    token,
                }
            }
            StatusReport::Revoked(err) => failed(CheckStatus::Revoked, err),
            StatusReport::Unknown(err) => failed(CheckStatus::Unknown, err),
        }
    }
}

/// A part asked for, split into its rows and the words of its failure.
fn read<T: Clone>(
    part: Option<&holst_core::Result<Vec<T>>>,
) -> (Option<Vec<T>>, Option<String>) {
    match part {
        Some(Ok(rows)) => (Some(rows.clone()), None),
        Some(Err(err)) => (None, Some(err.to_string())),
        None => (None, None),
    }
}

impl fmt::Display for SavedLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (store, token, cookies) = (self.store, &self.token, self.cookies);
        writeln!(
            f,
            "source:        {store}\nsession token: {token}\ncookies:       {cookies}"
        )?;
        let error = self.error.as_deref().unwrap_or_default();
        match self.status {
            CheckStatus::Active => {
                let name = self.name.as_deref().unwrap_or_default();
                let count = self.workspaces.len();
                write!(
                    f,
                    "status:  active, logged in as {name}\nworkspaces: {count}"
                )?;
                self.workspaces.iter().try_for_each(|workspace| {
                    write!(
                        f,
                        "\n  {} ({}): {}",
                        workspace.name,
                        workspace.id,
                        workspace.counted()
                    )
                })?;
                write_part(
                    f,
                    "sessions",
                    self.sessions.as_ref(),
                    self.sessions_error.as_ref(),
                    Session::line,
                )?;
                write_part(
                    f,
                    "tokens",
                    self.tokens.as_ref(),
                    self.tokens_error.as_ref(),
                    Token::line,
                )
            }
            CheckStatus::Revoked => {
                write!(f, "status:  revoked ({error}). Run `{LOGIN}`")
            }
            CheckStatus::Unknown => write!(f, "status:  could not check: {error}"),
        }
    }
}

fn write_part<T>(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    rows: Option<&Vec<T>>,
    error: Option<&String>,
    line: fn(&T) -> String,
) -> fmt::Result {
    match (rows, error) {
        (Some(rows), _) => {
            write!(f, "\n{label}: {}", rows.len())?;
            rows.iter()
                .try_for_each(|row| write!(f, "\n  {}", line(row)))
        }
        (None, Some(error)) => write!(f, "\n{label}: could not be listed: {error}"),
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::auth::{AccountStatus, NodeCounts, StatusReport, WorkspaceStatus},
        domain::{
            access::{Session, Token},
            account::Account,
        },
        error::{hint::LOGIN, login::LoginError},
    };
    use serde_json::json;

    use crate::commands::auth::status::{MissingLogin, SavedLogin};

    fn active(
        sessions: Option<holst_core::Result<Vec<Session>>>,
        tokens: Option<holst_core::Result<Vec<Token>>>,
    ) -> holst_core::Result<StatusReport> {
        let me = json!({ "id": "u1", "name": "Ann", "workspaces": [{ "id": "w1", "name": "easy" }] });
        let account = Account::of(&me)?;
        let workspaces = account
            .workspaces
            .iter()
            .cloned()
            .map(|workspace| WorkspaceStatus {
                workspace,
                counts: Ok(NodeCounts {
                    boards: 2,
                    folders: 1,
                }),
            })
            .collect();
        Ok(StatusReport::Active(AccountStatus {
            account,
            workspaces,
            sessions,
            tokens,
        }))
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn the_status_names_whose_session_it_is() -> holst_core::Result<()> {
        let report =
            SavedLogin::of("file", "(12 characters)".into(), 2, &active(None, None)?);
        assert_eq!(
            report.to_string(),
            "source:        file\nsession token: (12 characters)\ncookies:       2\nstatus:  active, logged in as Ann\nworkspaces: 1\n  easy (w1): 2 boards, 1 folder"
        );
        assert_eq!(
            serde_json::to_value(&report)
                .ok()
                .and_then(|json| json.get("status").cloned()),
            Some(json!("active"))
        );
        let refused =
            StatusReport::Revoked(holst_core::Error::Login(LoginError::SessionRevoked));
        let rejected = SavedLogin::of("file", String::new(), 0, &refused);
        assert!(
            rejected.to_string().ends_with(&format!("Run `{LOGIN}`")),
            "{rejected}"
        );
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn status_json_has_every_field_in_every_state() -> holst_core::Result<()> {
        let refused = holst_core::Error::Login(LoginError::SessionRevoked);
        let down =
            holst_core::Error::Http(holst_core::error::http::HttpError::Status {
                method: "GET",
                path: "/gapi/current_user".into(),
                status: 503,
                detail: String::new(),
            });
        let shape = |checked: &StatusReport| {
            serde_json::to_value(SavedLogin::of("file", String::new(), 2, checked)).ok()
        };
        let (refused_text, down_text) = (refused.to_string(), down.to_string());
        assert_eq!(
            shape(&active(None, None)?),
            Some(json!({
                "logged_in": true, "source": "file", "cookies": 2, "status": "active",
                "name": "Ann",
                "workspaces": [{ "id": "w1", "name": "easy", "boards": 2, "folders": 1 }],
                "error": null, "hint": null
            }))
        );
        assert_eq!(
            shape(&StatusReport::Revoked(refused)),
            Some(json!({
                "logged_in": true, "source": "file", "cookies": 2, "status": "revoked",
                "name": null, "workspaces": [], "error": refused_text,
                "hint": "run `holst auth login` to log in again"
            }))
        );
        assert_eq!(
            shape(&StatusReport::Unknown(down)),
            Some(json!({
                "logged_in": true, "source": "file", "cookies": 2, "status": "unknown",
                "name": null, "workspaces": [], "error": down_text,
                "hint": "try again in a minute"
            }))
        );
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn sessions_and_tokens_are_listed_as_lines_and_as_typed_json()
    -> holst_core::Result<()> {
        let sessions = vec![Session {
            id: "s1".into(),
            device: Some("holst-rs".into()),
            current: true,
            ..Session::default()
        }];
        let tokens = vec![Token {
            id: "t1".into(),
            name: Some("ci".into()),
            ..Token::default()
        }];
        let report = SavedLogin::of(
            "file",
            String::new(),
            2,
            &active(Some(Ok(sessions)), Some(Ok(tokens)))?,
        );
        assert!(
            report
                .to_string()
                .ends_with("\nsessions: 1\n  holst-rs (this session)\ntokens: 1\n  ci"),
            "{report}"
        );
        let json = serde_json::to_value(&report).ok();
        assert_eq!(
            json.as_ref().and_then(|json| json.get("sessions")),
            Some(&json!([{ "id": "s1", "device": "holst-rs", "current": true }]))
        );
        assert_eq!(
            json.as_ref().and_then(|json| json.get("tokens")),
            Some(&json!([{ "id": "t1", "name": "ci", "scopes": [] }]))
        );
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
    fn an_empty_list_says_none_and_a_failed_one_says_why() -> holst_core::Result<()> {
        let down = holst_core::Error::Login(LoginError::SessionRevoked);
        let report = SavedLogin::of(
            "file",
            String::new(),
            2,
            &active(Some(Ok(Vec::new())), Some(Err(down)))?,
        );
        assert!(
            report.to_string().ends_with(&format!(
                "\nsessions: 0\ntokens: could not be listed: {}",
                LoginError::SessionRevoked
            )),
            "{report}"
        );
        Ok(())
    }

    #[test]
    fn no_login_says_what_to_run() {
        let none = MissingLogin {
            logged_in: false,
            file: Some("/h/session".into()),
        };
        assert_eq!(
            none.to_string(),
            "not logged in. Run `holst auth login`\nlogin file: /h/session"
        );
    }
}
