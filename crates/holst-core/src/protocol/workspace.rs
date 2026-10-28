//! The replies of the workspace requests, as the server words them.

use serde::Deserialize;
use serde_json::Value;

/// `GET /gapi/workspace/<id>`.
#[derive(Debug, Deserialize)]
pub(crate) struct WorkspaceReply {
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    #[serde(rename = "members_count")]
    pub(crate) member_count: Option<u64>,
    #[serde(rename = "users_count_by_role")]
    pub(crate) counts_by_role: Option<RoleCounts>,
    pub(crate) users: Option<Vec<ReplyUser>>,
    #[serde(rename = "workspace_users")]
    pub(crate) seats: Option<Vec<Seat>>,
}

/// How many users hold each role; only the guests are read.
#[derive(Debug, Deserialize)]
pub(crate) struct RoleCounts {
    pub(crate) guest: Option<u64>,
}

/// A user of a workspace reply.
#[derive(Debug, Deserialize)]
pub(crate) struct ReplyUser {
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) email: Option<String>,
}

/// What a user is in the workspace.
#[derive(Debug, Deserialize)]
pub(crate) struct Seat {
    pub(crate) user_id: Option<String>,
    pub(crate) role: Option<String>,
    #[serde(rename = "last_active_at")]
    pub(crate) last_active: Option<String>,
}

/// `GET /gapi/current_user`: the lists that name the user's workspaces.
#[derive(Debug, Deserialize)]
pub(crate) struct CurrentUser {
    pub(crate) workspaces: Option<Vec<WorkspaceItem>>,
    #[serde(rename = "workspace_users", alias = "workspaceUsers")]
    pub(crate) seats: Option<Vec<WorkspaceItem>>,
}

/// A workspace, or the user's seat in one, in a list of the current user.
#[derive(Debug, Deserialize)]
pub(crate) struct WorkspaceItem {
    pub(crate) id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    #[serde(rename = "workspaceId")]
    pub(crate) workspace_id_camel: Option<String>,
    pub(crate) organization: Option<Organization>,
    pub(crate) role: Option<String>,
}

impl WorkspaceItem {
    /// The id of the workspace the item is about.
    pub(crate) fn workspace(&self) -> Option<&str> {
        [&self.id, &self.workspace_id, &self.workspace_id_camel]
            .into_iter()
            .find_map(|id| id.as_deref())
    }
}

/// An organization: its name, or an object that has one.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Organization {
    Name(String),
    Named { name: Option<String> },
}

impl Organization {
    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Organization::Name(name) => Some(name),
            Organization::Named { name } => name.as_deref(),
        }
    }
}

/// `GET /gapi/current_user/joinable_workspaces`: a list, or an object with one.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JoinableReply {
    List(Vec<JoinableItem>),
    Wrapped {
        workspaces: Option<Vec<JoinableItem>>,
    },
}

impl JoinableReply {
    pub(crate) fn into_items(self) -> Vec<JoinableItem> {
        match self {
            JoinableReply::List(items) => items,
            JoinableReply::Wrapped { workspaces } => workspaces.unwrap_or_default(),
        }
    }
}

/// A workspace the user can join.
#[derive(Debug, Deserialize)]
pub(crate) struct JoinableItem {
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) organization: Option<Organization>,
    #[serde(rename = "members_count")]
    pub(crate) member_count: Option<u64>,
}

/// The audit events: a list, or a page with a cursor.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum EventsReply {
    List(Vec<Value>),
    Page {
        events: Option<Vec<Value>>,
        cursor: Option<String>,
        next_cursor: Option<String>,
    },
}

impl EventsReply {
    /// The events and the cursor of the next page.
    pub(crate) fn into_page(self) -> (Vec<Value>, Option<String>) {
        match self {
            EventsReply::List(events) => (events, None),
            EventsReply::Page {
                events,
                cursor,
                next_cursor,
            } => (events.unwrap_or_default(), cursor.or(next_cursor)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use serde_json::{Error, json};

    use crate::protocol::workspace::{EventsReply, JoinableReply};

    #[test]
    fn a_page_gives_either_cursor_and_a_bare_list_none() -> Result<(), Error> {
        let page = |value| {
            Ok::<_, Error>(serde_json::from_value::<EventsReply>(value)?.into_page())
        };
        assert_eq!(
            (
                page(json!({ "events": [1], "next_cursor": "c" }))?.1,
                page(json!([1, 2]))?.1
            ),
            (Some("c".to_owned()), None)
        );
        Ok(())
    }

    #[test]
    fn an_object_without_workspaces_joins_nothing() -> Result<(), Error> {
        let reply: JoinableReply = serde_json::from_value(json!({}))?;
        assert!(reply.into_items().is_empty());
        Ok(())
    }
}
