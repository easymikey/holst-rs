//! Plans changes of the tree, once for both shells.

use super::lookup::{self, Fresh};
use crate::{
    domain::{
        id::{BoardId, FolderId, WorkspaceId},
        lookup::choice::{
            self,
            FoundNode,
            LookupQuery,
            ParentReference,
            SkippedWorkspaces,
        },
        node::{
            DeleteMode,
            NewNode,
            NewParent,
            NodeBrief,
            NodeCopy,
            NodeKind,
            Parent,
            RightsCopy,
            copy_target,
        },
        tree::name_index::build::NameIndex,
    },
    effect::request::ApiRequest,
    error::{
        Error,
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        http::HttpError,
        input::InputError,
        lookup::{LookupError, LookupKind, LookupMiss},
    },
    protocol::channel::root_access,
    remote::{api, client::Client},
};

/// The request that creates a board or a folder as the current user. At the
/// root it carries the workspace's default access, read from the account; in a
/// folder the node inherits the folder's.
///
/// # Errors
/// [`crate::error::input::InputError::NotNodeKind`] for a kind the server does
/// not create; errors of the user request.
pub async fn plan_create(client: &Client, new: &NewNode) -> Result<ApiRequest> {
    let author = super::account::account_id(client).await?;
    let account = match new.parent {
        NewParent::Root => Some(api::user::current(client).await?),
        NewParent::Folder(_) => None,
    };
    let root_access = account
        .as_ref()
        .and_then(|account| root_access(account, new.workspace.as_str()));
    api::channel::create_request(&api::channel::CreateChannel::new(
        new,
        &author,
        &client.stamp(),
        root_access,
    )?)
}

/// The folder `reference` names in `workspace`; no reference, or `root`, is the
/// root. `shown` lists several matches of a name.
///
/// # Errors
/// As [`look_up_parent`], and errors of the tree request.
pub async fn parent(
    client: &Client,
    workspace: &WorkspaceId,
    reference: Option<&str>,
    shown: impl FnOnce(&NameIndex, &[FoundNode]) -> crate::error::lookup::Candidates,
) -> Result<NewParent> {
    match choice::parent(workspace.as_str(), reference) {
        ParentReference::Known(Parent::Workspace(_)) => Ok(NewParent::Root),
        ParentReference::Known(Parent::Folder(_))
        | ParentReference::Root(_)
        | ParentReference::Named(_) => look_up_parent(
            &lookup::tree_holding(client, workspace.as_str(), Fresh::Kept, |index| {
                has_parent(index, workspace, reference)
            })
            .await?,
            workspace,
            reference,
            shown,
        ),
    }
}

/// The folder `reference` names in the live tree `index` of `workspace`; no
/// reference, or `root`, is the root.
///
/// # Errors
/// [`crate::error::input::InputError::RootOrFolder`] when a folder is called
/// `root`; a [`LookupError`] of a folder that is missing, in the trash, not
/// one, or named by several.
pub fn look_up_parent(
    index: &NameIndex,
    workspace: &WorkspaceId,
    reference: Option<&str>,
    shown: impl FnOnce(&NameIndex, &[FoundNode]) -> crate::error::lookup::Candidates,
) -> Result<NewParent> {
    match choice::parent(workspace.as_str(), reference) {
        ParentReference::Known(Parent::Workspace(_)) => Ok(NewParent::Root),
        ParentReference::Known(Parent::Folder(id)) => existing_folder(index, &id),
        ParentReference::Root(word) => {
            choice::root_in(index, workspace.as_str(), word).map(|_| NewParent::Root)
        }
        ParentReference::Named(name) => choice::chosen(
            choice::folder(index, name),
            LookupQuery {
                what: LookupKind::Folder,
                reference: name,
            },
            &SkippedWorkspaces::default(),
            |found| shown(index, found),
        )
        .and_then(|id| existing_folder(index, &id)),
    }
}

/// Whether live tree `index` has folder `reference`; the root always does.
fn has_parent(
    index: &NameIndex,
    workspace: &WorkspaceId,
    reference: Option<&str>,
) -> bool {
    match choice::parent(workspace.as_str(), reference) {
        ParentReference::Known(Parent::Folder(id)) => {
            existing_folder(index, &id).is_ok()
        }
        ParentReference::Named(name) => {
            !matches!(choice::folder(index, name), choice::Choice::NotFound)
        }
        ParentReference::Known(Parent::Workspace(_)) | ParentReference::Root(_) => true,
    }
}

fn existing_folder(index: &NameIndex, id: &str) -> Result<NewParent> {
    let folder: FolderId = id.parse()?;
    index
        .find_id(id)
        .filter(|&node| index.kind(node) == NodeKind::Folder && !index.in_trash(node))
        .map(|_| NewParent::Folder(folder))
        .ok_or_else(|| not_found(LookupKind::Folder, id))
}

