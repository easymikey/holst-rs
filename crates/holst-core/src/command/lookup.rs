//! Finds workspaces, boards, folders for [`crate::domain::lookup::choice`].

use serde_json::Value;

use crate::{
    command::{account::account, board_text::load_all_with},
    domain::{
        account::Workspace,
        id::BoardId,
        lookup::choice::{
            Choice,
            ChoiceQuery,
            FoundNode,
            LookupQuery,
            MatchCoverage,
            SkippedWorkspaces,
            choose,
        },
        node::NodeBrief,
        tree::name_index::{build::NameIndex, query::NodeFilter},
    },
    error::{
        Error,
        Result,
        lookup::{LookupError, LookupKind, LookupMiss},
    },
    remote::client::Client,
};

/// The account's workspaces and whether the list was kept from an earlier ask.
struct Listing {
    workspaces: Vec<Workspace>,
    kept: bool,
}

async fn listing(client: &Client) -> Result<Listing> {
    match client.known_workspaces() {
        Some(workspaces) => Ok(Listing {
            workspaces,
            kept: true,
        }),
        None => Ok(Listing {
            workspaces: account(client).await?.workspaces,
            kept: false,
        }),
    }
}

fn is_not_found(error: &Error) -> bool {
    matches!(
        error,
        Error::Lookup(LookupError {
            miss: LookupMiss::NotFound { .. },
            ..
        })
    )
}

/// The current user's workspaces: id and name. The kept list is used without a
/// request; otherwise `current_user` is asked and its workspaces are kept.
///
/// # Errors
/// Errors of the `current_user` request.
pub async fn workspaces(client: &Client) -> Result<Vec<Workspace>> {
    listing(client).await.map(|listing| listing.workspaces)
}

/// A workspace by id or part of its name; without one — the only workspace. A
/// reference the kept list does not know is looked for again in a fresh list.
///
/// # Errors
/// As [`workspaces`] and [`crate::domain::lookup::choice::workspace`].
pub async fn workspace(client: &Client, reference: Option<&str>) -> Result<Workspace> {
    let Listing { workspaces, kept } = listing(client).await?;
    match crate::domain::lookup::choice::workspace(&workspaces, reference) {
        Err(error) if kept && is_not_found(&error) => {
            client.forget_workspaces();
            crate::domain::lookup::choice::workspace(
                &listing(client).await?.workspaces,
                reference,
            )
        }
        found => found,
    }
}

const TREE_CONCURRENCY: usize = 8;

/// A tree this old, or older, is asked again by a change that picks by name.
const CHANGE_TREE_AGE: u64 = 3_600_000;

/// Which nodes of a workspace to index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexedNodes {
    Live,
    Trash,
}

/// Whether a tree may come from the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fresh {
    /// The kept copy when there is one (a change: only a young one).
    Kept,
    /// Always a request.
    Asked,
}

/// An index of a tree and, when it came from the cache, when it was read.
struct Tree {
    index: NameIndex,
    kept_at: Option<u64>,
}

fn aged(now: u64, read_at: u64) -> bool {
    now.saturating_sub(read_at) >= CHANGE_TREE_AGE
}

fn kept_tree(
    client: &Client,
    workspace: &str,
    fresh: Fresh,
    changing: bool,
) -> Option<Tree> {
    let kept = client
        .known_tree(workspace)
        .filter(|kept| match (fresh, changing) {
            (Fresh::Asked, _) => false,
            (Fresh::Kept, true) => !aged(client.unix_millis(), kept.read_at),
            (Fresh::Kept, false) => true,
        })?;
    crate::remote::api::workspace::parse_tree(&kept.text)
        .ok()
        .map(|index| Tree {
            index,
            kept_at: Some(kept.read_at),
        })
}

/// A live tree: the kept copy `fresh` allows, else asked and kept.
async fn read_tree(
    client: &Client,
    workspace: &str,
    fresh: Fresh,
    changing: bool,
) -> Result<Tree> {
    match kept_tree(client, workspace, fresh, changing) {
        Some(kept) => Ok(kept),
        None => asked_tree(client, workspace).await,
    }
}

