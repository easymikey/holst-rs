//! Choosing a workspace, a board or a folder by id or part of a name.

#[cfg(test)] mod tests;

use std::{borrow::Borrow, cmp::Reverse, collections::BTreeMap};

pub use crate::error::lookup::{SkipReason, SkippedWorkspaces};
use crate::{
    domain::{
        account::Workspace,
        id::BoardId,
        limit::Limit,
        lookup::rules::{
            LookupMatch,
            MatchOrder,
            MatchRules,
            NAME_ORDER,
            Reference,
            WHOLE_NAME_ORDER,
            WORKSPACE_ORDER,
            find_workspace,
        },
        node::{NodeKind, Parent},
        tree::name_index::{
            build::{NameIndex, NodeIndex},
            query::NodeFilter,
        },
    },
    error::{
        Error,
        Result,
        input::InputError,
        lookup,
        lookup::{Candidate, Candidates, LookupError, Skipped},
    },
};

/// A workspace by id or part of its name; without a `reference`, the only one.
///
/// # Errors
///
/// [`crate::error::input::InputError::NoWorkspaces`], `WorkspaceNotChosen`, or
/// a [`crate::error::lookup::LookupError`] of a workspace.
///
/// ```
/// use holst_core::domain::{account::Workspace, lookup::choice::workspace};
///
/// let named = |id: &str, name: &str| Workspace {
///     id: id.into(),
///     name: name.into(),
/// };
/// let all = [named("w1", "easy"), named("w2", "Design team")];
/// assert_eq!(workspace(&all, Some("design"))?.id, "w2");
/// assert!(workspace(&all, None).is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
pub fn workspace(all: &[Workspace], reference: Option<&str>) -> Result<Workspace> {
    reference
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map_or_else(
            || only_workspace(all),
            |reference| named_workspace(all, reference),
        )
}

fn only_workspace(all: &[Workspace]) -> Result<Workspace> {
    match all {
        [single] => Ok(single.clone()),
        [] => Err(Error::Input(InputError::NoWorkspaces)),
        many => Err(Error::Input(InputError::WorkspaceNotChosen {
            workspaces: named(many.iter()),
        })),
    }
}

fn named_workspace(all: &[Workspace], reference: &str) -> Result<Workspace> {
    let failed = |miss| {
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Workspace,
            reference: reference.to_owned(),
            miss,
        }))
    };
    match find_workspace(all, &Reference::workspace(reference), &WORKSPACE_ORDER) {
        LookupMatch::One(workspace) => Ok(workspace.clone()),
        LookupMatch::NotFound => failed(lookup::LookupMiss::NotFound {
            unread: SkippedWorkspaces::default(),
            available: named(all.iter()),
        }),
        LookupMatch::Several(many) => {
            failed(lookup::LookupMiss::ambiguous(named(many.iter().copied())))
        }
    }
}

impl SkippedWorkspaces {
    /// The same unread workspaces plus `workspace`, not read because of
    /// `error`.
    #[must_use]
    pub fn with(self, workspace: &Workspace, error: &Error) -> Self {
        Self(
            self.0
                .into_iter()
                .chain([Skipped {
                    workspace: workspace.name.clone(),
                    reason: SkipReason::from(error),
                    status: error.status(),
                }])
                .collect(),
        )
    }
}

/// Trees that were read, and the workspaces that were not.
pub type TreeReads<T> = (Vec<(Workspace, T)>, SkippedWorkspaces);

/// Split per-workspace results into read trees and unread workspaces.
///
/// # Errors
///
/// The first fatal error ([`Error::is_fatal`]) among the results.
pub fn partition_reads<T>(
    results: Vec<(Workspace, Result<T>)>,
) -> Result<TreeReads<T>> {
    let init = (
        Vec::with_capacity(results.len()),
        SkippedWorkspaces::default(),
    );
    results
        .into_iter()
        .try_fold(init, |(mut ok, unread), (workspace, result)| match result {
            Ok(tree) => {
                ok.push((workspace, tree));
                Ok((ok, unread))
            }
            Err(error) if error.is_fatal() => Err(error),
            Err(error) => Ok((ok, unread.with(&workspace, &error))),
        })
}