/// How a board is copied: the name and the folder of the copy, if asked.
#[derive(Debug, Clone, Copy)]
pub struct Copying<'a> {
    /// `<name>-copy` when absent.
    pub name: Option<&'a str>,
    /// Next to the original when absent.
    pub folder: Option<&'a str>,
    pub permissions: RightsCopy,
}

/// The request that copies the board `id`.
///
/// # Errors
/// As [`parent`] for the folder; [`HttpError::MissingField`] when the board's
/// reply has no workspace or place; errors of the requests.
pub async fn plan_copy(
    client: &Client,
    id: &BoardId,
    copying: &Copying<'_>,
    shown: impl FnOnce(&NameIndex, &[FoundNode]) -> crate::error::lookup::Candidates,
) -> Result<ApiRequest> {
    let source = NodeBrief::of(&api::channel::get(client, id.as_str()).await?)?;
    let workspace: WorkspaceId = source
        .workspace_id()
        .ok_or_else(|| missing_field("workspace_id"))?
        .parse()?;
    let folder = match copying.folder {
        Some(reference) => Some(
            parent(client, &workspace, Some(reference), shown)
                .await?
                .parent_in(&workspace),
        ),
        None => None,
    };
    let target = copy_target(&source, copying.name, folder)
        .ok_or_else(|| missing_field("parent_id/parent_type"))?;
    api::channel::duplicate_request(&api::channel::Duplicate::new(
        &NodeCopy {
            source_id: id.as_str(),
            target: &target,
            permissions: copying.permissions,
        },
        &client.stamp(),
    ))
}

pub(super) fn missing_field(field: &'static str) -> Error {
    Error::Http(HttpError::MissingField {
        response: "channel",
        field,
    })
}

fn not_found(kind: LookupKind, reference: &str) -> Error {
    Error::Lookup(LookupError {
        kind,
        reference: reference.to_owned(),
        miss: LookupMiss::not_found(),
    })
}

/// A board or a folder found in the live tree of its workspace.
#[derive(Debug)]
pub struct LocatedNode {
    pub id: BoardId,
    pub workspace: WorkspaceId,
    pub name: String,
    pub kind: NodeKind,
    /// The live boards a folder takes along; 0 for a board.
    pub boards_along: usize,
    index: NameIndex,
}

/// Node `id` in its workspace; a kept tree lacking it or `into` is re-read.
///
/// # Errors
/// A [`LookupError`] when the node is in no live tree, or in the trash;
/// [`HttpError::MissingField`] when its reply has no workspace; errors of the
/// requests.
pub async fn locate(
    client: &Client,
    id: &BoardId,
    into: Option<&str>,
) -> Result<LocatedNode> {
    let workspace: WorkspaceId = match lookup::cached_node(client, id.as_str()) {
        Some(cached) => cached.workspace.id.parse()?,
        None => NodeBrief::of(&api::channel::get(client, id.as_str()).await?)?
            .workspace_id()
            .ok_or_else(|| missing_field("workspace_id"))?
            .parse()?,
    };
    let index =
        lookup::tree_holding(client, workspace.as_str(), Fresh::Kept, |index| {
            index
                .find_id(id.as_str())
                .is_some_and(|node| !index.in_trash(node))
                && has_parent(index, &workspace, into)
        })
        .await?;
    located(id, workspace, index)
}

fn located(
    id: &BoardId,
    workspace: WorkspaceId,
    index: NameIndex,
) -> Result<LocatedNode> {
    let node = index
        .find_id(id.as_str())
        .filter(|&node| !index.in_trash(node))
        .ok_or_else(|| not_found(LookupKind::Node, id.as_str()))?;
    let kind = index.kind(node);
    Ok(LocatedNode {
        id: id.clone(),
        workspace,
        name: index.name(node).to_owned(),
        kind,
        boards_along: Some(kind)
            .filter(|&kind| kind == NodeKind::Folder)
            .map_or(0, |_| index.boards_within(node)),
        index,
    })
}

impl LocatedNode {
    /// The live tree of the node's workspace.
    #[must_use]
    pub const fn index(&self) -> &NameIndex {
        &self.index
    }

    /// The folder `reference` names in this node's workspace, as
    /// [`look_up_parent`].
    ///
    /// # Errors
    /// As [`look_up_parent`]; [`InputError::MoveIntoItself`] for this node or a
    /// folder inside it.
    pub fn parent(
        &self,
        reference: Option<&str>,
        shown: impl FnOnce(&NameIndex, &[FoundNode]) -> crate::error::lookup::Candidates,
    ) -> Result<NewParent> {
        let parent = look_up_parent(&self.index, &self.workspace, reference, shown)?;
        match &parent {
            NewParent::Folder(folder) if self.encloses(folder) => {
                Err(Error::Input(InputError::MoveIntoItself {
                    name: self.name.clone(),
                }))
            }
            NewParent::Folder(_) | NewParent::Root => Ok(parent),
        }
    }