async fn asked_tree(client: &Client, workspace: &str) -> Result<Tree> {
    let started = client.unix_millis();
    let text = crate::remote::api::workspace::tree_text(client, workspace).await?;
    let index = crate::remote::api::workspace::parse_tree(&text)?;
    client.learn_tree(workspace, &text, started);
    Ok(Tree {
        index,
        kept_at: None,
    })
}

/// The live tree of one workspace as `fresh` says; a refused tree is forgotten.
///
/// # Errors
/// Errors of the tree request.
pub async fn tree(client: &Client, workspace: &str, fresh: Fresh) -> Result<NameIndex> {
    tree_holding(client, workspace, fresh, |_| true).await
}

/// As [`tree`]; a kept tree that `hit` says lacks what is wanted is asked once
/// instead, as a node made elsewhere may be missing from it.
///
/// # Errors
/// Errors of the tree request.
pub async fn tree_holding(
    client: &Client,
    workspace: &str,
    fresh: Fresh,
    hit: impl Fn(&NameIndex) -> bool,
) -> Result<NameIndex> {
    let read = match read_tree(client, workspace, fresh, client.runs_change()).await {
        Ok(Tree {
            index,
            kept_at: Some(_),
        }) if !hit(&index) => asked_tree(client, workspace).await,
        read => read,
    };
    match read {
        Err(error) if error.is_board_closed() => {
            client.forget_tree(workspace);
            client.forget_workspaces();
            Err(error)
        }
        Err(error) => Err(error),
        Ok(tree) => Ok(tree.index),
    }
}

/// Trees of several workspaces in parallel, in order; errors stay per tree.
pub async fn trees(
    client: &Client,
    workspaces: Vec<Workspace>,
    fresh: Fresh,
) -> Vec<(Workspace, Result<NameIndex>)> {
    indexes(client, workspaces, IndexedNodes::Live, fresh).await
}

/// Indexes of live trees or trashes of workspaces, in parallel, in order. A
/// trash is never kept, so it is always asked.
pub async fn indexes(
    client: &Client,
    workspaces: Vec<Workspace>,
    nodes: IndexedNodes,
    fresh: Fresh,
) -> Vec<(Workspace, Result<NameIndex>)> {
    read_all(client, workspaces, nodes, |_| fresh, client.runs_change())
        .await
        .into_iter()
        .map(|(workspace, tree)| (workspace, tree.map(|tree| tree.index)))
        .collect()
}

async fn read_all(
    client: &Client,
    workspaces: Vec<Workspace>,
    nodes: IndexedNodes,
    fresh: impl Fn(&Workspace) -> Fresh,
    changing: bool,
) -> Vec<(Workspace, Result<Tree>)> {
    let read = load_all_with(workspaces, TREE_CONCURRENCY, |workspace| {
        let client = client.clone();
        let fresh = fresh(&workspace);
        async move {
            match nodes {
                IndexedNodes::Live => {
                    read_tree(&client, &workspace.id, fresh, changing).await
                }
                IndexedNodes::Trash => {
                    crate::remote::api::workspace::trash_index(&client, &workspace.id)
                        .await
                        .map(|index| Tree {
                            index,
                            kept_at: None,
                        })
                }
            }
        }
    })
    .await;
    read.iter()
        .filter(|(_, tree)| tree.as_ref().is_err_and(Error::is_board_closed))
        .for_each(|(workspace, _)| client.forget_tree(&workspace.id));
    read.iter()
        .any(|(_, tree)| tree.as_ref().is_err_and(Error::is_board_closed))
        .then(|| client.forget_workspaces());
    read
}

/// Forgets the kept tree of one workspace, or of every workspace of the
/// account; the workspace list stays.
pub fn forget_trees(client: &Client, workspace: Option<&str>) {
    match workspace {
        Some(workspace) => client.forget_tree(workspace),
        None => client.forget_all_trees(),
    }
}

/// The kept workspaces that have a kept tree, with its text.
fn kept_trees(client: &Client) -> Vec<(Workspace, String)> {
    client
        .known_workspaces()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|workspace| {
            let text = client.known_tree(&workspace.id)?.text;
            Some((workspace, text))
        })
        .collect()
}

/// Forgets each kept tree whose text holds `id`.
pub fn forget_trees_holding(client: &Client, id: &str) {
    kept_trees(client)
        .into_iter()
        .filter(|(_, text)| text.contains(id))
        .for_each(|(workspace, _)| client.forget_tree(&workspace.id));
}

