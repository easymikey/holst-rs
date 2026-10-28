//! `board create rename mv cp rm restore`: changes of the tree for both shells.

use serde::Serialize;
use serde_json::Value;

use crate::{
    command::{
        board::{
            CopyArgs,
            CreateArgs,
            MoveArgs,
            NewKind,
            RemoveArgs,
            RenameArgs,
            RestoreArgs,
        },
        change::{ChangeReport, settle},
        lookup::{self, NodeLookup},
        node::{self, Carrying, Copying},
        table::Confirmation,
    },
    domain::{
        id::{BoardId, WorkspaceId},
        lookup::choice::{self, MatchCoverage},
        node::{DeleteMode, NewNode, NodeBrief, NodeKind, NodeName, RightsCopy},
        text::printable,
    },
    effect::{board_write::WriteMode, request::ApiRequest},
    error::{
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        lookup::{LookupError, LookupKind, LookupMiss},
    },
    protocol::json::changed_node,
    remote::{api, client::Client},
};

/// What a tree change did to its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Rename,
    Move,
    Copy,
    Trash,
    DeleteForever,
    Restore,
    Favorite,
    Unfavorite,
    Touch,
    Leave,
}

/// A change of the tree: the node it touched and whether it went out.
#[derive(Debug, Serialize)]
pub struct Changed {
    #[serde(flatten)]
    pub node: ChangedNode,
    pub outcome: ChangeReport,
}

/// The node a change touched; what the change cannot know stays `None`.
#[derive(Debug, Serialize)]
pub struct ChangedNode {
    pub action: Action,
    pub id: Option<String>,
    /// The name it has after the change.
    pub name: Option<String>,
    /// The name before a rename.
    pub previous_name: Option<String>,
    pub kind: Option<&'static str>,
    /// Where a board opens.
    pub link: Option<String>,
    /// The live boards a folder takes along when it moves or goes to the trash.
    pub boards_along: usize,
    /// What a folder deleted for good left behind (moved up, not deleted).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_behind: Vec<LeftBehind>,
    /// The folder they are in now; left out: the workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_in: Option<String>,
}

/// A board or folder that was inside a folder deleted for good.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeftBehind {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
}

/// A request with the node it touches and the refusal it stands under.
pub(super) struct Plan {
    request: ApiRequest,
    node: ChangedNode,
    unconfirmed: Option<Unconfirmed>,
}

impl Plan {
    /// A change that needs no confirmation.
    pub(super) const fn plain(request: ApiRequest, node: ChangedNode) -> Self {
        Self {
            request,
            node,
            unconfirmed: None,
        }
    }
}

pub(super) fn node_of(
    action: Action,
    id: Option<String>,
    kind: Option<NodeKind>,
) -> ChangedNode {
    ChangedNode {
        action,
        id,
        name: None,
        previous_name: None,
        kind: kind.map(NodeKind::as_str),
        link: None,
        boards_along: 0,
        left_behind: Vec::new(),
        left_in: None,
    }
}