/// How much of a name a reference must cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchCoverage {
    Partial,
    Exact,
}

impl MatchCoverage {
    const fn order(self) -> &'static MatchOrder {
        match self {
            MatchCoverage::Partial => &NAME_ORDER,
            MatchCoverage::Exact => &WHOLE_NAME_ORDER,
        }
    }
}

/// A matching node: which of the searched indexes holds it, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoundNode {
    pub workspace: usize,
    pub node: NodeIndex,
}

/// What a choice by name ends in; each surface words the failures itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Chosen(String),
    Uncertain(FoundNode),
    NotFound,
    Ambiguous(Vec<FoundNode>),
}

pub const BOARDS: NodeFilter = NodeFilter {
    kind: Some(NodeKind::Board),
    in_trash: false,
};

pub const NODES: NodeFilter = NodeFilter {
    kind: None,
    in_trash: false,
};

pub const FOLDERS: NodeFilter = NodeFilter {
    kind: Some(NodeKind::Folder),
    in_trash: false,
};

pub const TRASH: NodeFilter = NodeFilter {
    kind: None,
    in_trash: true,
};

pub const LISTED: usize = 10;

/// What a choice looks for: the reference, the nodes it may name and how well.
#[derive(Debug, Clone, Copy)]
pub struct ChoiceQuery<'a> {
    pub reference: &'a str,
    pub filter: NodeFilter,
    pub matching: MatchCoverage,
}

/// What a failed choice reports as sought: the kind and the reference.
#[derive(Debug, Clone, Copy)]
pub struct LookupQuery<'a> {
    pub what: lookup::LookupKind,
    pub reference: &'a str,
}

fn matches(
    indexes: &[&NameIndex],
    reference: &str,
    filter: NodeFilter,
    matching: MatchCoverage,
) -> Vec<FoundNode> {
    let ranked = indexes
        .iter()
        .enumerate()
        .flat_map(|(position, index)| {
            let (hits, _) = index.search(reference, filter, Limit::All);
            hits.into_iter().map(move |hit| (position, hit))
        })
        .enumerate()
        .map(|(seen, (position, hit))| {
            let found = FoundNode {
                workspace: position,
                node: hit.node,
            };
            ((Reverse(hit.score), seen), (found, hit.score))
        })
        .collect::<BTreeMap<_, _>>()
        .into_values();
    let kind = |found: &FoundNode| {
        indexes
            .get(found.workspace)
            .map(|index| index.kind(found.node))
    };
    matching.order().strongest(
        ranked.map(|(found, score)| (found, kind(&found), MatchRules::of_score(score))),
    )
}

/// The one node `reference` names; only a lone match can be uncertain.
///
/// ```
/// use holst_core::domain::{
///     account::Workspace,
///     lookup::choice::{
///         BOARDS,
///         Choice,
///         ChoiceQuery,
///         MatchCoverage,
///         SkippedWorkspaces,
///         choose,
///     },
///     tree::name_index::build::NameIndex,
/// };
///
/// let index = NameIndex::from_tree_json(
///     r#"{"channels":[{"id":"b1","type":"board","name":"Plans Q4"}]}"#,
/// )?;
/// let easy = Workspace {
///     id: "w1".into(),
///     name: "easy".into(),
/// };
/// let indexes = [(easy, &index)];
/// let choose = |reference, matching| {
///     let filter = BOARDS;
///     let query = ChoiceQuery {
///         reference,
///         filter,
///         matching,
///     };
///     choose(&indexes, &SkippedWorkspaces::default(), query)
/// };
/// assert_eq!(
///     choose("plans", MatchCoverage::Partial),
///     Choice::Chosen("b1".into())
/// );
/// assert_eq!(choose("plans", MatchCoverage::Exact), Choice::NotFound);
///
/// # Ok::<(), holst_core::Error>(())
/// ```
pub fn choose<I: Borrow<NameIndex>>(
    indexes: &[(Workspace, I)],
    unread: &SkippedWorkspaces,
    search: ChoiceQuery<'_>,
) -> Choice {
    let searched: Vec<&NameIndex> =
        indexes.iter().map(|(_, index)| index.borrow()).collect();
    decided(
        matches(&searched, search.reference, search.filter, search.matching),
        unread,
        |found| {
            indexes
                .get(found.workspace)
                .map(|(_, index)| index.borrow().id(found.node))
        },
    )
}

