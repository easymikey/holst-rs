//! The `auth` tool: the saved login, the account and its workspaces.

use holst_core::{
    command::{
        auth::{
            AccountStatus,
            AuthVerb,
            StatusArgs,
            StatusReport,
            WorkspaceStatus,
            status,
        },
        table::parse,
    },
    error::hint::Surface,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::{HandedClient, page},
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
            AuthVerb::Login => self.auth_login().await,
            // `logout` is for the CLI; the table gives it no tool.
            AuthVerb::Logout => self.fail(&Error::CliOnly {
                command: "auth logout".to_owned(),
            }),
            AuthVerb::Status => match parse::<StatusArgs>(arguments) {
                Ok(args) => self.auth_status(&args).await,
                Err(err) => self.fail(&err.into()),
            },
        }
    }

    /// A link to a page on this computer where the user logs in.
    async fn auth_login(&self) -> CallToolResult {
        match page::open(&self.login).await {
            Ok(link) => reply::text(format!(
                "{link}\nOpen it in a browser to log in, then call auth (action status).\n"
            )),
            Err(err) => self.fail(&err),
        }
    }

    async fn auth_status(&self, args: &StatusArgs) -> CallToolResult {
        match self.store_kind().await {
            Err(err) => self.fail(&err),
            Ok(None) => reply::text(
                "no login: call auth with action login, or the user runs `holst auth login` in a terminal",
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
            StatusReport::Active(found) => Ok(self.status_text(client, source, found)),
            // The words and the hint of the error, as every other tool says them.
            StatusReport::Revoked(err) => {
                Ok(format!("{}\n", reply::explain(&Error::from(err))))
            }
            StatusReport::Unknown(err) => Err(err.into()),
        }
    }

    fn status_text(
        &self,
        client: &HandedClient,
        source: &str,
        found: AccountStatus,
    ) -> String {
        drop(
            self.state
                .store_workspaces(found.account.workspaces.clone(), client.generation),
        );
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
                "{name} · {} boards, {} folders\n",
                counts.boards, counts.folders
            ),
            Err(err) => format!("{name} · not counted: {}\n", err.on(Surface::Mcp)),
        }
    }
}
