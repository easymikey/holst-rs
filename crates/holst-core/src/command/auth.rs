//! The `auth` group: log in, log out, and the status of the saved login.

use holst_macros::CommandArgs;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    command::{
        account::{self, SessionCheck},
        lookup,
        table::{ArgumentValue, Command, CommandArgs, EnumArgument, ValueKind},
    },
    domain::{
        account::{Account, Workspace},
        login::Email,
        node::NodeKind,
        tree::name_index::build::NameIndex,
    },
    error::{Error, Result},
    remote::{api, client::Client},
};

/// The verbs of `holst auth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum AuthVerb {
    Login,
    Logout,
    Status,
}

impl AuthVerb {
    pub(crate) const fn command(self) -> &'static Command {
        match self {
            AuthVerb::Login => &LoginArgs::COMMAND,
            AuthVerb::Logout => &LogoutArgs::COMMAND,
            AuthVerb::Status => &StatusArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Auth,
    verb = "login",
    risk = Write,
    surfaces = [Cli],
    local_files,
    about = "Log in and save the login",
    help = "Log in with your email address and a login code or password, and save the login.\n\
            It prints the account and its workspaces. The login goes to the keychain, or to --home.",
    example = "holst auth login",
    example = "holst auth login --email you@example.com",
    example = "holst auth login --no-save"
)]
pub struct LoginArgs {
    #[argument(
        value_name = "address",
        help = "The email address to log in with; asked for when missing",
        surfaces = [Cli]
    )]
    pub email: Option<Email>,
    #[argument(
        help = "Only check that the login works; do not save the login",
        surfaces = [Cli]
    )]
    pub no_save: SaveLogin,
}

/// Whether a login is saved once it works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "bool")]
pub enum SaveLogin {
    Keep,
    Skip,
}

impl From<bool> for SaveLogin {
    fn from(no_save: bool) -> Self {
        if no_save {
            SaveLogin::Skip
        } else {
            SaveLogin::Keep
        }
    }
}

impl ArgumentValue for SaveLogin {
    const KIND: ValueKind = ValueKind::Switch;
    const REQUIRED: bool = false;
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Auth,
    verb = "logout",
    risk = Write,
    surfaces = [Cli],
    no_dry_run,
    about = "End the server session and forget the saved login",
    help = "End the server session and forget the saved login on this machine.\n\
            The board cache of the account is removed too.",
    example = "holst auth logout",
    example = "holst auth logout --local-only"
)]
pub struct LogoutArgs {
    #[argument(
        help = "Only forget the saved login here; do not end the server session"
    )]
    pub local_only: LogoutMode,
}

/// Whether a logout ends the server session or only forgets the login here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "bool")]
pub enum LogoutMode {
    Everywhere,
    LocalOnly,
}

impl From<bool> for LogoutMode {
    fn from(local_only: bool) -> Self {
        if local_only {
            LogoutMode::LocalOnly
        } else {
            LogoutMode::Everywhere
        }
    }
}

impl ArgumentValue for LogoutMode {
    const KIND: ValueKind = ValueKind::Switch;
    const REQUIRED: bool = false;
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Auth,
    verb = "status",
    risk = Read,
    about = "Show the saved login, account and workspaces",
    help = "Show the saved login, the account and each workspace with its board and folder count.\n\
            Counts come from one request per workspace; --with adds the tokens.",
    example = "holst auth status",
    example = "holst auth status --with tokens -o json"
)]
pub struct StatusArgs {
    #[argument(
        value_name = "parts",
        help = "Also show these parts, comma separated: tokens; none by default"
    )]
    pub with: Vec<StatusPart>,
}

/// A part `auth status` adds to its report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StatusPart {
    Tokens,
}

impl EnumArgument for StatusPart {}

/// What `auth status` found out about the client's login.
#[derive(Debug)]
pub enum StatusReport {
    Active(AccountStatus),
    /// The server refuses the login (revoked, expired, or no access).
    Revoked(Error),
    /// The server could not be asked or answered something else.
    Unknown(Error),
}

/// The account of an accepted login, with the parts asked for.
#[derive(Debug)]
pub struct AccountStatus {
    pub account: Account,
    pub workspaces: Vec<WorkspaceStatus>,
    /// `None` unless asked for with [`StatusPart::Tokens`].
    pub tokens: Option<Result<Value>>,
}

/// A workspace with what its tree holds; a tree that failed fails alone.
#[derive(Debug)]
pub struct WorkspaceStatus {
    pub workspace: Workspace,
    pub counts: Result<NodeCounts>,
}

/// How many live boards and folders a tree holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeCounts {
    pub boards: usize,
    pub folders: usize,
}

impl NodeCounts {
    fn of(tree: &NameIndex) -> Self {
        tree.nodes().filter(|node| !tree.in_trash(*node)).fold(
            Self {
                boards: 0,
                folders: 0,
            },
            |counts, node| match tree.kind(node) {
                NodeKind::Board => Self {
                    boards: counts.boards + 1,
                    ..counts
                },
                NodeKind::Folder => Self {
                    folders: counts.folders + 1,
                    ..counts
                },
                NodeKind::Other => counts,
            },
        )
    }
}