/// A live folder of one workspace by part of its name.
#[must_use]
pub fn folder(index: &NameIndex, reference: &str) -> Choice {
    within(index, reference, FOLDERS)
}

/// An item of one workspace's trash index by part of its name.
#[must_use]
pub fn trashed(index: &NameIndex, reference: &str) -> Choice {
    within(index, reference, TRASH)
}

fn within(index: &NameIndex, reference: &str, filter: NodeFilter) -> Choice {
    let found = matches(&[index], reference, filter, MatchCoverage::Partial);
    decided(found, &SkippedWorkspaces::default(), |found| {
        Some(index.id(found.node))
    })
}

fn decided(
    found: Vec<FoundNode>,
    unread: &SkippedWorkspaces,
    id: impl Fn(FoundNode) -> Option<String>,
) -> Choice {
    match (found.as_slice(), unread.is_uncertain()) {
        ([], _) => Choice::NotFound,
        (&[one], true) => Choice::Uncertain(one),
        (&[one], false) => id(one).map_or(Choice::NotFound, Choice::Chosen),
        _ => Choice::Ambiguous(found),
    }
}

/// The id of the one node `reference` names, as [`choose`] decides it.
///
/// # Errors
///
/// A [`LookupError`] of the filter's kind: not found, uncertain or ambiguous.
pub fn one<I: Borrow<NameIndex>>(
    indexes: &[(Workspace, I)],
    unread: &SkippedWorkspaces,
    search: ChoiceQuery<'_>,
) -> Result<String> {
    let choice = choose(indexes, unread, search);
    let wanted = LookupQuery {
        what: lookup_kind(search.filter),
        reference: search.reference,
    };
    chosen(choice, wanted, unread, |found| {
        Candidates::capped(
            found.iter().filter_map(|found| qualified(indexes, *found)),
            LISTED,
        )
    })
}

/// The id `choice` chose, else its [`LookupError`] error.
///
/// # Errors
///
/// A [`LookupError`] of `wanted`: not found, uncertain or ambiguous, with
/// `unread`.
pub fn chosen(
    choice: Choice,
    wanted: LookupQuery<'_>,
    unread: &SkippedWorkspaces,
    shown: impl FnOnce(&[FoundNode]) -> Candidates,
) -> Result<String> {
    let failed = |miss| {
        Err(Error::Lookup(LookupError {
            kind: wanted.what,
            reference: wanted.reference.to_owned(),
            miss,
        }))
    };
    match choice {
        Choice::Chosen(id) => Ok(id),
        Choice::Uncertain(found) => failed(lookup::LookupMiss::Uncertain {
            candidates: shown(&[found]),
            unread: unread.clone(),
        }),
        Choice::NotFound => failed(lookup::LookupMiss::NotFound {
            unread: unread.clone(),
            available: Candidates::default(),
        }),
        Choice::Ambiguous(found) => failed(lookup::LookupMiss::Ambiguous {
            candidates: shown(&found),
            unread: unread.clone(),
        }),
    }
}

/// `name (id), …` of the first [`LISTED`] matches in one `index`.
#[must_use]
pub fn listed(index: &NameIndex, found: &[FoundNode]) -> Candidates {
    Candidates::capped(
        found.iter().map(|found| Candidate {
            name: index.name(found.node).to_owned(),
            id: Some(index.id(found.node)),
            ..Candidate::default()
        }),
        LISTED,
    )
}

