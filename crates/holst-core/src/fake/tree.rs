//! The folders and boards of the workspaces and the endpoints that change them.

mod recent;

use serde_json::{Map, Value, json};
use yrs::{Doc, Text as _, Transact as _};

use self::recent::Recents;
use crate::{
    domain::{
        board::{note_root, note_text},
        change::fill_blocks,
        id_source::SequentialIds,
        rich_text::{TextFormat, lines},
    },
    fake::{
        Fake,
        FixtureError,
        FixtureObject,
        board::{
            Board,
            add_objects,
            copied_document,
            fixture_document,
            fixture_document_v3,
            objects_json,
        },
        fixture::unknown,
        http::{Parts, Request, Response, found, refused},
    },
};

/// A board or a folder as a test sees it; the parent is a folder's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedNode {
    pub name: String,
    pub kind: String,
    pub parent: Option<String>,
}

/// A node the fake holds when it starts.
#[derive(Debug, Clone)]
pub(crate) struct FixtureNode {
    pub(crate) workspace: String,
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) folder: Option<String>,
    pub(crate) objects: Vec<FixtureObject>,
    pub(crate) read_only: bool,
    /// The name of the person who owns the node; the user when `None`.
    pub(crate) owner: Option<String>,
    /// Tells whether the user got in by an invitation.
    pub(crate) invited: bool,
    pub(crate) storage: Storage,
}

#[derive(Debug)]
struct Node {
    id: String,
    name: String,
    kind: String,
    workspace_id: String,
    parent_id: String,
    parent_type: String,
    objects: Vec<FixtureObject>,
    // Shared with the sockets that serve and change the board.
    document: Doc,
    read_only: bool,
    trashed: bool,
    owner_id: String,
    invited: bool,
    cover: Cover,
    storage: Storage,
}

impl Node {
    /// A node of `kind` in the `workspace_id`, under `(parent_type,
    /// parent_id)`, owned by nobody, holding no objects.
    fn new(
        document: Doc,
        id: String,
        name: String,
        kind: String,
        workspace_id: String,
        (parent_type, parent_id): (&str, String),
    ) -> Self {
        Self {
            id,
            name,
            kind,
            workspace_id,
            parent_id,
            parent_type: parent_type.to_owned(),
            objects: Vec::new(),
            document,
            read_only: false,
            trashed: false,
            owner_id: String::new(),
            invited: false,
            cover: Cover::default(),
            storage: Storage::default(),
        }
    }
}

/// The cover of a board: its thumbnail and how it was chosen.
#[derive(Debug, Default)]
struct Cover {
    thumbnail: Option<String>,
    fixed: bool,
    color: Option<String>,
}

/// The format of the board's document and the server that holds it.
#[derive(Debug, Clone)]
pub(crate) struct Storage {
    pub(crate) schema_version: u32,
    pub(crate) provider: &'static str,
}

impl Storage {
    /// The document of a board stored so, holding `objects`.
    fn document(&self, board: &str, objects: &[FixtureObject]) -> Doc {
        match self.schema_version {
            3 => fixture_document_v3(board, objects),
            _ => fixture_document(board, objects),
        }
    }
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            schema_version: 2,
            provider: "hud",
        }
    }
}

const TRASHED_AT: &str = "2026-10-01T00:00:00Z";

