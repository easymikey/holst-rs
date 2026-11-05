//! The `workspace` group on either surface: list, show, create, rename, log.

use serde_json::{Value, json};

use crate::{
    error::ScenarioError,
    holst::{On, SendMode},
    observation::Observation,
    then::Then,
};

/// A workspace command that needs a login, for a scenario of all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCommand {
    Show,
    Create,
    Rename,
    Log,
}

impl On<'_> {
    /// Lists the workspaces with the details this surface shows for each.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_workspace_details(&self) -> Result<Observation, ScenarioError> {
        self.reading(&["workspace", "ls"], "workspace", json!({ "action": "ls" }))
            .await
    }

    /// Lists the workspaces the user may join and does not belong to.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_joinable_workspaces(&self) -> Result<Observation, ScenarioError> {
        self.reading(
            &["workspace", "ls", "--joinable"],
            "workspace",
            json!({ "action": "ls", "joinable": true }),
        )
        .await
    }

    /// Shows a workspace with its basics; `None` leaves the choice to the
    /// surface.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn show_workspace(
        &self,
        workspace: Option<&str>,
    ) -> Result<Observation, ScenarioError> {
        self.show_workspace_parts(workspace, &[], None).await
    }

    /// Shows a workspace with the `parts` asked for, of the member `user` alone
    /// when given.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn show_workspace_parts(
        &self,
        workspace: Option<&str>,
        parts: &[&str],
        user: Option<&str>,
    ) -> Result<Observation, ScenarioError> {
        let joined = parts.join(",");
        let line: Vec<&str> = ["workspace", "show"]
            .into_iter()
            .chain(workspace)
            .chain((!parts.is_empty()).then_some("--with"))
            .chain((!parts.is_empty()).then_some(joined.as_str()))
            .chain(user.map(|_| "--user"))
            .chain(user)
            .collect();
        let given = [
            Some(("action", json!("show"))),
            workspace.map(|workspace| ("workspace", json!(workspace))),
            (!parts.is_empty()).then(|| ("with", json!(parts))),
            user.map(|user| ("user", json!(user))),
        ];
        let arguments: serde_json::Map<String, Value> = given
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        self.reading(&line, "workspace", Value::Object(arguments))
            .await
    }

    /// Creates a workspace called `name`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn create_workspace(
        &self,
        name: &str,
    ) -> Result<Observation, ScenarioError> {
        let dry = self.mode == SendMode::DryRun;
        let line: Vec<&str> = ["workspace", "create", name]
            .into_iter()
            .chain(dry.then_some("-n"))
            .collect();
        let arguments = json!({ "action": "create", "name": name, "dry_run": dry });
        self.changing(&line, "workspace", arguments).await
    }

    /// Renames the workspace `workspace` to `name` for all its members.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn rename_workspace(
        &self,
        workspace: &str,
        name: &str,
    ) -> Result<Observation, ScenarioError> {
        let line: Vec<&str> = ["workspace", "rename", workspace, name]
            .into_iter()
            .chain((self.mode == SendMode::Confirmed).then_some("-y"))
            .chain((self.mode == SendMode::DryRun).then_some("-n"))
            .collect();
        let arguments = self.admin_arguments(json!({
            "action": "rename",
            "workspace": workspace,
            "name": name,
        }));
        self.changing(&line, "workspace", arguments).await
    }

    /// Reads the event log, filtered by `KEY=VALUE` queries.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn workspace_log(
        &self,
        queries: &[(&str, &str)],
    ) -> Result<Observation, ScenarioError> {
        let pairs: Vec<String> = queries
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        let line: Vec<&str> = ["workspace", "log"]
            .into_iter()
            .chain(pairs.iter().flat_map(|pair| ["-p", pair.as_str()]))
            .collect();
        let query: serde_json::Map<String, Value> = queries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), json!(value)))
            .collect();
        self.reading(
            &line,
            "workspace",
            json!({ "action": "log", "param": query }),
        )
        .await
    }

    /// Reads the page of the event log that follows the one `previous` showed.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn workspace_log_after(
        &self,
        previous: &Observation,
        queries: &[(&str, &str)],
    ) -> Result<Observation, ScenarioError> {
        let cursor = previous.shown().and_then(cursor_of).unwrap_or_default();
        let queries: Vec<(&str, &str)> = queries
            .iter()
            .copied()
            .chain([("cursor", cursor.as_str())])
            .collect();
        self.workspace_log(&queries).await
    }

    /// Runs `command` on a workspace named Team.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn run_workspace_command(
        &self,
        command: WorkspaceCommand,
    ) -> Result<Observation, ScenarioError> {
        match command {
            WorkspaceCommand::Show => self.show_workspace(Some("Team")).await,
            WorkspaceCommand::Create => self.create_workspace("Studio").await,
            WorkspaceCommand::Rename => self.rename_workspace("Team", "Crew").await,
            WorkspaceCommand::Log => self.workspace_log(&[]).await,
        }
    }
}

/// The cursor a log page ends with: a field of the JSON, a line of the text.
fn cursor_of(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|reply| reply.get("cursor")?.as_str().map(str::to_owned))
        .or_else(|| {
            text.lines()
                .find_map(|line| line.split_once("; next: cursor "))
                .map(|(_, cursor)| cursor.to_owned())
        })
}

impl Then<'_> {
    /// Expects the surface not to have shown `words`.
    ///
    /// # Panics
    /// When it showed them.
    pub fn does_not_show(&self, words: &str) {
        assert!(
            matches!(self.observation, Observation::Shown(text) if !text.contains(words)),
            "expected a reply without {words:?}, got {:?}",
            self.observation
        );
    }

    /// Expects the surface to have shown each of `words`, in this order.
    ///
    /// # Panics
    /// When one is missing or out of order.
    pub fn shows_in_order(&self, words: &[&str]) {
        let ordered = self.observation.shown().is_some_and(|text| {
            words
                .iter()
                .try_fold(0, |from, word| {
                    text.get(from..)?
                        .find(word)
                        .map(|at| from + at + word.len())
                })
                .is_some()
        });
        assert!(
            ordered,
            "expected {words:?} in this order, got {:?}",
            self.observation
        );
    }

    /// Expects a log page that says where the rest begins.
    ///
    /// # Panics
    /// When there is no cursor.
    pub fn shows_a_cursor(&self) {
        assert!(
            self.observation.shown().and_then(cursor_of).is_some(),
            "expected a cursor, got {:?}",
            self.observation
        );
    }

    /// Expects a log page that is the last.
    ///
    /// # Panics
    /// When there is a cursor.
    pub fn shows_no_cursor(&self) {
        assert!(
            self.observation
                .shown()
                .is_some_and(|text| cursor_of(text).is_none()),
            "expected no cursor, got {:?}",
            self.observation
        );
    }

    /// Expects a workspace with this many members.
    ///
    /// # Panics
    /// When the count differs.
    pub fn shows_members(&self, count: u64) {
        let shown = self.observation.shown().is_some_and(|text| {
            serde_json::from_str::<Value>(text).map_or_else(
                |_| text.contains(&format!("{count} members")),
                |reply| reply.get("member_count") == Some(&json!(count)),
            )
        });
        assert!(
            shown,
            "expected {count} members, got {:?}",
            self.observation
        );
    }
}