/// A node as the kept trees show it, with no request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedNode {
    pub workspace: Workspace,
    pub name: String,
    /// The id and name of the folder it sits in.
    pub folder: Option<(String, String)>,
}

/// The node `id` in the kept trees of the kept workspaces.
#[must_use]
pub fn cached_node(client: &Client, id: &str) -> Option<CachedNode> {
    kept_trees(client)
        .into_iter()
        .find_map(|(workspace, text)| {
            let index = crate::remote::api::workspace::parse_tree(&text).ok()?;
            let node = index.find_id(id)?;
            Some(CachedNode {
                workspace,
                name: index.name(node).to_owned(),
                folder: index
                    .parent(node)
                    .map(|up| (index.id(up), index.name(up).to_owned())),
            })
        })
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
    board_id_with(client, reference, workspace_reference, Fresh::Kept).await
}

/// As [`board_id`], with the trees as `fresh` says.
///
/// # Errors
/// As [`node`].
pub async fn board_id_with(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
    fresh: Fresh,
) -> Result<String> {
    resolve(
        client,
        reference,
        workspace_reference,
        NodeLookup {
            filter: crate::domain::lookup::choice::BOARDS,
            coverage: MatchCoverage::Partial,
        },
        fresh,
    )
    .await
    .map(|found| found.id)
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
    resolve(
        client,
        reference,
        workspace_reference,
        selection,
        Fresh::Kept,
    )
    .await
    .map(|found| found.id)
}

/// A node found and the workspaces its search read, when it read any.
struct Resolved {
    id: String,
    workspaces: Option<Vec<Workspace>>,
}

async fn resolve(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
    selection: NodeLookup,
    fresh: Fresh,
) -> Result<Resolved> {
    match BoardId::find_in(reference) {
        Some(id) => Ok(Resolved {
            id: id.into(),
            workspaces: None,
        }),
        None => by_name(client, reference, workspace_reference, selection, fresh).await,
    }
}

async fn by_name(
    client: &Client,
    reference: &str,
    workspace_reference: Option<&str>,
    selection: NodeLookup,
    fresh: Fresh,
) -> Result<Resolved> {
    let search = NameSearch {
        reference,
        selection,
        fresh,
    };
    match workspace_reference {
        Some(named) => search.within(client, named).await,
        None => search.everywhere(client).await,
    }
}

struct NameSearch<'a> {
    reference: &'a str,
    selection: NodeLookup,
    fresh: Fresh,
}

/// Trees read for one choice, with the read time of each one from the cache.
struct Read {
    indexes: Vec<(Workspace, NameIndex)>,
    unread: SkippedWorkspaces,
    kept: Vec<(String, u64)>,
}

/// The ids of the workspaces whose trees hold what `choice` found.
fn holding(indexes: &[(Workspace, NameIndex)], choice: &Choice) -> Vec<String> {
    let positions: Vec<usize> = match choice {
        Choice::Chosen(id) => indexes
            .iter()
            .position(|(_, index)| index.find_id(id).is_some())
            .into_iter()
            .collect(),
        Choice::Uncertain(found) => vec![found.workspace],
        Choice::Ambiguous(found) => found.iter().map(|found| found.workspace).collect(),
        Choice::NotFound => Vec::new(),
    };
    positions
        .into_iter()
        .filter_map(|position| indexes.get(position))
        .map(|(workspace, _)| workspace.id.clone())
        .collect()
}

