use holst_macros::get;
use serde_json::Value;

use crate::{
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

/// `GET /gapi/current_user` — the profile and the workspaces.
#[get("current_user", class = Read)]
pub async fn current(client: &Client) -> Result<Value>;

/// Workspaces one can join. Not verified live.
#[get("current_user/joinable_workspaces", class = Read)]
pub async fn joinable_workspaces(client: &Client) -> Result<Value>;

/// The PAT list via a normal session (a PAT fails on `/gapi/*`).
#[get("personal_access_token/list", class = Read, query(statuses = "active"))]
pub async fn pat_list(client: &Client) -> Result<Value>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[CURRENT, JOINABLE_WORKSPACES, PAT_LIST];