/// What a lookup through `filter` looks for, as its messages name it.
#[must_use]
pub const fn lookup_kind(filter: NodeFilter) -> lookup::LookupKind {
    match (filter.in_trash, filter.kind) {
        (true, _) => lookup::LookupKind::Trash,
        (false, Some(NodeKind::Board)) => lookup::LookupKind::Board,
        (false, Some(NodeKind::Folder)) => lookup::LookupKind::Folder,
        (false, Some(NodeKind::Other) | None) => lookup::LookupKind::Node,
    }
}

fn qualified<I: Borrow<NameIndex>>(
    indexes: &[(Workspace, I)],
    found: FoundNode,
) -> Option<Candidate> {
    let (workspace, index) = indexes.get(found.workspace)?;
    let index = index.borrow();
    Some(Candidate {
        name: index.name(found.node).to_owned(),
        id: Some(index.id(found.node)),
        workspace: Some(workspace.name.clone()),
        folders: index.folder_names(found.node),
        ..Candidate::default()
    })
}

/// Where a folder reference points before any request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentReference<'a> {
    Known(Parent),
    Root(&'a str),
    Named(&'a str),
}

pub const ROOT: &str = "root";

/// A folder reference of `workspace_id`; nothing is the root.
///
/// ```
/// use holst_core::domain::{
///     lookup::choice::{ParentReference, parent},
///     node::Parent,
/// };
///
/// let root = ParentReference::Known(Parent::Workspace("w1".into()));
/// assert_eq!(parent("w1", None), root);
/// assert_eq!(parent("w1", Some("/")), root);
/// assert_eq!(parent("w1", Some(" Root ")), ParentReference::Root("Root"));
/// assert_eq!(parent("w1", Some("Plans")), ParentReference::Named("Plans"));
/// ```
#[must_use]
pub fn parent<'a>(
    workspace_id: &str,
    reference: Option<&'a str>,
) -> ParentReference<'a> {
    let root = || ParentReference::Known(Parent::Workspace(workspace_id.to_owned()));
    let reference = reference
        .map(str::trim)
        .filter(|reference| !reference.is_empty());
    match reference {
        None | Some("/") => root(),
        Some(reference) if reference.eq_ignore_ascii_case(ROOT) => {
            ParentReference::Root(reference)
        }
        Some(reference) => match BoardId::find_in(reference) {
            Some(id) if id.as_str().eq_ignore_ascii_case(workspace_id) => root(),
            Some(id) => ParentReference::Known(Parent::Folder(id.into())),
            None => ParentReference::Named(reference),
        },
    }
}

/// The root `root` names in `index`, unless a folder is called so.
///
/// # Errors
///
/// [`crate::error::input::InputError::RootOrFolder`] with the folders called
/// `root`.
pub fn root_in(
    index: &NameIndex,
    workspace_id: &str,
    reference: &str,
) -> Result<Parent> {
    let named: Candidates = index
        .nodes()
        .filter(|&node| index.kind(node) == NodeKind::Folder && !index.in_trash(node))
        .filter(|&node| {
            crate::domain::tree::name_index::build::normalized(index.name(node)) == ROOT
        })
        .map(|node| Candidate {
            name: index.name(node).to_owned(),
            id: Some(index.id(node)),
            ..Candidate::default()
        })
        .collect();
    if named.is_empty() {
        Ok(Parent::Workspace(workspace_id.to_owned()))
    } else {
        Err(Error::Input(InputError::RootOrFolder {
            reference: reference.to_owned(),
            folders: named,
        }))
    }
}

fn named<'a>(all: impl Iterator<Item = &'a Workspace>) -> Candidates {
    all.map(|Workspace { id, name }| Candidate {
        name: name.clone(),
        id: Some(id.clone()),
        ..Candidate::default()
    })
    .collect()
}
