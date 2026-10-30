//! `board ls`: the tree of boards and folders, or another view of them.

use futures_util::future::{join_all, try_join};
use itertools::Itertools as _;
use serde::Serialize;
use serde_json::Value;

use crate::{
    command::{
        board::{ListView, LsArgs},
        lookup,
        node,
        table::not_value,
    },
    domain::{
        account::Workspace,
        id::WorkspaceId,
        lookup::choice,
        node::{NewParent, NodeKind},
        tree::{
            self,
            name_index::build::{NameIndex, NodeIndex},
            walk::TrashWalk,
        },
    },
    error::{Error, Result},
    remote::{api, client::Client},
};

const COMMAND: &str = "board ls";
/// How many recent boards one page holds when `-l` is left out.
const RECENT_PAGE: u32 = 20;
/// The most recent boards the server gives in one page.
const RECENT_MAX: u32 = 100;

/// A page of boards and folders in one view.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub view: ListView,
    /// The workspaces the view looked at.
    pub workspaces: Vec<ListedWorkspace>,
    pub nodes: Vec<ListedNode>,
    /// How many nodes the view holds, before paging.
    pub total: usize,
    /// Where the next page starts; `None` on the last page.
    pub cursor: Option<String>,
}

impl Listing {
    /// Each node with the workspace to head it, when several workspaces are
    /// listed and the node starts a new one.
    pub fn sections(
        &self,
    ) -> impl Iterator<Item = (Option<&ListedWorkspace>, &ListedNode)> + Clone {
        let several = self.workspaces.len() > 1;
        let before = std::iter::once(None)
            .chain(self.nodes.iter().map(|node| node.workspace.as_deref()));
        self.nodes.iter().zip(before).map(move |(node, before)| {
            let heading = node
                .workspace
                .as_deref()
                .filter(|current| several && Some(*current) != before)
                .and_then(|current| self.workspaces.iter().find(|w| w.id == current));
            (heading, node)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedWorkspace {
    pub id: String,
    pub name: String,
}

impl From<Workspace> for ListedWorkspace {
    fn from(workspace: Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name,
        }
    }
}

/// A board or folder of a view; `depth` is its place under its folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedNode {
    /// The id of its workspace; `None` for a recent board no tree holds.
    pub workspace: Option<String>,
    pub id: String,
    /// Empty for a recent board no tree holds.
    pub name: String,
    pub kind: NodeKind,
    pub depth: usize,
    pub in_trash: bool,
}

/// Lists boards and folders in the view asked for, one page of it.
///
/// # Errors
/// A workspace, folder or cursor that is not found or not valid, and errors
/// of the requests.
pub async fn ls(client: &Client, args: &LsArgs) -> Result<Listing> {
    let view = args.view.unwrap_or_default();
    folder_fits(view, args.folder.as_deref())?;
    match view {
        ListView::Recent => recent(client, args).await,
        ListView::Tree
        | ListView::Trash
        | ListView::Both
        | ListView::Favorites
        | ListView::Sidebar => in_workspaces(client, args, view).await,
    }
}

fn folder_fits(view: ListView, folder: Option<&str>) -> Result<()> {
    match (view, folder) {
        (
            ListView::Trash
            | ListView::Favorites
            | ListView::Recent
            | ListView::Sidebar,
            Some(_),
        ) => Err(not_value(
            COMMAND,
            "folder",
            "left out with this view; only tree and both look into a folder",
        )),
        (
            ListView::Tree
            | ListView::Trash
            | ListView::Both
            | ListView::Favorites
            | ListView::Recent
            | ListView::Sidebar,
            _,
        ) => Ok(()),
    }
}

async fn in_workspaces(
    client: &Client,
    args: &LsArgs,
    view: ListView,
) -> Result<Listing> {
    let workspaces = match args.workspace.as_deref() {
        Some(reference) => vec![lookup::workspace(client, Some(reference)).await?],
        None => lookup::workspaces(client).await?,
    };
    let folder = args.folder.as_deref();
    let sections = join_all(
        workspaces
            .iter()
            .map(|workspace| section(client, workspace, view, folder)),
    )
    .await;
    let nodes = settled(sections, folder.is_some())?;
    paged(view, workspaces, nodes, args)
}

/// The sections of all workspaces, or the first error. With a folder, a
/// workspace that does not have it stays out while another one has.
fn settled(
    sections: Vec<Result<Vec<ListedNode>>>,
    narrowed: bool,
) -> Result<Vec<ListedNode>> {
    let (found, failed): (Vec<_>, Vec<_>) = sections.into_iter().partition_result();
    let skippable = narrowed
        && !found.is_empty()
        && failed.iter().all(|error| matches!(error, Error::Lookup(_)));
    match failed.into_iter().next() {
        Some(error) if !skippable => Err(error),
        Some(_) | None => Ok(found.into_iter().flatten().collect()),
    }
}

fn paged(
    view: ListView,
    workspaces: Vec<Workspace>,
    nodes: Vec<ListedNode>,
    args: &LsArgs,
) -> Result<Listing> {
    let total = nodes.len();
    let start = cursor_start(args.cursor.as_deref())?;
    let size = args
        .limit
        .filter(|&limit| limit > 0)
        .map_or(usize::MAX, |limit| {
            usize::try_from(limit).unwrap_or(usize::MAX)
        });
    let end = start.saturating_add(size).min(total);
    Ok(Listing {
        view,
        workspaces: workspaces.into_iter().map(ListedWorkspace::from).collect(),
        nodes: nodes.into_iter().skip(start).take(size).collect(),
        total,
        cursor: (end < total).then(|| end.to_string()),
    })
}

fn cursor_start(cursor: Option<&str>) -> Result<usize> {
    cursor.map_or(Ok(0), |text| {
        text.trim().parse().map_err(|_| {
            not_value(COMMAND, "cursor", "the cursor a previous page printed")
        })
    })
}

async fn section(
    client: &Client,
    workspace: &Workspace,
    view: ListView,
    folder: Option<&str>,
) -> Result<Vec<ListedNode>> {
    match view {
        ListView::Tree => live(client, workspace, folder).await,
        ListView::Trash => trashed(client, workspace).await,
        ListView::Both => {
            let (live, trash) =
                try_join(live(client, workspace, folder), trashed(client, workspace))
                    .await?;
            Ok(live.into_iter().chain(trash).collect())
        }
        ListView::Favorites => favorites(client, workspace).await,
        ListView::Sidebar => sidebar(client, workspace).await,
        // Per account: `ls` reads it before any workspace is chosen.
        ListView::Recent => Ok(Vec::new()),
    }
}

async fn live(
    client: &Client,
    workspace: &Workspace,
    folder: Option<&str>,
) -> Result<Vec<ListedNode>> {
    let index = api::workspace::tree_index(client, &workspace.id).await?;
    let root = folder_node(&index, workspace, folder)?;
    Ok(walked(workspace, &index, root, TrashWalk::Hidden))
}

async fn trashed(client: &Client, workspace: &Workspace) -> Result<Vec<ListedNode>> {
    let index = api::workspace::trash_index(client, &workspace.id).await?;
    Ok(walked(workspace, &index, None, TrashWalk::Shown))
}

/// The folder `reference` names in the live tree; `None` is the whole tree.
fn folder_node(
    index: &NameIndex,
    workspace: &Workspace,
    reference: Option<&str>,
) -> Result<Option<NodeIndex>> {
    let id: WorkspaceId = workspace.id.parse()?;
    Ok(match reference {
        None => None,
        Some(_) => match node::look_up_parent(index, &id, reference, choice::listed)? {
            NewParent::Root => None,
            NewParent::Folder(folder) => index.find_id(folder.as_str()),
        },
    })
}

fn walked(
    workspace: &Workspace,
    index: &NameIndex,
    root: Option<NodeIndex>,
    trash: TrashWalk,
) -> Vec<ListedNode> {
    index
        .walk(root, trash)
        .into_iter()
        .map(|(node, depth)| listed(workspace, index, node, depth))
        .collect()
}

fn listed(
    workspace: &Workspace,
    index: &NameIndex,
    node: NodeIndex,
    depth: usize,
) -> ListedNode {
    ListedNode {
        workspace: Some(workspace.id.clone()),
        id: index.id(node),
        name: index.name(node).to_owned(),
        kind: index.kind(node),
        depth,
        in_trash: index.in_trash(node),
    }
}

async fn favorites(client: &Client, workspace: &Workspace) -> Result<Vec<ListedNode>> {
    let reply = api::favorite::list(client, &workspace.id).await?;
    Ok(tree::flatten(&reply)
        .into_iter()
        .map(|node| ListedNode {
            workspace: Some(workspace.id.clone()),
            id: node.id,
            name: node.name,
            kind: node.kind,
            depth: 0,
            in_trash: node.in_trash,
        })
        .collect())
}

async fn sidebar(client: &Client, workspace: &Workspace) -> Result<Vec<ListedNode>> {
    let (order, index) = try_join(
        api::channel::order_index_list(client, &workspace.id),
        api::workspace::tree_index(client, &workspace.id),
    )
    .await?;
    Ok(ordered_ids(&order)
        .iter()
        .filter_map(|id| index.find_id(id))
        .map(|node| listed(workspace, &index, node, 0))
        .collect())
}

/// The ids of a user's order index, first to last. The reply is an array (or an
/// object holding one) of entries with a channel id and an `order_index`.
fn ordered_ids(reply: &Value) -> Vec<String> {
    let entries = reply.as_array().or_else(|| {
        reply
            .as_object()
            .and_then(|map| map.values().find_map(Value::as_array))
    });
    entries
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let field = |keys: &[&str]| keys.iter().find_map(|key| entry.get(*key));
            let id = field(&["channel_id", "channelId", "id"])?.as_str()?;
            let order = field(&["order_index", "index"])
                .and_then(Value::as_f64)
                .unwrap_or(f64::INFINITY);
            Some((order, id.to_owned()))
        })
        .sorted_by(|(left, _), (right, _)| left.total_cmp(right))
        .map(|(_, id)| id)
        .collect()
}

