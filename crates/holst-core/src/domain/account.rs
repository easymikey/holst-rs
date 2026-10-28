//! The signed-in user and the workspaces they belong to.

use serde_json::Value;

use crate::error::{Error, Result, http::HttpError};

/// A workspace as the choice sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
}

/// The signed-in user as `GET /gapi/current_user` describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: Option<String>,
    pub workspaces: Vec<Workspace>,
}

impl Account {
    /// The user of a `current_user` reply.
    ///
    /// ```
    /// use holst_core::domain::account::Account;
    /// use serde_json::json;
    ///
    /// let me = Account::of(&json!({ "id": "u1", "name": "Ann" }))?;
    /// assert_eq!((me.id.as_str(), me.name.as_deref()), ("u1", Some("Ann")));
    /// assert!(Account::of(&json!({})).is_err());
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`crate::error::http::HttpError::MissingField`] when there is no `id`.
    pub fn of(response: &Value) -> Result<Self> {
        let text = |key| response.get(key).and_then(Value::as_str).map(str::to_owned);
        Ok(Self {
            id: text("id").ok_or(Error::Http(HttpError::MissingField {
                response: "current_user",
                field: "id",
            }))?,
            name: text("name"),
            workspaces: account_workspaces(response),
        })
    }
}

/// Workspaces of a `current_user` reply, deduplicated in first-seen order.
///
/// ```
/// use holst_core::domain::account::account_workspaces;
/// use serde_json::json;
///
/// let me = json!({
///     "workspace_users": [{ "workspace_id": "w1" }],
///     "workspaces": [{ "id": "w1", "name": "easy" }],
/// });
/// let all = account_workspaces(&me);
/// assert_eq!((all.len(), all[0].name.as_str()), (1, "easy"));
/// ```
#[must_use]
pub fn account_workspaces(response: &Value) -> Vec<Workspace> {
    ["workspaces", "workspace_users", "workspaceUsers"]
        .into_iter()
        .filter_map(|key| response.get(key)?.as_array())
        .flatten()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("workspace_id"))
                .or_else(|| item.get("workspaceId"))?
                .as_str()?;
            let name = item
                .get("name")
                .or_else(|| item.pointer("/workspace/name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            Some((id, name))
        })
        .fold(Vec::new(), |mut out: Vec<Workspace>, (id, name)| {
            match out.iter_mut().find(|known| known.id == id) {
                Some(known) => known.name = merged_name(&known.name, name),
                None => out.push(Workspace {
                    id: id.to_owned(),
                    name: name.to_owned(),
                }),
            }
            out
        })
}

fn merged_name(known: &str, fresh: &str) -> String {
    match known {
        "" => fresh,
        known => known,
    }
    .to_owned()
}
