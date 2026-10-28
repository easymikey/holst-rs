//! The login and the session: what a terminal does, what each surface says.

use std::process::Output;

use serde::Deserialize;
use serde_json::{Map, json};

use crate::{
    cli::{failure, output},
    error::ScenarioError,
    holst::{Holst, On},
    mcp,
    observation::{CountedWorkspace, Observation},
    surface::Surface,
    then::Then,
};

/// What the MCP text puts between the entries of a listing.
pub(crate) const LISTING_SEPARATOR: &str = " · ";

/// What `holst auth status -o json` prints, in every state.
#[derive(Deserialize)]
struct CliStatus {
    status: Option<String>,
    name: Option<String>,
    #[serde(default)]
    workspaces: Vec<CliWorkspace>,
    error: Option<String>,
    hint: Option<String>,
    logged_in: bool,
}

#[derive(Deserialize)]
struct CliWorkspace {
    name: String,
    boards: Option<usize>,
    folders: Option<usize>,
}

impl Holst {
    /// Logs in with the emailed `code`, in a terminal: only the CLI signs in.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn log_in(
        &self,
        email: &str,
        code: &str,
    ) -> Result<Observation, ScenarioError> {
        self.terminal(&["auth", "login", "--email", email], &format!("{code}\n"))
            .await
    }

    /// Logs out, in a terminal: only the CLI signs out.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn log_out(&self) -> Result<Observation, ScenarioError> {
        self.terminal(&["auth", "logout"], "").await
    }

    async fn terminal(
        &self,
        arguments: &[&str],
        input: &str,
    ) -> Result<Observation, ScenarioError> {
        let output = output(self.command("holst")?, arguments, input).await?;
        self.checked(failure(&output).unwrap_or(Observation::Done))
    }
}

impl On<'_> {
    /// Asks this surface for the login, with the access tokens of the account.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn status_with_tokens(&self) -> Result<Observation, ScenarioError> {
        self.reading(
            &["auth", "status", "--with", "tokens"],
            "auth",
            json!({ "with": ["tokens"] }),
        )
        .await
    }

    /// Asks this surface whether the saved login works.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn status(&self) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                let output =
                    output(self.holst.command("holst")?, ["auth", "status"], "")
                        .await?;
                cli_status(&output)
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "auth",
                    Map::new(),
                    mcp_status,
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

impl Then<'_> {
    /// Expects the login of `name`, reaching exactly these workspaces.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_login_of(&self, name: &str, workspaces: &[&str]) {
        let found = self.observation.login().map(|(found, listed)| {
            let names: Vec<_> = listed
                .iter()
                .map(|workspace| workspace.name.as_str())
                .collect();
            (found, names)
        });
        assert_eq!(found, Some((name, workspaces.to_vec())));
    }

    /// Expects each `(workspace, boards, folders)` to be counted as given.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_counts(&self, expected: &[(&str, usize, usize)]) {
        let wanted: Vec<_> = expected
            .iter()
            .map(|&(name, boards, folders)| CountedWorkspace {
                name: name.to_owned(),
                boards: Some(boards),
                folders: Some(folders),
            })
            .collect();
        let found = self.observation.login().map(|(_, listed)| listed);
        assert_eq!(found, Some(wanted.as_slice()));
    }

    /// Expects to be told there is no login.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_no_login(&self) {
        assert_eq!(self.observation, &Observation::NotLoggedIn);
    }

    /// Expects the rejection in words: no status number, no request path.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn says_the_login_was_rejected(&self) {
        assert!(
            matches!(self.observation, Observation::LoginNotAccepted { message }
                if message.contains("the saved login was revoked")
                    && message.contains("log in again")
                    && !message.contains("HTTP")
                    && !message.contains("/gapi")),
            "expected the rejection in words, got {:?}",
            self.observation
        );
    }
}

/// The report is on stdout even when the check fails.
fn cli_status(output: &Output) -> Observation {
    match serde_json::from_slice::<CliStatus>(&output.stdout).ok() {
        Some(CliStatus {
            logged_in: false, ..
        }) => Observation::NotLoggedIn,
        Some(CliStatus {
            status: Some(status),
            name,
            workspaces,
            error,
            hint,
            ..
        }) => match status.as_str() {
            "active" => Observation::LoggedIn {
                name: name.unwrap_or_default(),
                workspaces: workspaces
                    .into_iter()
                    .map(|workspace| CountedWorkspace {
                        name: workspace.name,
                        boards: workspace.boards,
                        folders: workspace.folders,
                    })
                    .collect(),
            },
            "revoked" => Observation::LoginNotAccepted {
                message: message(error, hint),
            },
            _ => failure(output).unwrap_or(Observation::Done),
        },
        Some(_) | None => failure(output).unwrap_or(Observation::Done),
    }
}

fn message(error: Option<String>, hint: Option<String>) -> String {
    [error, hint]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("; ")
}

/// `logged in: NAME @N · login from: SOURCE`, then a line per workspace,
/// `NAME @N · B boards, F folders`, then `tokens:` when asked for; or the
/// words of the refusal; a tool error never gets here (see `mcp::call`).
fn mcp_status(reply: &mcp::Reply) -> Observation {
    let text = reply.text.clone();
    let name = |head: &str| {
        head.rsplit_once(' ')
            .map_or(head, |(name, _alias)| name)
            .to_owned()
    };
    match text.strip_prefix("logged in: ") {
        Some(rest) => {
            let mut lines = rest.lines();
            Observation::LoggedIn {
                name: lines
                    .next()
                    .and_then(|first| first.split_once(LISTING_SEPARATOR))
                    .map(|(head, _)| name(head))
                    .unwrap_or_default(),
                workspaces: lines
                    .take_while(|line| !line.starts_with("tokens:"))
                    .filter_map(|line| line.split_once(LISTING_SEPARATOR))
                    .map(|(head, counted)| CountedWorkspace {
                        name: name(head),
                        boards: counted
                            .split_once(" boards")
                            .and_then(|(count, _)| count.parse().ok()),
                        folders: counted
                            .split_once(", ")
                            .and_then(|(_, folders)| folders.strip_suffix(" folders"))
                            .and_then(|count| count.parse().ok()),
                    })
                    .collect(),
            }
        }
        None if text.starts_with("no login") => Observation::NotLoggedIn,
        None => Observation::LoginNotAccepted { message: text },
    }
}