/// The recent boards of the account, newest first, named from the trees.
async fn recent(client: &Client, args: &LsArgs) -> Result<Listing> {
    let workspaces = lookup::workspaces(client).await?;
    let size = match args.limit {
        None => RECENT_PAGE,
        Some(0) => RECENT_MAX,
        Some(limit) => limit.min(RECENT_MAX),
    };
    let reply = api::recent::page(client, size, args.cursor.as_deref()).await?;
    let trees = lookup::trees(client, workspaces.clone())
        .await
        .into_iter()
        .map(|(workspace, index)| index.map(|index| (workspace, index)))
        .collect::<Result<Vec<_>>>()?;
    let nodes: Vec<ListedNode> = reply
        .channels
        .iter()
        .map(|entry| recent_node(&trees, &entry.channel_id))
        .collect();
    Ok(Listing {
        view: ListView::Recent,
        workspaces: workspaces.into_iter().map(ListedWorkspace::from).collect(),
        total: nodes.len(),
        nodes,
        cursor: reply.cursor,
    })
}

fn recent_node(trees: &[(Workspace, NameIndex)], id: &str) -> ListedNode {
    trees
        .iter()
        .find_map(|(workspace, index)| {
            index
                .find_id(id)
                .map(|node| listed(workspace, index, node, 0))
        })
        .unwrap_or_else(|| ListedNode {
            workspace: None,
            id: id.to_owned(),
            name: String::new(),
            kind: NodeKind::Other,
            depth: 0,
            in_trash: false,
        })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};

    use crate::{
        Result,
        command::{
            board::{
                LsArgs,
                tree::{Listing, ls},
            },
            fixture::uuid,
            table::parse,
        },
        domain::node::NodeKind,
        remote::client::fake::{on_fake, reply, stored},
    };

    fn account() -> String {
        json!({
            "id": "u1",
            "name": "Ann",
            "workspaces": [
                { "id": uuid(1), "name": "easy" },
                { "id": uuid(2), "name": "team" },
            ],
        })
        .to_string()
    }

    fn easy_tree() -> String {
        json!({ "channels": [
            { "id": uuid(11), "type": "folder", "name": "Plans", "order_index": 1.0 },
            { "id": uuid(12), "type": "board", "name": "Roadmap", "parent_id": uuid(11), "order_index": 1.0 },
            { "id": uuid(13), "type": "board", "name": "Retro", "order_index": 2.0 },
            { "id": uuid(14), "type": "board", "name": "Notes", "order_index": 3.0 },
        ]})
        .to_string()
    }

    fn team_tree() -> String {
        json!({ "channels": [
            { "id": uuid(21), "type": "board", "name": "Ops", "order_index": 1.0 },
        ]})
        .to_string()
    }

    fn trash() -> String {
        json!({ "channels": [
            { "id": uuid(15), "type": "board", "name": "Old" },
        ]})
        .to_string()
    }

    fn arguments(given: &Value) -> Result<LsArgs> {
        parse(given.as_object().cloned().unwrap_or_default())
    }

    /// Runs `ls` on the replies in order; returns the listing and the paths
    /// with their queries that were asked.
    async fn run(given: &Value, replies: &[String]) -> Result<(Listing, Vec<String>)> {
        let canned = replies
            .iter()
            .map(|body| reply(200, &[("content-type", "application/json")], body));
        let (fake, client) = on_fake(canned, Some(stored("tok", "s=1")))?;
        let listing = ls(&client, &arguments(given)?).await?;
        let asked = fake
            .requests()
            .iter()
            .map(|request| {
                request.url.query().map_or_else(
                    || request.url.path().to_owned(),
                    |query| format!("{}?{query}", request.url.path()),
                )
            })
            .collect();
        Ok((listing, asked))
    }

    /// Each node as `name` indented by its depth, folders ending with `/`.
    fn lines(listing: &Listing) -> Vec<String> {
        listing
            .nodes
            .iter()
            .map(|node| {
                format!(
                    "{}{}{}",
                    "  ".repeat(node.depth),
                    node.name,
                    match node.kind {
                        NodeKind::Folder => "/",
                        NodeKind::Board | NodeKind::Other => "",
                    }
                )
            })
            .collect()
    }

    fn ends_with(asked: &[String], tail: &str) -> usize {
        asked.iter().filter(|path| path.ends_with(tail)).count()
    }

    #[tokio::test]
    async fn a_workspace_that_is_not_yours_is_refused_with_the_list() {
        let refused = run(&json!({ "workspace": "nowhere" }), &[account()])
            .await
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(
            refused.contains("easy") && refused.contains("team"),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn a_folder_of_one_workspace_leaves_the_others_out() -> Result<()> {
        let (listing, _) = run(
            &json!({ "folder": "Plans" }),
            &[account(), easy_tree(), team_tree()],
        )
        .await?;
        assert_eq!(lines(&listing), ["Roadmap"]);
        Ok(())
    }

    #[tokio::test]
    async fn the_root_lists_the_whole_workspace() -> Result<()> {
        let (listing, _) = run(
            &json!({ "folder": "/", "workspace": "team" }),
            &[account(), team_tree()],
        )
        .await?;
        assert_eq!(lines(&listing), ["Ops"]);
        Ok(())
    }

    #[tokio::test]
    async fn a_missing_folder_is_not_found_in_any_workspace() {
        let refused = run(
            &json!({ "folder": "Nowhere" }),
            &[account(), easy_tree(), team_tree()],
        )
        .await
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
        assert!(refused.contains("Nowhere"), "{refused}");
    }

    #[tokio::test]
    async fn both_lists_the_tree_then_the_trash() -> Result<()> {
        let (listing, asked) = run(
            &json!({ "view": "both", "workspace": "easy" }),
            &[account(), easy_tree(), trash()],
        )
        .await?;
        assert_eq!(
            lines(&listing),
            ["Plans/", "  Roadmap", "Retro", "Notes", "Old"]
        );
        assert_eq!(
            (ends_with(&asked, "/tree"), ends_with(&asked, "/trash")),
            (1, 1)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_cursor_that_is_not_ours_is_refused() {
        let refused = run(
            &json!({ "cursor": "abc" }),
            &[account(), easy_tree(), team_tree()],
        )
        .await
        .err()
        .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("cursor of `board ls` must be the cursor a previous page printed")
        );
    }

    #[tokio::test]
    async fn recent_boards_are_per_account_across_workspaces() -> Result<()> {
        let page = json!({ "channels": [
            { "channelId": uuid(21), "date": "2026-10-27T10:00:00Z" },
            { "channelId": uuid(13), "date": "2026-10-27T09:00:00Z" },
            { "channelId": uuid(77), "date": "2026-10-26T09:00:00Z" },
        ], "cursor": "next" })
        .to_string();
        let (listing, asked) = run(
            &json!({ "view": "recent", "workspace": "team", "limit": 3, "cursor": "c0" }),
            &[account(), page, easy_tree(), team_tree()],
        )
        .await?;
        assert_eq!(
            (
                lines(&listing),
                listing.cursor,
                listing
                    .nodes
                    .iter()
                    .map(|node| node.workspace.clone())
                    .collect::<Vec<_>>(),
                asked
                    .iter()
                    .any(|path| path.contains("pageSize=3")
                        && path.contains("cursor=c0")),
                ends_with(&asked, "/tree")
            ),
            (
                vec![String::from("Ops"), String::from("Retro"), String::new()],
                Some(String::from("next")),
                vec![Some(uuid(2)), Some(uuid(1)), None],
                true,
                2
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_folder_is_refused_with_a_view_that_has_none() {
        let refused = run(&json!({ "view": "trash", "folder": "Plans" }), &[])
            .await
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some(
                "folder of `board ls` must be left out with this view; only tree and both look into a folder"
            )
        );
    }
}
