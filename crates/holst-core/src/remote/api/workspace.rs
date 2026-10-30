use holst_macros::{get, patch, post};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::tree::TreeFallback,
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

fn logged<T>(parsed: T, fallback: Option<TreeFallback>) -> T {
    match fallback {
        Some(why) => {
            tracing::warn!(err = %why, "the tree differs from the shape expected; parsed heuristically");
            parsed
        }
        None => parsed,
    }
}

/// A workspace.
#[get("workspace/{workspace_id}", class = Read)]
pub async fn get(client: &Client, workspace_id: &str) -> Result<Value>;

/// The trash.
#[get("workspace/{workspace_id}/trash", class = Read)]
pub async fn trash(client: &Client, workspace_id: &str) -> Result<Value>;

/// When members were last active.
#[get("workspace_user/{workspace_id}/list_last_active_dates", class = Admin)]
pub async fn last_active_dates(client: &Client, workspace_id: &str) -> Result<Value>;

/// The requests to join that wait for an answer.
#[get("workspace/{workspace_id}/join_request", class = Admin, query(scope = "active"))]
pub async fn join_requests(client: &Client, workspace_id: &str) -> Result<Value>;

/// How many boards a user created in the workspace; `userId` is required.
#[get("workspace/{workspace_id}/created_channels_count", class = Read, query(userId = user_id))]
pub async fn created_channels_count(
    client: &Client,
    workspace_id: &str,
    user_id: &str,
) -> Result<Value>;

/// SSO settings.
#[get("workspace/{workspace_id}/sso_settings", class = Admin)]
pub async fn sso_settings(client: &Client, workspace_id: &str) -> Result<Value>;

/// Verified domains.
#[get("workspace_domains/{workspace_id}/list", class = Admin)]
pub async fn domains(client: &Client, workspace_id: &str) -> Result<Value>;

/// The body of `workspace/{id}` (rename) and `workspace/create`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceName {
    pub name: String,
}

/// Rename a workspace. Not verified live.
#[patch("workspace/{workspace_id}", class = Admin)]
pub async fn rename(
    client: &Client,
    workspace_id: &str,
    #[body] body: &WorkspaceName,
) -> Result<Value>;

/// Create a workspace. Not verified live.
#[post("workspace/create", class = Write)]
pub async fn create(client: &Client, #[body] body: &WorkspaceName) -> Result<Value>;

/// The tree reply as text, for the parsers that skip a `Value`.
#[get("workspace/{workspace_id}/tree", class = Read, response = text)]
pub async fn tree_text(client: &Client, workspace_id: &str) -> Result<String>;

/// The trash as a search index; every node in it counts as trashed.
///
/// # Errors
/// [`Client::send_json`] errors.
pub async fn trash_index(
    client: &Client,
    workspace_id: &str,
) -> Result<crate::domain::tree::name_index::build::NameIndex> {
    Ok(crate::domain::tree::name_index::build::NameIndex::of_trash(
        &trash(client, workspace_id).await?,
    ))
}

/// The tree as a compact search index.
///
/// # Errors
/// [`Client::send_text`] errors or
/// [`crate::domain::tree::name_index::build::NameIndex::parse`] errors.
pub async fn tree_index(
    client: &Client,
    workspace_id: &str,
) -> Result<crate::domain::tree::name_index::build::NameIndex> {
    parse_tree(&tree_text(client, workspace_id).await?)
}

/// A tree reply, asked or kept, as a compact search index.
///
/// # Errors
/// [`crate::domain::tree::name_index::build::NameIndex::parse`] errors.
pub fn parse_tree(
    text: &str,
) -> Result<crate::domain::tree::name_index::build::NameIndex> {
    let (index, fallback) =
        crate::domain::tree::name_index::build::NameIndex::parse(text)?;
    Ok(logged(index, fallback))
}

/// The event log by `query` (needs a plan with audit). Not verified live.
#[get("audit_events", class = Admin, query = query.iter())]
pub async fn audit_events(client: &Client, query: &[(&str, String)]) -> Result<Value>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[
    GET,
    TRASH,
    LAST_ACTIVE_DATES,
    JOIN_REQUESTS,
    CREATED_CHANNELS_COUNT,
    SSO_SETTINGS,
    DOMAINS,
    RENAME,
    CREATE,
    TREE_TEXT,
    AUDIT_EVENTS,
];