/// Asks the server whether the login works and, when it does, counts each
/// workspace's boards and folders (one tree request per workspace, in
/// parallel).
pub async fn status(client: &Client, status: &StatusArgs) -> StatusReport {
    match account::check(client).await {
        SessionCheck::Active(account) => {
            StatusReport::Active(account_status(client, status, account).await)
        }
        SessionCheck::Revoked(error) => StatusReport::Revoked(error),
        SessionCheck::Unknown(error) => StatusReport::Unknown(error),
    }
}

async fn account_status(
    client: &Client,
    status: &StatusArgs,
    account: Account,
) -> AccountStatus {
    let workspaces = lookup::trees(client, account.workspaces.clone())
        .await
        .into_iter()
        .map(|(workspace, tree)| WorkspaceStatus {
            workspace,
            counts: tree.map(|tree| NodeCounts::of(&tree)),
        })
        .collect();
    let tokens = if status.with.contains(&StatusPart::Tokens) {
        Some(api::user::pat_list(client).await)
    } else {
        None
    };
    AccountStatus {
        account,
        workspaces,
        tokens,
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;

    use crate::{
        Result,
        command::auth::{NodeCounts, StatusArgs, StatusPart, StatusReport, status},
        remote::client::fake::{on_fake, reply, stored},
    };

    const TREE_PATH: &str = "/tree";
    const TOKENS_PATH: &str = "/gapi/personal_access_token/list";

    fn id(ordinal: u32) -> String {
        format!("00000001-0000-4000-8000-{ordinal:012x}")
    }

    fn account() -> String {
        json!({
            "id": "u1",
            "name": "Ann",
            "workspaces": [{ "id": "w1", "name": "easy" }, { "id": "w2", "name": "team" }],
        })
        .to_string()
    }

    fn trees() -> [String; 2] {
        [
            json!({ "channels": [
                { "id": id(1), "type": "folder", "name": "Plans" },
                { "id": id(2), "type": "board", "name": "Roadmap", "parent_id": id(1) },
                { "id": id(3), "type": "board", "name": "Retro" },
                { "id": id(4), "type": "board", "name": "Old", "in_trash": true },
            ]})
            .to_string(),
            json!({ "channels": [] }).to_string(),
        ]
    }

    /// The workspaces' names with their counts, `None` for a failed tree.
    fn counted(report: &StatusReport) -> Vec<(String, Option<NodeCounts>)> {
        match report {
            StatusReport::Active(active) => active
                .workspaces
                .iter()
                .map(|each| {
                    (
                        each.workspace.name.clone(),
                        each.counts.as_ref().ok().copied(),
                    )
                })
                .collect(),
            StatusReport::Revoked(_) | StatusReport::Unknown(_) => Vec::new(),
        }
    }

    fn has_tokens(report: &StatusReport) -> bool {
        match report {
            StatusReport::Active(active) => {
                active.tokens.as_ref().is_some_and(Result::is_ok)
            }
            StatusReport::Revoked(_) | StatusReport::Unknown(_) => false,
        }
    }

    async fn run(
        with: Vec<StatusPart>,
        extra: &[&str],
    ) -> Result<(StatusReport, Vec<String>)> {
        let [first, second] = trees();
        let replies = [account(), first, second]
            .into_iter()
            .chain(extra.iter().map(ToString::to_string))
            .map(|body| reply(200, &[("content-type", "application/json")], &body));
        let (fake, client) = on_fake(replies, Some(stored("tok", "s=1")))?;
        let report = status(&client, &StatusArgs { with }).await;
        let paths = fake
            .requests()
            .iter()
            .map(|request| request.url.path().to_owned())
            .collect();
        Ok((report, paths))
    }

    #[tokio::test]
    async fn status_counts_the_boards_and_folders_of_each_workspace() -> Result<()> {
        let (report, paths) = run(Vec::new(), &[]).await?;
        assert_eq!(
            counted(&report),
            [
                (
                    "easy".to_owned(),
                    Some(NodeCounts {
                        boards: 2,
                        folders: 1
                    })
                ),
                (
                    "team".to_owned(),
                    Some(NodeCounts {
                        boards: 0,
                        folders: 0
                    })
                ),
            ]
        );
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.ends_with(TREE_PATH))
                .count(),
            2
        );
        Ok(())
    }

    #[tokio::test]
    async fn status_asks_for_tokens_only_with_tokens() -> Result<()> {
        let (without, plain) = run(Vec::new(), &[]).await?;
        let (with, asked) = run(vec![StatusPart::Tokens], &["[]"]).await?;
        assert_eq!(
            (
                has_tokens(&without),
                plain.iter().any(|path| path == TOKENS_PATH)
            ),
            (false, false)
        );
        assert_eq!(
            (
                has_tokens(&with),
                asked.iter().any(|path| path == TOKENS_PATH)
            ),
            (true, true)
        );
        Ok(())
    }
}