    /// The folder is this node, or has it among its parents.
    fn encloses(&self, folder: &FolderId) -> bool {
        let index = &self.index;
        let this = index.find_id(self.id.as_str());
        std::iter::successors(index.find_id(folder.as_str()), |&up| index.parent(up))
            .take(index.len())
            .any(|up| Some(up) == this)
    }

    /// The refusal to move the boards of a folder along unasked; none for a
    /// board or an empty folder.
    #[must_use]
    pub fn contents_unconfirmed(&self) -> Option<Unconfirmed> {
        Some(self.boards_along)
            .filter(|&boards| boards > 0)
            .map(|boards| {
                let counted = counted(boards);
                Unconfirmed {
                    change: format!("moving a folder with {counted}").into(),
                    risk: ChangeRisk::Contents,
                }
            })
    }

    /// The request that moves the node to the trash.
    ///
    /// # Errors
    /// Errors of building the request.
    pub fn plan_trash(&self) -> Result<ApiRequest> {
        api::channel::delete_request(self.id.as_str(), &DeleteMode::Trash.into())
    }

    /// The request that moves the node to `parent`.
    ///
    /// # Errors
    /// Errors of building the request.
    pub fn plan_move(&self, parent: &NewParent) -> Result<ApiRequest> {
        api::channel::move_to_request(
            self.id.as_str(),
            &api::channel::MoveTo::new(
                &parent.parent_in(&self.workspace),
                self.place_first_in(parent),
            ),
        )
    }

    /// The sidebar place above the first live sibling the node has in `parent`:
    /// the web client drops a node on a folder as its first child.
    fn place_first_in(&self, parent: &NewParent) -> f64 {
        let index = &self.index;
        let folder = match parent {
            NewParent::Root => None,
            NewParent::Folder(folder) => index.find_id(folder.as_str()),
        };
        let this = index.find_id(self.id.as_str());
        let first = index
            .nodes()
            .filter(|&node| {
                Some(node) != this
                    && !index.in_trash(node)
                    && index.parent(node) == folder
            })
            .map(|node| index.order(node))
            .filter(|order| order.is_finite())
            .min_by(f64::total_cmp);
        first.map_or(0.0, |first| first - FIRST_STEP)
    }
}

/// The middle of the client's 12 345 plus 0 to 10 000 above the first sibling.
const FIRST_STEP: f64 = 17_345.0;

