use holst_macros::get;
use serde_json::Value;

use crate::{error::Result, remote::client::Client};

/// The current plan.
#[get("subscription/get_subscription_with_billing_details/{workspace_id}")]
pub async fn current(client: &Client, workspace_id: &str) -> Result<Value>;

/// The plans one can buy, with their ids.
#[get("subscription/get_available_plans")]
pub async fn available_plans(client: &Client) -> Result<Value>;
