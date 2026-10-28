//! Finds workspaces, boards, folders for [`crate::domain::lookup::choice`].

use crate::{
    command::board_text::load_all_with,
    domain::{
        account::Workspace,
        id::BoardId,
        lookup::choice::{FoundNode, LookupQuery, MatchCoverage, SkippedWorkspaces},
        node::NodeBrief,
        tree::name_index::{build::NameIndex, query::NodeFilter},
    },
    error::{Result, lookup::LookupKind},
    remote::client::Client,
};

/// The current user's workspaces: id and name.
///
/// # Errors
/// Errors of the `current_user` request.
pub async fn workspaces(client: &Client) -> Result<Vec<Workspace>> {
    crate::remote::api::user::current(client)
        .await
        .map(|me| crate::domain::account::account_workspaces(&me))
}

/// A workspace by id or part of its name; without one — the only workspace.
///
/// # Errors
/// As [`workspaces`] and [`crate::domain::lookup::choice::workspace`].
pub async fn workspace(client: &Client, reference: Option<&str>) -> Result<Workspace> {
    crate::domain::lookup::choice::workspace(&workspaces(client).await?, reference)
}

const TREE_CONCURRENCY: usize = 8;

/// Which nodes of a workspace to index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexedNodes {
    Live,
    Trash,
}

/// Trees of several workspaces in parallel, in order; errors stay per tree.
pub async fn trees(
    client: &Client,
    workspaces: Vec<Workspace>,
) -> Vec<(Workspace, Result<NameIndex>)> {
    indexes(client, workspaces, IndexedNodes::Live).await
}

/// Indexes of live trees or trashes of workspaces, in parallel, in order.
pub async fn indexes(
    client: &Client,
    workspaces: Vec<Workspace>,
    nodes: IndexedNodes,
) -> Vec<(Workspace, Result<NameIndex>)> {
    load_all_with(workspaces, TREE_CONCURRENCY, |workspace| {
        let client = client.clone();
        async move {
            match nodes {
                IndexedNodes::Live => {
                    crate::remote::api::workspace::tree_index(&client, &workspace.id)
                        .await
                }
                IndexedNodes::Trash => {
                    crate::remote::api::workspace::trash_index(&client, &workspace.id)
                        .await
                }
            }
        }
    })
    .await
}

/// A board by id, link or name (searched in `workspace_reference` or
/// everywhere).
///
/// # Errors
/// As [`node`].
pub async fn board_id(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
) -> Result<String> {
    node(
        client,
        reference,
        workspace_reference,
        NodeLookup {
            filter: crate::domain::lookup::choice::BOARDS,
            coverage: MatchCoverage::Partial,
        },
    )
    .await
}

/// A board by id, link or whole name only: for what cannot be undone.
///
/// # Errors
/// As [`node`].
pub async fn exact_board_id(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
) -> Result<String> {
    node(
        client,
        reference,
        workspace_reference,
        NodeLookup {
            filter: crate::domain::lookup::choice::BOARDS,
            coverage: MatchCoverage::Exact,
        },
    )
    .await
}

/// Which nodes a name may match and how much of the name it must cover.
#[derive(Debug, Clone, Copy)]
pub struct NodeLookup {
    pub filter: NodeFilter,
    pub coverage: MatchCoverage,
}

/// A node of `selection` by id, link or name, in `workspace_reference` or
/// anywhere.
///
/// # Errors
/// [`crate::domain::lookup::choice::one`],
/// [`crate::domain::lookup::choice::workspace`], request and fatal tree errors.
pub async fn node(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
    selection: NodeLookup,
) -> Result<String> {
    match BoardId::find_in(reference) {
        Some(id) => Ok(id.into()),
        None => by_name(client, reference, workspace_reference, selection).await,
    }
}

async fn by_name(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
    selection: NodeLookup,
) -> Result<String> {
    let all = workspaces(client).await?;
    let targets = match workspace_reference {
        Some(workspace) => vec![crate::domain::lookup::choice::workspace(
            &all,
            Some(workspace),
        )?],
        None => all,
    };
    let (indexes, unread) =
        crate::domain::lookup::choice::partition_reads(trees(client, targets).await)?;
    let search = crate::domain::lookup::choice::ChoiceQuery {
        reference,
        filter: selection.filter,
        matching: selection.coverage,
    };
    crate::domain::lookup::choice::one(&indexes, &unread, search)
}

/// The id of the node `reference` names in one workspace's trash; `shown` lists
/// the candidates of a miss.
///
/// # Errors
/// Errors of the trash request, and a
/// [`crate::error::lookup::LookupError`] of a name that is missing or many.
pub async fn trashed(
    client: &Client,
    workspace_id: &str,
    reference: &str,
    shown: impl FnOnce(&NameIndex, &[FoundNode]) -> crate::error::lookup::Candidates,
) -> Result<String> {
    let index =
        crate::remote::api::workspace::trash_index(client, workspace_id).await?;
    crate::domain::lookup::choice::chosen(
        crate::domain::lookup::choice::trashed(&index, reference),
        LookupQuery {
            what: LookupKind::Trash,
            reference,
        },
        &SkippedWorkspaces::default(),
        |found| shown(&index, found),
    )
}

/// A board named for a whole-document write: its id, name and workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportTarget {
    pub id: String,
    pub name: String,
    pub workspace: Workspace,
}

/// A board by id, link or whole name only, with its name and workspace.
///
/// # Errors
/// As [`exact_board_id`], the board request and [`NodeBrief::of`].
pub async fn exact_board(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
) -> Result<ImportTarget> {
    let id = exact_board_id(client, reference, workspace_reference).await?;
    let node = NodeBrief::of(&crate::remote::api::channel::get(client, &id).await?)?;
    let workspace_id = node
        .workspace_id()
        .ok_or_else(|| super::node::missing_field("workspace_id"))?
        .to_owned();
    let workspace = workspaces(client)
        .await?
        .into_iter()
        .find(|workspace| workspace.id == workspace_id)
        .unwrap_or(Workspace {
            id: workspace_id,
            name: String::new(),
        });
    Ok(ImportTarget {
        name: node.name().to_owned(),
        id,
        workspace,
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use crate::{
        command::lookup::exact_board,
        error::{Error, Result, http::HttpError},
        remote::client::fake::{on_fake, reply, stored},
    };

    #[tokio::test]
    async fn a_board_reply_without_its_workspace_is_refused() -> Result<()> {
        let board = "00000000-0000-4000-8000-0000000000a8";
        let (_, client) = on_fake(
            [reply(200, &[], r#"{"name":"Plans"}"#)],
            Some(stored("t", "sid=1")),
        )?;
        let target = exact_board(&client, board, None).await;
        assert!(
            matches!(
                target,
                Err(Error::Http(HttpError::MissingField {
                    field: "workspace_id",
                    ..
                }))
            ),
            "{target:?}"
        );
        Ok(())
    }
}
