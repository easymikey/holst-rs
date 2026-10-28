use holst_macros::{get, patch, post};
use serde_json::{Value, json};

use crate::{domain::tree::TreeFallback, error::Result, remote::client::Client};

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
#[get("workspace/{workspace_id}")]
pub async fn get(client: &Client, workspace_id: &str) -> Result<Value>;

/// The trash.
#[get("workspace/{workspace_id}/trash")]
pub async fn trash(client: &Client, workspace_id: &str) -> Result<Value>;

/// When members were last active.
#[get("workspace_user/{workspace_id}/list_last_active_dates")]
pub async fn last_active_dates(client: &Client, workspace_id: &str) -> Result<Value>;

/// How many boards a user created in the workspace; `userId` is required.
#[get("workspace/{workspace_id}/created_channels_count", query(userId = user_id))]
pub async fn created_channels_count(
    client: &Client,
    workspace_id: &str,
    user_id: &str,
) -> Result<Value>;

/// SSO settings.
#[get("workspace/{workspace_id}/sso_settings")]
pub async fn sso_settings(client: &Client, workspace_id: &str) -> Result<Value>;

/// Verified domains.
#[get("workspace_domains/{workspace_id}/list")]
pub async fn domains(client: &Client, workspace_id: &str) -> Result<Value>;

/// Rename a workspace. Not verified live.
#[patch("workspace/{workspace_id}", body = json!({ "name": name }))]
pub async fn rename(client: &Client, workspace_id: &str, name: &str) -> Result<Value>;

/// Create a workspace. Not verified live.
#[post("workspace/create", body = json!({ "name": name }))]
pub async fn create(client: &Client, name: &str) -> Result<Value>;

/// The tree reply as text, for the parsers that skip a `Value`.
#[get("workspace/{workspace_id}/tree", response = text)]
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
    let (index, fallback) = crate::domain::tree::name_index::build::NameIndex::parse(
        &tree_text(client, workspace_id).await?,
    )?;
    Ok(logged(index, fallback))
}

/// The event log by `query` (needs a plan with audit). Not verified live.
#[get("audit_events", query = query.iter())]
pub async fn audit_events(client: &Client, query: &[(&str, String)]) -> Result<Value>;