impl Node {
    fn channel(&self) -> Value {
        json!({
            "id": self.id,
            "type": self.kind,
            "name": self.name,
            "owner_id": self.owner_id,
            "author_id": self.owner_id,
            "workspace_id": self.workspace_id,
            "parent_id": self.parent_id,
            "parent_type": self.parent_type,
            "in_trash": self.trashed.then_some(TRASHED_AT),
            "thumbnail": self.cover.thumbnail,
            "thumbnail_fixed": self.cover.fixed,
            "thumbnail_color": self.cover.color,
            "schema_version": self.storage.schema_version,
            "server_provider_type": self.storage.provider,
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Tree {
    nodes: Vec<Node>,
    user_id: String,
    // The ids of the favorites, in the order the sidebar shows them.
    favorites: Vec<String>,
    // The ids of the boards the user ordered in the sidebar, first to last.
    sidebar: Vec<String>,
    /// Where the client placed the nodes it moved, by id.
    placed: std::collections::HashMap<String, f64>,
    recent: Recents,
}

impl Tree {
    /// Fills the nodes in order; `workspaces` maps a workspace name to its id,
    /// `people` a person's name to their id, and `user_id` is the signed-in
    /// user's.
    ///
    /// # Errors
    /// [`FixtureError`] when a node names a workspace, a folder or an owner the
    /// fixture does not hold before it.
    pub(crate) fn fixture(
        nodes: &[FixtureNode],
        workspaces: &[(String, String)],
        user_id: &str,
        people: &[(String, String)],
    ) -> Result<Self, FixtureError> {
        let start = Self {
            user_id: user_id.to_owned(),
            ..Self::default()
        };
        nodes.iter().try_fold(start, |mut tree, fixture| {
            let owner_id = fixture
                .owner
                .as_ref()
                .map(|name| {
                    people
                        .iter()
                        .find(|(known, _)| known == name)
                        .map(|(_, id)| id.clone())
                        .ok_or_else(|| unknown("person", name))
                })
                .transpose()?
                .unwrap_or_else(|| user_id.to_owned());
            let workspace_id = workspaces
                .iter()
                .find(|(name, _)| *name == fixture.workspace)
                .map(|(_, id)| id.clone())
                .ok_or_else(|| unknown("workspace", &fixture.workspace))?;
            let parent = fixture
                .folder
                .as_ref()
                .map(|name| {
                    tree.nodes
                        .iter()
                        .find(|node| node.kind == "folder" && node.name == *name)
                        .map(|node| node.id.clone())
                        .ok_or_else(|| unknown("folder", name))
                })
                .transpose()?;
            let id =
                format!("00000000-0000-4000-8000-{:012x}", 0xa000 + tree.nodes.len());
            let parent_type = parent.as_ref().map_or("workspace", |_| "channel");
            tree.nodes.push(Node {
                read_only: fixture.read_only,
                objects: fixture.objects.clone(),
                owner_id,
                invited: fixture.invited,
                storage: fixture.storage.clone(),
                ..Node::new(
                    fixture.storage.document(&id, &fixture.objects),
                    id,
                    fixture.name.clone(),
                    fixture.kind.to_owned(),
                    workspace_id.clone(),
                    (parent_type, parent.unwrap_or(workspace_id)),
                )
            });
            Ok(tree)
        })
    }

    /// The live nodes of the workspace `workspace_id`, or the ones in its
    /// trash, in the order they came.
    pub(crate) fn nodes(&self, workspace_id: &str, trashed: bool) -> Vec<ListedNode> {
        self.nodes
            .iter()
            .filter(|node| node.workspace_id == workspace_id && node.trashed == trashed)
            .map(|node| ListedNode {
                name: node.name.clone(),
                kind: node.kind.clone(),
                parent: self
                    .nodes
                    .iter()
                    .find(|parent| parent.id == node.parent_id)
                    .map(|parent| parent.name.clone()),
            })
            .collect()
    }

    pub(crate) fn favorites(&self, workspace_id: &str) -> Vec<String> {
        self.favorite_nodes(workspace_id)
            .map(|node| node.name.clone())
            .collect()
    }

    /// The id of the first node called `name`.
    pub(crate) fn id_of(&self, name: &str) -> Option<String> {
        self.nodes
            .iter()
            .find(|node| node.name == name)
            .map(|node| node.id.clone())
    }

    pub(crate) fn board(&self, id: &str) -> Option<Board> {
        self.find(id)
            .filter(|node| node.kind == "board")
            .map(|node| Board {
                id: node.id.clone(),
                document: node.document.clone(),
                read_only: node.read_only,
            })
    }

    /// Answers a download of the stored file `name`: from an image or a file
    /// of a board.
    pub(crate) fn stored(&self, name: &str) -> Option<Response> {
        self.nodes
            .iter()
            .flat_map(|node| node.objects.iter())
            .find_map(|object| object.stored().filter(|(stored, _)| *stored == name))
            .map(|(_, served)| {
                if served {
                    // A stored SVG is a picture a client can embed.
                    let body = if std::path::Path::new(name)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
                    {
                        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"
                    } else {
                        "pixels"
                    };
                    Response {
                        status: 200,
                        body: body.to_owned(),
                        cookie: None,
                    }
                } else {
                    refused(403, "forbidden")
                }
            })
    }

    /// Adds `objects` to the board called `name`, numbered after the ones it
    /// holds; tells whether there is such a board.
    pub(crate) fn append(&mut self, name: &str, objects: Vec<FixtureObject>) -> bool {
        self.nodes
            .iter_mut()
            .find(|node| node.name == name && node.kind == "board")
            .map(|node| {
                let held = u32::try_from(node.objects.len()).unwrap_or_default();
                add_objects(&node.document, &node.id, held, &objects);
                node.objects.extend(objects);
            })
            .is_some()
    }

    pub(crate) fn objects_json(&self, id: &str) -> Vec<Value> {
        self.board(id)
            .map(|board| objects_json(&board.document))
            .unwrap_or_default()
    }

    /// Answers the tree endpoints; none for any other request.
    pub(crate) fn respond(
        &mut self,
        request: &Request,
        workspace_ids: &[&str],
    ) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "workspace", workspace_id, "tree"])
                if workspace_ids.contains(workspace_id) =>
            {
                Some(self.channels(workspace_id, false))
            }
            ("GET", ["", "gapi", "workspace", workspace_id, "trash"])
                if workspace_ids.contains(workspace_id) =>
            {
                Some(self.channels(workspace_id, true))
            }
            (
                "GET",
                [
                    "",
                    "gapi",
                    "workspace",
                    workspace_id,
                    "created_channels_count",
                ],
            ) if workspace_ids.contains(workspace_id) => {
                let user = parts.query("userId").unwrap_or_default();
                let count = self
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.workspace_id == *workspace_id
                            && node.kind == "board"
                            && !node.trashed
                            && node.owner_id == user
                    })
                    .count();
                Some(found(&json!({ "count": count })))
            }
            ("GET", ["", "gapi", "workspace", _, "tree" | "trash"]) => {
                Some(refused(404, "workspace-not-found"))
            }
            ("GET", ["", "gapi", "channel", id]) => Some(self.find(id).map_or_else(
                || refused(404, "not-found"),
                |node| found(&node.channel()),
            )),
            ("POST", ["", "gapi", "channel", "create"]) => {
                Some(self.create(&request.body, workspace_ids))
            }
            ("POST", ["", "gapi", "channel", "duplicate"]) => {
                Some(self.copy(&request.body))
            }
            ("POST", ["", "gapi", "channel", "restore", id]) => {
                Some(self.restore(id, &request.body))
            }
            ("POST", ["", "gapi", "channel", id, "move" | "delete"])
            | ("PATCH", ["", "gapi", "channel", id])
                if self.find(id).is_some_and(|node| node.read_only) =>
            {
                Some(refused(403, "forbidden"))
            }
            ("POST", ["", "gapi", "channel", id, "move"]) => {
                Some(self.relocate(id, &request.body))
            }
            ("POST", ["", "gapi", "channel", id, "delete"]) => {
                Some(self.delete(id, &request.body))
            }
            ("PATCH", ["", "gapi", "channel", id]) => {
                Some(self.rename(id, &request.body))
            }
            ("POST", ["", "gapi", "favorite_channel", "add"]) => {
                Some(self.favorite(&request.body))
            }
            ("DELETE", ["", "gapi", "favorite_channel", "delete"]) => {
                Some(self.unfavorite(&request.body))
            }
            ("GET", ["", "gapi", "favorite_channel", "channels", workspace_id]) => {
                Some(if workspace_ids.contains(workspace_id) {
                    let channels: Vec<Value> = self
                        .favorite_nodes(workspace_id)
                        .filter(|node| !node.trashed)
                        .map(Node::channel)
                        .collect();
                    found(&Value::Array(channels))
                } else {
                    refused(404, "workspace-not-found")
                })
            }
            ("GET", ["", "gapi", "channel_user_order_index", workspace_id, "list"]) => {
                Some(self.sidebar_order(workspace_id, workspace_ids))
            }
            ("POST", ["", "gapi", "recent", "set"]) => Some(self.visit(&request.body)),
            ("GET", ["", "gapi", "recent", "channels", "page"]) => {
                Some(self.recent_page(&parts))
            }
            ("POST", ["", "gapi", "channel", id, "leave"]) => Some(self.leave(id)),
            ("POST", ["", "gapi", "channel", id, "transfer_ownership"]) => {
                Some(self.transfer(id, &request.body))
            }
            _ => None,
        }
    }

    /// The boards the user ordered in the sidebar of a workspace.
    fn sidebar_order(&self, workspace_id: &str, workspace_ids: &[&str]) -> Response {
        if workspace_ids.contains(&workspace_id) {
            let entries: Vec<Value> = self
                .sidebar
                .iter()
                .filter(|id| {
                    self.find(id).is_some_and(|node| {
                        node.workspace_id == workspace_id && !node.trashed
                    })
                })
                .enumerate()
                .map(|(place, id)| json!({ "channel_id": id, "order_index": place }))
                .collect();
            found(&Value::Array(entries))
        } else {
            refused(404, "workspace-not-found")
        }
    }

    fn favorite_nodes<'a>(
        &'a self,
        workspace_id: &'a str,
    ) -> impl Iterator<Item = &'a Node> {
        self.favorites
            .iter()
            .filter_map(|id| self.find(id))
            .filter(move |node| node.workspace_id == workspace_id)
    }

    fn find(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    fn folder_exists(&self, id: &str) -> bool {
        self.find(id)
            .is_some_and(|node| node.kind == "folder" && !node.trashed)
    }

    /// The id of the node and of everything inside it.
    fn within(&self, id: &str) -> Vec<String> {
        std::iter::once(id.to_owned())
            .chain(
                self.nodes
                    .iter()
                    .filter(|node| node.parent_id == id)
                    .flat_map(|node| self.within(&node.id)),
            )
            .collect()
    }

    fn channels(&self, workspace_id: &str, trashed: bool) -> Response {
        let channels: Vec<Value> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.workspace_id == workspace_id && node.trashed == trashed
            })
            .map(|(position, node)| {
                json!({
                    "id": node.id,
                    "type": node.kind,
                    "name": node.name,
                    "parent_id": node.parent_id,
                    "in_trash": node.trashed.then_some(TRASHED_AT),
                    "order_index": self
                        .placed
                        .get(&node.id)
                        .map_or(json!(position), |place| json!(place)),
                })
            })
            .collect();
        found(&json!({ "channels": channels }))
    }

    fn create(&mut self, body: &str, workspace_ids: &[&str]) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        let node = (|| {
            let id = text("id")?;
            Some(Node {
                owner_id: self.user_id.clone(),
                ..Node::new(
                    fixture_document(&id, &[]),
                    id,
                    text("name")?,
                    text("type")?,
                    text("workspace_id")?,
                    (&text("parent_type")?, text("parent_id")?),
                )
            })
        })();
        match node {
            Some(node)
                if workspace_ids.contains(&node.workspace_id.as_str())
                    && (node.parent_type == "workspace"
                        || self.folder_exists(&node.parent_id)) =>
            {
                let reply = found(&json!({ "result": node.channel() }));
                self.nodes.push(node);
                reply
            }
            Some(_) => refused(404, "not-found"),
            None => refused(400, "bad-request"),
        }
    }

    fn relocate(&mut self, id: &str, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let parent = body.get("newParentId").and_then(Value::as_str);
        let place = body.get("newIndex").and_then(Value::as_f64);
        let target = parent.map_or_else(
            || {
                self.find(id)
                    .map(|node| (node.workspace_id.clone(), "workspace"))
            },
            |folder| {
                self.folder_exists(folder)
                    .then(|| (folder.to_owned(), "channel"))
            },
        );
        match (
            place,
            target,
            self.nodes.iter_mut().find(|node| node.id == id),
        ) {
            (None, ..) => refused(400, "bad-request"),
            (Some(place), Some((parent_id, parent_type)), Some(node)) => {
                node.parent_id = parent_id;
                parent_type.clone_into(&mut node.parent_type);
                let reply = found(&json!({ "updatedChannel": node.channel() }));
                self.placed.insert(id.to_owned(), place);
                reply
            }
            _ => refused(404, "not-found"),
        }
    }

    fn delete(&mut self, id: &str, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let permanently =
            body.get("permanently").and_then(Value::as_bool) == Some(true);
        let affected = self.within(id);
        match self.find(id) {
            // Only the node goes; what was inside moves up and is answered.
            Some(gone) if permanently => {
                let (parent_id, parent_type) =
                    (gone.parent_id.clone(), gone.parent_type.clone());
                let moved: Vec<String> = self
                    .nodes
                    .iter_mut()
                    .filter(|node| node.parent_id == id)
                    .map(|node| {
                        node.parent_id.clone_from(&parent_id);
                        node.parent_type.clone_from(&parent_type);
                        node.id.clone()
                    })
                    .collect();
                self.nodes.retain(|node| node.id != id);
                found(&json!({ "updatedChannelIds": moved }))
            }
            Some(_) => {
                self.nodes
                    .iter_mut()
                    .filter(|node| affected.contains(&node.id))
                    .for_each(|node| node.trashed = true);
                found(&json!({ "updatedChannelIds": affected }))
            }
            None => refused(404, "not-found"),
        }
    }

    fn restore(&mut self, id: &str, body: &str) -> Response {
        let affected = self.within(id);
        match self.find(id) {
            _ if serde_json::from_str::<Value>(body).is_err() => {
                refused(400, "bad-request")
            }
            Some(_) => {
                self.nodes
                    .iter_mut()
                    .filter(|node| affected.contains(&node.id))
                    .for_each(|node| node.trashed = false);
                found(&json!({}))
            }
            None => refused(404, "not-found"),
        }
    }

    /// Renames a node or changes its cover; the body holds the fields to
    /// change.
    fn rename(&mut self, id: &str, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let name = body.get("name").and_then(Value::as_str);
        // The access keys are kept by `Access::shown_on`.
        let covered = [
            "thumbnail",
            "thumbnail_fixed",
            "thumbnail_color",
            "public",
            "public_access",
            "workspace_member_access",
            "share_as_template",
        ]
        .iter()
        .any(|field| body.get(*field).is_some());
        match (
            name.is_some() || covered,
            self.nodes.iter_mut().find(|node| node.id == id),
        ) {
            (true, Some(node)) => {
                if let Some(name) = name {
                    name.clone_into(&mut node.name);
                }
                let old = std::mem::take(&mut node.cover);
                node.cover = Cover {
                    thumbnail: body
                        .get("thumbnail")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(old.thumbnail),
                    fixed: body
                        .get("thumbnail_fixed")
                        .and_then(Value::as_bool)
                        .unwrap_or(old.fixed),
                    color: body
                        .get("thumbnail_color")
                        .map_or(old.color, |color| color.as_str().map(str::to_owned)),
                };
                found(&node.channel())
            }
            (false, Some(_)) => refused(400, "bad-request"),
            (_, None) => refused(404, "not-found"),
        }
    }

    /// Gives boards the covers and the notes the fixture holds, and the sidebar
    /// its order, by board name.
    ///
    /// # Errors
    /// [`FixtureError`] when one names a board the fixture does not hold.
    pub(crate) fn settle(
        &mut self,
        covers: &[(String, String)],
        notes: &[(String, String)],
        sidebar: &[String],
    ) -> Result<(), FixtureError> {
        covers.iter().try_for_each(|(name, thumbnail)| {
            self.nodes
                .iter_mut()
                .find(|node| node.name == *name && node.kind == "board")
                .map(|node| {
                    node.cover = Cover {
                        thumbnail: Some(thumbnail.clone()),
                        fixed: true,
                        color: None,
                    };
                })
                .ok_or_else(|| unknown("board", name))
        })?;
        notes.iter().try_for_each(|(name, text)| {
            self.nodes
                .iter()
                .find(|node| node.name == *name && node.kind == "board")
                .map(|node| write_note(&node.document, text))
                .ok_or_else(|| unknown("board", name))
        })?;
        self.sidebar = sidebar
            .iter()
            .map(|name| {
                self.nodes
                    .iter()
                    .find(|node| node.name == *name)
                    .map(|node| node.id.clone())
                    .ok_or_else(|| unknown("board", name))
            })
            .collect::<Result<_, _>>()?;
        Ok(())
    }

    fn favorite(&mut self, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let id = body.get("channelId").and_then(Value::as_str);
        match id.and_then(|id| self.find(id)).map(|node| node.id.clone()) {
            Some(id) => {
                let index = body.get("index").and_then(Value::as_i64).unwrap_or(-1);
                self.favorites.retain(|known| *known != id);
                // -1, or any place past the end, is the end.
                let place = usize::try_from(index)
                    .map_or(self.favorites.len(), |at| at.min(self.favorites.len()));
                self.favorites.insert(place, id.clone());
                found(
                    &json!({ "channelId": id, "createdAt": TRASHED_AT, "index": index }),
                )
            }
            None => refused(404, "not-found"),
        }
    }

    fn unfavorite(&mut self, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let id = body.get("channelId").and_then(Value::as_str);
        match id.filter(|id| self.find(id).is_some()) {
            Some(id) => {
                self.favorites.retain(|known| known != id);
                found(&json!({}))
            }
            None => refused(404, "not-found"),
        }
    }

    /// The owner gives the node, and with `transferOwnedSubtree` what they
    /// own inside it, to another person.
    fn transfer(&mut self, id: &str, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let target = body.get("newOwnerUserId").and_then(Value::as_str);
        let subtree = body
            .get("transferOwnedSubtree")
            .and_then(Value::as_bool)
            .unwrap_or_default();
        match (self.find(id), target) {
            (None, _) => refused(404, "not-found"),
            (Some(node), _) if node.owner_id != self.user_id => {
                refused(403, "forbidden")
            }
            (Some(_), None) => refused(400, "bad-request"),
            (Some(_), Some(new_owner)) => {
                let moved: Vec<String> = self
                    .within(id)
                    .into_iter()
                    .filter(|inside| subtree || inside == id)
                    .collect();
                let former = self.user_id.clone();
                self.nodes
                    .iter_mut()
                    .filter(|node| moved.contains(&node.id) && node.owner_id == former)
                    .for_each(|node| new_owner.clone_into(&mut node.owner_id));
                found(&json!({}))
            }
        }
    }

    pub(crate) fn owner_of(&self, id: &str) -> Option<String> {
        self.find(id).map(|node| node.owner_id.clone())
    }

    /// The user leaves a node they were invited to; it is gone for them.
    fn leave(&mut self, id: &str) -> Response {
        match self.find(id) {
            None => refused(404, "not-found"),
            Some(node) if !node.invited => refused(403, "forbidden"),
            Some(_) => {
                let gone = self.within(id);
                self.nodes.retain(|node| !gone.contains(&node.id));
                found(&json!({}))
            }
        }
    }

    fn copy(&mut self, body: &str) -> Response {
        let body: Value = serde_json::from_str(body).unwrap_or_default();
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        let node = (|| {
            let source = self.find(&text("source_channel_id")?)?;
            let id = text("channel_id")?;
            Some(Node {
                objects: source.objects.clone(),
                owner_id: self.user_id.clone(),
                ..Node::new(
                    copied_document(&source.document),
                    id,
                    text("name")?,
                    source.kind.clone(),
                    source.workspace_id.clone(),
                    (&text("parent_type")?, text("parent_id")?),
                )
            })
        })();
        match node {
            Some(node)
                if node.parent_type == "workspace"
                    || self.folder_exists(&node.parent_id) =>
            {
                let reply = found(&json!({ "result": node.channel() }));
                self.nodes.push(node);
                reply
            }
            Some(_) => refused(404, "not-found"),
            None => refused(400, "bad-request"),
        }
    }
}

