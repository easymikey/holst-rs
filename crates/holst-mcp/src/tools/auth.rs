//! The `auth` tool: the saved login, the account and its workspaces.

use holst_core::{
    command::auth::{
        AccountStatus,
        AuthVerb,
        StatusArgs,
        StatusReport,
        WorkspaceStatus,
        status,
    },
    domain::text::counted,
    error::hint::Surface,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::HandedClient,
    reply,
    server::McpServer,
};

impl McpServer {
    pub(super) async fn auth(
        &self,
        verb: AuthVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            // `login` and `logout` are for the CLI; the table gives them no
            // tool.
            AuthVerb::Login => self.fail(&Error::CliOnly {
                command: "auth login".to_owned(),
            }),
            AuthVerb::Logout => self.fail(&Error::CliOnly {
                command: "auth logout".to_owned(),
            }),
            AuthVerb::Status => {
                self.parsed(arguments, async |args: StatusArgs| {
                    self.auth_status(&args).await
                })
                .await
            }
        }
    }

    async fn auth_status(&self, args: &StatusArgs) -> CallToolResult {
        match self.store_kind().await {
            Err(err) => self.fail(&err),
            Ok(None) => reply::text(
                "no login: ask the user to run `holst-mcp setup` in a terminal, then call auth again",
            ),
            Ok(Some(source)) => {
                self.run(async |client| {
                    self.status_reply(&client, source.as_str(), args).await
                })
                .await
            }
        }
    }

    async fn status_reply(
        &self,
        client: &HandedClient,
        source: &str,
        args: &StatusArgs,
    ) -> Result<String> {
        match status(client, args).await {
            StatusReport::Active(found) => Ok(self.status_text(source, found)),
            // The words and the hint of the error, as every other tool says
            // them.
            StatusReport::Revoked(err) => {
                Ok(format!("{}\n", reply::explain(&Error::from(err))))
            }
            StatusReport::Unknown(err) => Err(err.into()),
        }
    }

    fn status_text(&self, source: &str, found: AccountStatus) -> String {
        let account = format!(
            "logged in: {} {} · login from: {source}\n",
            found.account.name.as_deref().unwrap_or("?"),
            self.state.alias(&found.account.id),
        );
        let workspaces: String = found
            .workspaces
            .iter()
            .map(|each| self.workspace_status(each))
            .collect();
        let tokens = match found.tokens {
            None => String::new(),
            Some(Ok(list)) => {
                format!("tokens:\n{}", self.state.render(list))
            }
            Some(Err(err)) => {
                format!("tokens: not read: {}\n", err.on(Surface::Mcp))
            }
        };
        account + &workspaces + &tokens
    }

    fn workspace_status(&self, each: &WorkspaceStatus) -> String {
        let name = format!(
            "{} {}",
            each.workspace.name,
            self.state.alias(&each.workspace.id)
        );
        match &each.counts {
            Ok(counts) => format!(
                "{name} · {}, {}\n",
                counted(counts.boards, "board"),
                counted(counts.folders, "folder")
            ),
            Err(err) => format!("{name} · not counted: {}\n", err.on(Surface::Mcp)),
        }
    }
}