impl ChangedNode {
    /// One sentence on what the change did, or with `would` what it would do.
    #[must_use]
    pub fn phrase(&self, would: bool) -> String {
        let (done, planned) = match self.action {
            Action::Create => ("created", "would create"),
            Action::Rename => ("renamed", "would rename"),
            Action::Move | Action::Trash => ("moved", "would move"),
            Action::Copy => ("copied", "would copy"),
            Action::DeleteForever => ("deleted", "would delete"),
            Action::Restore => ("restored", "would restore"),
            Action::Favorite => ("added", "would add"),
            Action::Unfavorite => ("removed", "would remove"),
            Action::Touch => ("marked", "would mark"),
            Action::Leave => ("left", "would leave"),
        };
        let verb = if would { planned } else { done };
        let name = printable(self.name.as_deref().unwrap_or_default());
        let subject = self.name.as_deref().or(self.id.as_deref()).map_or_else(
            || String::from("the board"),
            |text| printable(text).into_owned(),
        );
        let boards_along = match self.boards_along {
            0 => String::new(),
            1 => String::from(", with 1 board"),
            count => format!(", with {count} boards"),
        };
        match self.action {
            Action::Create => {
                format!("{verb} {} {name}", self.kind.unwrap_or("board"))
            }
            Action::Rename => format!(
                "{verb} {} to {name}",
                printable(self.previous_name.as_deref().unwrap_or("it"))
            ),
            Action::Move => format!("{verb} {subject}{boards_along}"),
            Action::Copy => match self.name.as_deref() {
                Some(_) => format!("{verb} the board as {name}"),
                None => format!("{verb} the board"),
            },
            Action::Trash => format!("{verb} {subject} to the trash{boards_along}"),
            Action::DeleteForever => {
                format!("{verb} {subject} for good{}", self.left_words(would))
            }
            Action::Restore | Action::Leave => format!("{verb} {subject}"),
            Action::Favorite => format!("{verb} {subject} to the favorites"),
            Action::Unfavorite => format!("{verb} {subject} from the favorites"),
            Action::Touch => format!("{verb} {subject} as recently opened"),
        }
    }

    /// Where a folder deleted for good left its contents; empty without any.
    fn left_words(&self, would: bool) -> String {
        let list = self
            .left_behind
            .iter()
            .map(|node| format!("{} ({})", printable(&node.name), node.kind))
            .collect::<Vec<_>>()
            .join(", ");
        let be = match (would, self.left_behind.len()) {
            (true, _) => "would be",
            (false, 1) => "is now",
            (false, _) => "are now",
        };
        let place = self.left_in.as_deref().map_or_else(
            || String::from("at the workspace root"),
            |folder| format!("in the folder {}", printable(folder)),
        );
        if list.is_empty() {
            list
        } else {
            format!("; {list} {be} {place}")
        }
    }

    /// Fills what the server's reply tells and the plan did not know.
    fn learn(&mut self, reply: &Value) {
        let about = changed_node(reply);
        self.id = self.id.take().or(about.id);
        self.name = self.name.take().or(about.name);
        let moved = reply
            .get("updatedChannelIds")
            .and_then(Value::as_array)
            .filter(|ids| !ids.is_empty());
        if let (Action::DeleteForever, Some(ids)) = (self.action, moved) {
            self.left_behind
                .retain(|node| ids.iter().any(|id| id.as_str() == Some(&node.id)));
        }
    }
}