impl NameSearch<'_> {
    async fn within(&self, client: &Client, named: &str) -> Result<Resolved> {
        let target = workspace(client, Some(named)).await?;
        self.in_workspaces(client, vec![target]).await
    }

    /// Over all workspaces; a name the kept list does not give is looked for
    /// again when a fresh list differs.
    async fn everywhere(&self, client: &Client) -> Result<Resolved> {
        let Listing { workspaces, kept } = listing(client).await?;
        match self.in_workspaces(client, workspaces.clone()).await {
            Err(error) if kept && is_not_found(&error) => {
                match account(client).await?.workspaces {
                    asked if asked == workspaces => Err(error),
                    asked => self.in_workspaces(client, asked).await,
                }
            }
            found => found,
        }
    }

    async fn in_workspaces(
        &self,
        client: &Client,
        targets: Vec<Workspace>,
    ) -> Result<Resolved> {
        let first = self.read(client, &targets, &[]).await?;
        let again = self.again(client, &first);
        let read = match again.as_slice() {
            [] => first,
            _ => self.read(client, &targets, &again).await?,
        };
        crate::domain::lookup::choice::one(&read.indexes, &read.unread, self.query())
            .map(|id| Resolved {
                id,
                workspaces: Some(targets),
            })
    }

    fn query(&self) -> ChoiceQuery<'_> {
        ChoiceQuery {
            reference: self.reference,
            filter: self.selection.filter,
            matching: self.selection.coverage,
        }
    }

    /// The trees of `targets`, those in `asked` from a request. A change takes
    /// the kept trees as they are; `again` decides which are too old.
    async fn read(
        &self,
        client: &Client,
        targets: &[Workspace],
        asked: &[String],
    ) -> Result<Read> {
        let reads = read_all(
            client,
            targets.to_vec(),
            IndexedNodes::Live,
            |workspace| match (asked.contains(&workspace.id), self.fresh) {
                (true, _) => Fresh::Asked,
                (false, fresh) => fresh,
            },
            false,
        )
        .await;
        let kept = reads
            .iter()
            .filter_map(|(workspace, tree)| {
                let read_at = tree.as_ref().ok()?.kept_at?;
                Some((workspace.id.clone(), read_at))
            })
            .collect();
        let (indexes, unread) = crate::domain::lookup::choice::partition_reads(
            reads
                .into_iter()
                .map(|(workspace, tree)| (workspace, tree.map(|tree| tree.index)))
                .collect(),
        )?;
        Ok(Read {
            indexes,
            unread,
            kept,
        })
    }

    /// The workspaces whose kept trees are asked again: all of them when the
    /// name is not found; for a change, the ones holding a match that are too
    /// old.
    fn again(&self, client: &Client, read: &Read) -> Vec<String> {
        let choice = choose(&read.indexes, &read.unread, self.query());
        let now = client.unix_millis();
        match (&choice, client.runs_change()) {
            (Choice::NotFound, _) => {
                read.kept.iter().map(|(id, _)| id.clone()).collect()
            }
            (_, true) => holding(&read.indexes, &choice)
                .into_iter()
                .filter(|id| {
                    read.kept
                        .iter()
                        .any(|(kept, at)| kept == id && aged(now, *at))
                })
                .collect(),
            (_, false) => Vec::new(),
        }
    }
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
    /// The `channel/{id}` reply the board was read from.
    pub channel: Value,
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
    let found = resolve(
        client,
        reference,
        workspace_reference,
        NodeLookup {
            filter: crate::domain::lookup::choice::BOARDS,
            coverage: MatchCoverage::Exact,
        },
        Fresh::Kept,
    )
    .await?;
    let channel = crate::remote::api::channel::get(client, &found.id).await?;
    let node = NodeBrief::of(&channel)?;
    let workspace_id = node
        .workspace_id()
        .ok_or_else(|| super::node::missing_field("workspace_id"))?
        .to_owned();
    let listed = match found.workspaces {
        Some(listed) => listed,
        None => workspaces(client).await?,
    };
    let workspace = listed
        .into_iter()
        .find(|workspace| workspace.id == workspace_id)
        .unwrap_or(Workspace {
            id: workspace_id,
            name: String::new(),
        });
    Ok(ImportTarget {
        name: node.name().to_owned(),
        id: found.id,
        workspace,
        channel,
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

    #[tokio::test]
    async fn a_target_holds_the_board_reply_and_its_workspace_name() -> Result<()> {
        let board = "00000000-0000-4000-8000-0000000000a8";
        let channel = r#"{"name":"Plans","workspace_id":"w1"}"#;
        let user = r#"{"id":"u1","workspaces":[{"id":"w1","name":"easy"}]}"#;
        let (fake, client) = on_fake(
            [reply(200, &[], channel), reply(200, &[], user)],
            Some(stored("t", "sid=1")),
        )?;
        let target = exact_board(&client, board, None).await?;
        assert_eq!(target.workspace.name, "easy");
        assert_eq!(
            target
                .channel
                .get("name")
                .and_then(serde_json::Value::as_str),
            Some("Plans")
        );
        assert_eq!(fake.requests().len(), 2);
        Ok(())
    }
}