fn counted(boards: usize) -> String {
    match boards {
        1 => "1 board".to_owned(),
        boards => format!("{boards} boards"),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use rstest::rstest;
    use serde_json::json;

    use super::*;

    const WORKSPACE: &str = "00000000-0000-4000-8000-000000000001";
    const PLANS: &str = "00000000-0000-4000-8000-0000000000a1";
    const ARCHIVE: &str = "00000000-0000-4000-8000-0000000000a2";
    const SUB: &str = "00000000-0000-4000-8000-0000000000a3";
    const BOARD: &str = "00000000-0000-4000-8000-0000000000b1";
    const ELSEWHERE: &str = "00000000-0000-4000-8000-0000000000ff";

    fn workspace() -> Result<WorkspaceId> {
        WORKSPACE.parse()
    }

    fn index() -> Result<NameIndex> {
        let channel = |id: &str, kind: &str, name: &str, parent: &str, trash: bool| {
            json!({
                "id": id,
                "type": kind,
                "name": name,
                "parent_id": parent,
                "in_trash": trash.then_some("2026-10-01T00:00:00Z"),
                "order_index": 1.0,
            })
        };
        let tree = json!({ "channels": [
            channel(PLANS, "folder", "Plans", WORKSPACE, false),
            channel(ARCHIVE, "folder", "Archive", WORKSPACE, true),
            channel(BOARD, "board", "Roadmap", PLANS, false),
            channel(SUB, "folder", "Sub", PLANS, false),
        ] });
        Ok(NameIndex::parse(&tree.to_string())?.0)
    }

    fn listed(_: &NameIndex, found: &[FoundNode]) -> crate::error::lookup::Candidates {
        crate::error::lookup::Candidates {
            listed: Vec::new(),
            more: found.len(),
        }
    }

    #[rstest]
    #[case::nothing(None, Some(NewParent::Root))]
    #[case::the_word(Some(" Root "), Some(NewParent::Root))]
    #[case::the_workspace(Some(WORKSPACE), Some(NewParent::Root))]
    #[case::a_folder_by_id(Some(PLANS), PLANS.parse().ok().map(NewParent::Folder))]
    #[case::a_folder_by_name(Some("plan"), PLANS.parse().ok().map(NewParent::Folder))]
    #[case::an_unknown_id(Some(ELSEWHERE), None)]
    #[case::an_unknown_name(Some("Nowhere"), None)]
    #[case::a_board_is_no_folder(Some(BOARD), None)]
    #[case::a_trashed_folder(Some(ARCHIVE), None)]
    fn a_parent_is_the_root_or_a_live_folder(
        #[case] reference: Option<&str>,
        #[case] expected: Option<NewParent>,
    ) -> Result<()> {
        let resolved = look_up_parent(&index()?, &workspace()?, reference, listed);
        assert_eq!(resolved.ok(), expected);
        Ok(())
    }

    #[rstest]
    #[case::into_a_folder_with_children(ELSEWHERE, Some(PLANS), 3.0 - FIRST_STEP)]
    #[case::into_an_empty_folder(ELSEWHERE, Some(SUB), 0.0)]
    #[case::to_the_root(BOARD, None, 4.0 - FIRST_STEP)]
    #[case::not_beside_itself(PLANS, None, 8.0 - FIRST_STEP)]
    fn a_moved_node_goes_above_the_first_sibling(
        #[case] moved: &str,
        #[case] folder: Option<&str>,
        #[case] expected: f64,
    ) -> Result<()> {
        let parent = folder
            .map(|folder| folder.parse().map(NewParent::Folder))
            .transpose()?
            .unwrap_or(NewParent::Root);
        let channel = |id: &str, kind: &str, within: &str, order: f64| {
            json!({
                "id": id, "type": kind, "name": id, "parent_id": within,
                "order_index": order,
            })
        };
        let tree = json!({ "channels": [
            channel(PLANS, "folder", WORKSPACE, 4.0),
            channel(ELSEWHERE, "board", WORKSPACE, 8.0),
            channel(BOARD, "board", PLANS, 3.0),
            channel(SUB, "folder", WORKSPACE, 9.0),
        ] });
        let index = NameIndex::parse(&tree.to_string())?.0;
        let node = located(&moved.parse()?, workspace()?, index)?;
        assert!(node.place_first_in(&parent).total_cmp(&expected).is_eq());
        Ok(())
    }

    #[test]
    fn a_node_carries_the_boards_of_its_folder() -> Result<()> {
        let boards_along = |id: &str| {
            located(&id.parse()?, workspace()?, index()?).map(|node| node.boards_along)
        };
        assert_eq!(boards_along(PLANS)?, 1);
        assert_eq!(boards_along(BOARD)?, 0);
        Ok(())
    }

    #[rstest]
    #[case::a_trashed_folder(ARCHIVE)]
    #[case::an_unknown_node(ELSEWHERE)]
    fn a_node_outside_the_live_tree_is_not_found(#[case] id: &str) -> Result<()> {
        let found = located(&id.parse()?, workspace()?, index()?);
        assert!(matches!(
            found,
            Err(Error::Lookup(LookupError {
                miss: LookupMiss::NotFound { .. },
                ..
            }))
        ));
        Ok(())
    }

    #[rstest]
    #[case::folder_moved(PLANS, Some("moving a folder with 1 board"))]
    #[case::empty_folder(SUB, None)]
    #[case::board(BOARD, None)]
    fn a_folder_with_boards_is_not_taken_along_unasked(
        #[case] id: &str,
        #[case] action: Option<&str>,
    ) -> Result<()> {
        let node = located(&id.parse()?, workspace()?, index()?)?;
        let refusal = node
            .contents_unconfirmed()
            .map(|unconfirmed| unconfirmed.change.into_owned());
        assert_eq!(refusal.as_deref(), action);
        Ok(())
    }

    #[rstest]
    #[case::itself(PLANS, PLANS, false)]
    #[case::its_subfolder(PLANS, SUB, false)]
    #[case::its_subfolder_by_name(PLANS, "Sub", false)]
    #[case::the_folder_it_is_in(SUB, PLANS, true)]
    #[case::a_board_into_a_subfolder(BOARD, SUB, true)]
    fn a_folder_is_not_moved_into_itself(
        #[case] moved: &str,
        #[case] destination: &str,
        #[case] allowed: bool,
    ) -> Result<()> {
        let node = located(&moved.parse()?, workspace()?, index()?)?;
        let parent = node.parent(Some(destination), listed);
        assert_eq!(parent.is_ok(), allowed, "{parent:?}");
        assert!(
            allowed
                || matches!(
                    parent,
                    Err(Error::Input(InputError::MoveIntoItself { .. }))
                )
        );
        Ok(())
    }
}