/// Creates a board or a folder.
///
/// # Errors
/// A blank name, a workspace or folder that is not found, and errors of the
/// requests.
pub async fn create(client: &Client, args: &CreateArgs) -> Result<Changed> {
    let name: NodeName = args.name.parse()?;
    let kind = match args.kind.unwrap_or_default() {
        NewKind::Board => NodeKind::Board,
        NewKind::Folder => NodeKind::Folder,
    };
    let workspace = lookup::workspace(client, args.workspace.as_deref()).await?;
    let workspace: WorkspaceId = workspace.id.parse()?;
    let parent =
        node::parent(client, &workspace, args.folder.as_deref(), choice::listed)
            .await?;
    let shown = name.to_string();
    let new = NewNode {
        name,
        kind,
        workspace,
        parent,
    };
    let request = node::plan_create(client, &new).await?;
    let node = ChangedNode {
        name: Some(shown),
        ..node_of(Action::Create, None, Some(kind))
    };
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// Renames a board or a folder.
///
/// # Errors
/// A blank name, a node that is not found, and errors of the requests.
pub async fn rename(client: &Client, args: &RenameArgs) -> Result<Changed> {
    let name: NodeName = args.name.parse()?;
    let id = node_id(client, &args.board, args.workspace.as_deref()).await?;
    let before = NodeBrief::of(&api::channel::get(client, &id).await?)?
        .name()
        .to_owned();
    let request = api::channel::rename_request(&id, name.as_str())?;
    let node = ChangedNode {
        name: Some(name.to_string()),
        previous_name: Some(before).filter(|before| !before.is_empty()),
        ..node_of(Action::Rename, Some(id), None)
    };
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// Moves a board or a folder into a folder, or to the root. A folder with
/// boards stays unmoved without its confirmation.
///
/// # Errors
/// A node or folder that is not found, a folder moved into itself, and errors
/// of the requests.
pub async fn mv(client: &Client, args: &MoveArgs) -> Result<Changed> {
    let id: BoardId = node_id(client, &args.board, args.workspace.as_deref())
        .await?
        .parse()?;
    let located = node::locate(client, &id).await?;
    // The destination is found before the confirmation is asked for.
    let parent = located.parent(Some(&args.folder), choice::listed)?;
    let node = ChangedNode {
        name: Some(located.name.clone()),
        boards_along: located.boards_along,
        ..node_of(Action::Move, Some(id.to_string()), Some(located.kind))
    };
    let plan = Plan {
        request: located.plan_move(&parent)?,
        node,
        unconfirmed: located.contents_unconfirmed(Carrying::Move),
    };
    carry(client, plan, (args.dry_run, args.confirm)).await
}

/// Copies a board.
///
/// # Errors
/// A board or folder that is not found, and errors of the requests.
pub async fn cp(client: &Client, args: &CopyArgs) -> Result<Changed> {
    let id: BoardId = lookup::board_id(client, &args.board, args.workspace.as_deref())
        .await?
        .parse()?;
    let copying = Copying {
        name: args.name.as_deref(),
        folder: args.folder.as_deref(),
        permissions: if args.rights {
            RightsCopy::With
        } else {
            RightsCopy::Without
        },
    };
    let request = node::plan_copy(client, &id, &copying, choice::listed).await?;
    let node = ChangedNode {
        name: args.name.clone(),
        ..node_of(Action::Copy, None, Some(NodeKind::Board))
    };
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// Moves a board or a folder to the trash, or with `permanently` deletes it
/// for good. A folder with boards and a deletion for good stay undone without
/// their confirmation.
///
/// # Errors
/// A node that is not found (for good, a board must be named whole), and
/// errors of the requests.
pub async fn rm(client: &Client, args: &RemoveArgs) -> Result<Changed> {
    let plan = if args.permanently {
        delete_forever(client, args).await?
    } else {
        trash(client, args).await?
    };
    carry(client, plan, (args.dry_run, args.confirm)).await
}

async fn trash(client: &Client, args: &RemoveArgs) -> Result<Plan> {
    let id: BoardId = node_id(client, &args.board, args.workspace.as_deref())
        .await?
        .parse()?;
    let located = node::locate(client, &id).await?;
    let node = ChangedNode {
        name: Some(located.name.clone()),
        boards_along: located.boards_along,
        ..node_of(Action::Trash, Some(id.to_string()), Some(located.kind))
    };
    Ok(Plan {
        request: located.plan_trash()?,
        node,
        unconfirmed: located.contents_unconfirmed(Carrying::Trash),
    })
}

async fn delete_forever(client: &Client, args: &RemoveArgs) -> Result<Plan> {
    let id =
        lookup::exact_board_id(client, &args.board, args.workspace.as_deref()).await?;
    let reply = api::channel::get(client, &id).await?;
    let brief = NodeBrief::of(&reply)?;
    let name = brief.name().to_owned();
    let (left_behind, left_in) = if reply.get("type").and_then(Value::as_str)
        == Some(NodeKind::Folder.as_str())
    {
        left_by(client, &id, &brief).await?
    } else {
        (Vec::new(), None)
    };
    let shown = if name.is_empty() { &id } else { &name };
    let unconfirmed = Unconfirmed {
        change: format!("deleting {shown} for good").into(),
        risk: ChangeRisk::Irreversible,
    };
    let node = ChangedNode {
        name: Some(name).filter(|name| !name.is_empty()),
        left_behind,
        left_in,
        ..node_of(Action::DeleteForever, Some(id.clone()), None)
    };
    Ok(Plan {
        request: api::channel::delete_request(&id, DeleteMode::Permanent)?,
        node,
        unconfirmed: Some(unconfirmed),
    })
}

/// What is directly inside the live folder `id`, and the folder it is in (none:
/// the root): where a deletion for good puts the contents.
async fn left_by(
    client: &Client,
    id: &str,
    brief: &NodeBrief,
) -> Result<(Vec<LeftBehind>, Option<String>)> {
    match brief.workspace_id() {
        None => Ok((Vec::new(), None)),
        Some(workspace) => {
            let index = api::workspace::tree_index(client, workspace).await?;
            let found = index.find_id(id).filter(|&node| !index.in_trash(node));
            Ok(found.map_or_else(
                || (Vec::new(), None),
                |folder| {
                    let inside = index
                        .nodes()
                        .filter(|&node| {
                            index.parent(node) == Some(folder) && !index.in_trash(node)
                        })
                        .map(|node| LeftBehind {
                            id: index.id(node),
                            name: index.name(node).to_owned(),
                            kind: index.kind(node).as_str(),
                        })
                        .collect();
                    let into = index.parent(folder).map(|up| index.name(up).to_owned());
                    (inside, into)
                },
            ))
        }
    }
}

/// Brings a board or a folder back from the trash.
///
/// # Errors
/// A workspace or trashed node that is not found, an id that is not in the
/// trash, and errors of the requests.
pub async fn restore(client: &Client, args: &RestoreArgs) -> Result<Changed> {
    let (id, name) = if let Some(id) = BoardId::find_in(&args.board) {
        trashed_by_id(client, String::from(id)).await?
    } else {
        let workspace = lookup::workspace(client, args.workspace.as_deref()).await?;
        let id =
            lookup::trashed(client, &workspace.id, &args.board, choice::listed).await?;
        (id, None)
    };
    let request = api::channel::restore_request(&id)?;
    let node = ChangedNode {
        name,
        ..node_of(Action::Restore, Some(id), None)
    };
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// An id that is in the trash, with its name; anything else is not found there.
async fn trashed_by_id(
    client: &Client,
    id: String,
) -> Result<(String, Option<String>)> {
    let node = NodeBrief::of(&api::channel::get(client, &id).await?)?;
    let name = Some(node.name().to_owned()).filter(|name| !name.is_empty());
    node.in_trash()
        .then_some((id.clone(), name))
        .ok_or_else(|| {
            LookupError {
                kind: LookupKind::Trash,
                reference: id,
                miss: LookupMiss::not_found(),
            }
            .into()
        })
}

/// A board or folder by id, link or part of its name.
async fn node_id(
    client: &Client,
    reference: &str,
    workspace: Option<&str>,
) -> Result<String> {
    lookup::node(
        client,
        reference,
        workspace,
        NodeLookup {
            filter: choice::NODES,
            coverage: MatchCoverage::Partial,
        },
    )
    .await
}

/// Sends the plan, reports it for a dry run, or holds it for want of its
/// confirmation (a dry run needs none).
pub(super) async fn carry(
    client: &Client,
    plan: Plan,
    (dry_run, confirm): (WriteMode, Confirmation),
) -> Result<Changed> {
    let Plan {
        request,
        mut node,
        unconfirmed,
    } = plan;
    let (outcome, reply) =
        settle(client, request, unconfirmed, (dry_run, confirm)).await?;
    if let Some(reply) = &reply {
        node.learn(reply);
    }
    node.link = match (&node.id, node.kind) {
        (Some(id), Some(kind)) if kind == NodeKind::Board.as_str() => {
            Some(client.board_link(id))
        }
        _ => None,
    };
    Ok(Changed { node, outcome })
}

#[cfg(test)] mod tests;
