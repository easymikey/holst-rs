//! The `workspace` tool: list, details, event log, create, rename.

use holst_core::{
    command::workspace::{
        EventLog,
        ListArgs,
        LogArgs,
        PartRead,
        ShowArgs,
        WorkspaceBrief,
        WorkspaceChange,
        WorkspaceRow,
        WorkspaceVerb,
        create,
        list,
        log,
        rename,
        show,
    },
    domain::text::{EMPTY, counted, printable},
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value, json};

use crate::{error::Result, server::McpServer};

impl McpServer {
    pub(super) async fn workspace_command(
        &self,
        verb: WorkspaceVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self.expanded_references(arguments) {
            Ok(arguments) => self.run_workspace_verb(verb, arguments).await,
            Err(err) => self.fail(&err),
        }
    }

    async fn run_workspace_verb(
        &self,
        verb: WorkspaceVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            WorkspaceVerb::Ls => {
                self.parsed(arguments, async |args: ListArgs| {
                    self.run(async |client| {
                        Ok(self.listing_text(&list(&client, &args).await?))
                    })
                    .await
                })
                .await
            }
            WorkspaceVerb::Show => {
                self.parsed(arguments, async |args: ShowArgs| {
                    self.run(async |client| {
                        Ok(self.shown_text(&show(&client, &args).await?))
                    })
                    .await
                })
                .await
            }
            WorkspaceVerb::Create => {
                self.change(arguments, create, Self::workspace_changed_text)
                    .await
            }
            WorkspaceVerb::Rename => {
                self.change(arguments, rename, Self::workspace_changed_text)
                    .await
            }
            WorkspaceVerb::Log => {
                self.parsed(arguments, async |args: LogArgs| {
                    self.run(async |client| {
                        Ok(self.log_text(&log(&client, &args).await?))
                    })
                    .await
                })
                .await
            }
        }
    }

    /// `@N` in the workspace, board, folder, template, category, user, object
    /// and cursor the model gives,
    /// as the other tools do.
    pub(super) fn expanded_references(
        &self,
        arguments: Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        arguments
            .into_iter()
            .map(|(key, value)| match (key.as_str(), value) {
                (
                    "workspace" | "board" | "folder" | "into" | "template" | "user"
                    | "cursor" | "category" | "object",
                    Value::String(text),
                ) => {
                    let reference = self.expand(text.trim())?;
                    Ok((key, Value::String(reference)))
                }
                ("board", Value::Array(texts)) => {
                    let ids = texts
                        .into_iter()
                        .map(|each| match each {
                            Value::String(text) => {
                                self.expand(text.trim()).map(Value::String)
                            }
                            Value::Null
                            | Value::Bool(_)
                            | Value::Number(_)
                            | Value::Array(_)
                            | Value::Object(_) => Ok(each),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok((key, Value::Array(ids)))
                }
                (_, value) => Ok((key, value)),
            })
            .collect()
    }

    fn workspace_changed_text(&self, change: &WorkspaceChange) -> String {
        change.phrase(|id| format!(" {}", self.state.alias(id))) + "\n"
    }

    fn listing_text(&self, rows: &[WorkspaceRow]) -> String {
        if rows.is_empty() {
            format!("{EMPTY}\n")
        } else {
            rows.iter()
                .map(|row| {
                    let details = [
                        row.organization.clone(),
                        row.role.clone(),
                        row.members.map(|count| counted(count, "member")),
                    ];
                    let name = format!(
                        "{} {}",
                        printable(&row.name),
                        self.state.alias(&row.id)
                    );
                    joined_by_dots(name, details) + "\n"
                })
                .collect()
        }
    }

    fn part_text(&self, label: &str, found: Value) -> String {
        let body = self.state.render(found);
        format!("{label}:\n{}\n", body.trim_end())
    }

    fn shown_text(&self, shown: &WorkspaceBrief) -> String {
        let counts = [
            shown.member_count.map(|count| counted(count, "member")),
            shown.guest_count.map(|count| counted(count, "guest")),
        ];
        let name =
            format!("{} {}", printable(&shown.name), self.state.alias(&shown.id));
        let parts: String = shown
            .parts
            .iter()
            .map(|(part, fetched)| {
                let label = format!("{part:?}").to_lowercase();
                match fetched {
                    PartRead::Found(value) => self.part_text(&label, value.clone()),
                    PartRead::Members(members) => {
                        self.part_text(&label, json!(members))
                    }
                    PartRead::Requests(rows) => self.part_text(&label, json!(rows)),
                    PartRead::Plan(plan) => self.part_text(&label, json!(plan)),
                    PartRead::NeedsAdmin { error } => format!("{label}: {error}\n"),
                    PartRead::NotRead { error } => {
                        format!("{label}: not read: {error}\n")
                    }
                }
            })
            .collect();
        joined_by_dots(name, counts) + "\n" + &parts
    }

    fn log_text(&self, found: &EventLog) -> String {
        let more = found
            .cursor
            .as_deref()
            .map(|cursor| {
                format!("… {} shown; next: cursor {cursor}\n", found.events.len())
            })
            .unwrap_or_default();
        let events = if found.events.is_empty() {
            EMPTY.to_owned()
        } else {
            self.state
                .render(Value::Array(found.events.clone()))
                .trim_end()
                .to_owned()
        };
        format!("{events}\n{more}")
    }
}

/// `head · detail · detail`, the details that are there.
pub(super) fn joined_by_dots(
    head: String,
    details: impl IntoIterator<Item = Option<String>>,
) -> String {
    details
        .into_iter()
        .flatten()
        .fold(head, |line, detail| line + " · " + &detail)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use holst_core::command::workspace::{WorkspaceBrief, WorkspaceRow};

    use crate::server::McpServer;

    #[test]
    fn hostile_workspace_names_are_printable() {
        let holst = McpServer::offline();
        let row = WorkspaceRow {
            id: "w1".to_owned(),
            name: "Te\u{202e}am".to_owned(),
            organization: None,
            role: None,
            members: None,
        };
        let brief = WorkspaceBrief {
            id: "w1".to_owned(),
            name: "Te\u{1b}[2Jam".to_owned(),
            member_count: None,
            guest_count: None,
            parts: BTreeMap::new(),
        };
        assert_eq!(holst.listing_text(&[row]), "Te\u{fffd}am @1\n");
        assert_eq!(holst.shown_text(&brief), "Te\u{fffd}[2Jam @1\n");
    }
}