/// Replaces the note of the document with `text`, a paragraph to a line.
fn write_note(document: &Doc, text: &str) {
    let note = note_root(document);
    let mut txn = document.transact_mut();
    let held = note.len(&txn);
    note.remove_range(&mut txn, 0, held);
    fill_blocks(&mut txn, &note, &lines(text), &SequentialIds::new("note"));
}

impl Fake {
    /// Returns the cover fields of the board called `board` as its channel
    /// holds them (`thumbnail`, `thumbnail_fixed`, `thumbnail_color`); empty
    /// when there is no such board.
    #[must_use]
    pub fn cover_of(&self, board: &str) -> Map<String, Value> {
        let state = self.locked();
        match state
            .tree
            .id_of(board)
            .and_then(|id| state.tree.find(&id).map(Node::channel))
        {
            Some(Value::Object(mut fields)) => {
                fields.retain(|name, _| name.starts_with("thumbnail"));
                fields
            }
            Some(_) | None => Map::new(),
        }
    }

    /// Returns the note of the board called `board`, as its document holds it
    /// now; none when there is no such board.
    #[must_use]
    pub fn note_of(&self, board: &str) -> Option<String> {
        let state = self.locked();
        let document = state
            .tree
            .id_of(board)
            .and_then(|id| state.tree.board(&id))?
            .document;
        Some(note_text(&document, TextFormat::Plain))
    }
}
