//! A name or id that matched nothing, several things, or not for certain.

use std::fmt;

use itertools::Itertools as _;

use crate::error::{Error, hint::Hint};

/// What was looked for; its word is in every lookup message.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    derive_more::Display,
    strum::EnumIter,
)]
pub enum LookupKind {
    #[display("board")]
    Board,
    #[display("folder")]
    Folder,
    #[display("board or folder")]
    Node,
    #[display("workspace")]
    Workspace,
    #[display("template category")]
    TemplateCategory,
    #[display("template")]
    Template,
    #[display("object")]
    Object,
    #[display("frame")]
    Frame,
    #[display("board or folder")]
    Trash,
    #[display("backup")]
    Backup,
    #[display("comment thread")]
    Thread,
}

impl LookupKind {
    /// The word for several of this kind.
    #[must_use]
    pub const fn plural(self) -> &'static str {
        match self {
            LookupKind::Board => "boards",
            LookupKind::Folder => "folders",
            LookupKind::Node | LookupKind::Trash => "boards or folders",
            LookupKind::Workspace => "workspaces",
            LookupKind::TemplateCategory => "template categories",
            LookupKind::Template => "templates",
            LookupKind::Object => "objects",
            LookupKind::Frame => "frames",
            LookupKind::Backup => "backups",
            LookupKind::Thread => "comment threads",
        }
    }

    /// Where it was looked for, e.g. ` on the board`; empty for the workspaces.
    #[must_use]
    pub const fn place(self) -> &'static str {
        match self {
            LookupKind::Object | LookupKind::Frame => " on the board",
            LookupKind::Thread => " on this board",
            LookupKind::Trash => " in the trash",
            LookupKind::Board
            | LookupKind::Folder
            | LookupKind::Node
            | LookupKind::Workspace
            | LookupKind::TemplateCategory
            | LookupKind::Template
            | LookupKind::Backup => "",
        }
    }
}

/// A thing a lookup names, with what the reader can tell it by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Candidate {
    /// The short handle a surface gave it (`@3`).
    pub alias: Option<String>,
    pub name: String,
    pub id: Option<String>,
    pub workspace: Option<String>,
    /// The folders above it, outermost first.
    pub folders: Vec<String>,
    pub folder: bool,
}

impl fmt::Display for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let alias = self.alias.iter().map(|alias| format!("{alias} ")).join("");
        let slash = if self.folder { "/" } else { "" };
        let id = self
            .id
            .iter()
            .filter(|id| **id != self.name)
            .map(|id| format!(" ({id})"))
            .join("");
        let place = self
            .workspace
            .iter()
            .chain(&self.folders)
            .map(|part| format!("{part}/"))
            .join("");
        write!(f, "{alias}{place}{}{slash}{id}", self.name)
    }
}

/// The first few candidates and how many more there are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Candidates {
    pub listed: Vec<Candidate>,
    pub more: usize,
}

impl Candidates {
    /// The first `shown` of `items`; the rest only counted.
    pub fn capped(items: impl IntoIterator<Item = Candidate>, shown: usize) -> Self {
        let all: Vec<Candidate> = items.into_iter().collect();
        Candidates {
            more: all.len().saturating_sub(shown),
            listed: all.into_iter().take(shown).collect(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.listed.is_empty()
    }
}

impl FromIterator<Candidate> for Candidates {
    fn from_iter<I: IntoIterator<Item = Candidate>>(items: I) -> Self {
        Candidates {
            listed: items.into_iter().collect(),
            more: 0,
        }
    }
}

impl fmt::Display for Candidates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let more = match self.more {
            0 => String::new(),
            more => format!(" and {more} more"),
        };
        write!(f, "{}{more}", self.listed.iter().join(", "))
    }
}

/// Why a workspace could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Denied,
    NotRead,
}

impl From<&Error> for SkipReason {
    fn from(err: &Error) -> Self {
        match err.status() {
            Some(403 | 404) => SkipReason::Denied,
            _ => SkipReason::NotRead,
        }
    }
}

/// One workspace that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub workspace: String,
    pub reason: SkipReason,
    pub status: Option<u16>,
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "{}: HTTP {status}", self.workspace),
            None => write!(f, "{}: could not be read", self.workspace),
        }
    }
}

/// Workspaces that could not be read, and why.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SkippedWorkspaces(pub Vec<Skipped>);

impl SkippedWorkspaces {
    /// Every workspace was read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A failure other than missing access: a choice by name is ambiguous.
    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        self.0
            .iter()
            .any(|unread| unread.reason == SkipReason::NotRead)
    }

    /// Every unread workspace, separated by `; `.
    #[must_use]
    pub fn all(&self) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| write!(f, "{}", self.0.iter().join("; ")))
    }

    /// Only those that make a choice by name ambiguous.
    #[must_use]
    pub fn uncertain(&self) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| {
            let mut failed = self.0.iter().filter(|u| u.reason == SkipReason::NotRead);
            write!(f, "{}", failed.join("; "))
        })
    }
}

/// Why the lookup did not end in one thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupMiss {
    NotFound {
        unread: SkippedWorkspaces,
        available: Candidates,
    },
    Ambiguous {
        candidates: Candidates,
        unread: SkippedWorkspaces,
    },
    Uncertain {
        candidates: Candidates,
        unread: SkippedWorkspaces,
    },
    Elsewhere {
        workspace: String,
    },
}

impl LookupMiss {
    /// Nothing matches, everything was read, and nothing is listed.
    #[must_use]
    pub const fn not_found() -> Self {
        LookupMiss::NotFound {
            unread: SkippedWorkspaces(Vec::new()),
            available: Candidates {
                listed: Vec::new(),
                more: 0,
            },
        }
    }

    /// Several match among everything there is to read.
    #[must_use]
    pub fn ambiguous(candidates: Candidates) -> Self {
        LookupMiss::Ambiguous {
            candidates,
            unread: SkippedWorkspaces::default(),
        }
    }
}

/// A lookup of `kind` by `reference` that did not end in one thing.
///
/// ```
/// use holst_core::error::lookup::{LookupError, LookupKind, LookupMiss};
///
/// let err = LookupError {
///     kind: LookupKind::Folder,
///     reference: "Plans".into(),
///     miss: LookupMiss::not_found(),
/// };
/// assert_eq!(err.to_string(), "folder \"Plans\" not found");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", message(.kind, .reference, .miss))]
pub struct LookupError {
    pub kind: LookupKind,
    pub reference: String,
    pub miss: LookupMiss,
}

impl LookupError {
    /// What to do next, decided for every kind and outcome.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match (&self.miss, self.kind) {
            (LookupMiss::NotFound { .. }, LookupKind::Board) => Some(Hint::ListBoards),
            (LookupMiss::NotFound { .. }, LookupKind::Folder | LookupKind::Node)
            | (LookupMiss::Elsewhere { .. }, _) => Some(Hint::ListTree),
            (LookupMiss::NotFound { .. }, LookupKind::Trash) => Some(Hint::ListTrash),
            (LookupMiss::NotFound { .. }, LookupKind::Backup) => {
                Some(Hint::ListBackups)
            }
            (
                LookupMiss::NotFound { .. } | LookupMiss::Ambiguous { .. },
                LookupKind::Object | LookupKind::Frame,
            ) => Some(Hint::ListObjects),
            // The message lists what there is.
            (
                LookupMiss::NotFound { .. },
                LookupKind::Workspace
                | LookupKind::TemplateCategory
                | LookupKind::Thread
                | LookupKind::Template,
            ) => None,
            (LookupMiss::Ambiguous { .. }, LookupKind::Board | LookupKind::Node)
            | (LookupMiss::Uncertain { .. }, _) => Some(Hint::NarrowDown),
            (
                LookupMiss::Ambiguous { .. },
                LookupKind::Folder
                | LookupKind::Trash
                | LookupKind::Backup
                | LookupKind::Workspace
                | LookupKind::TemplateCategory
                | LookupKind::Thread
                | LookupKind::Template,
            ) => Some(Hint::ChooseId),
        }
    }
}

fn message<'a>(
    what: &LookupKind,
    reference: &'a str,
    outcome: &'a LookupMiss,
) -> impl fmt::Display + 'a {
    let (what, place) = (*what, what.place());
    fmt::from_fn(move |f| match outcome {
        LookupMiss::NotFound {
            unread,
            available: known,
        } => write!(
            f,
            "{what} \"{reference}\" not found{place}{}{}",
            note(". Available: ", known.is_empty(), known),
            note("; could not read: ", unread.is_empty(), unread.all())
        ),
        LookupMiss::Ambiguous { candidates, unread } => write!(
            f,
            "\"{reference}\" matches several {}{place}: {candidates}{}",
            what.plural(),
            note("; could not read: ", unread.is_empty(), unread.all())
        ),
        LookupMiss::Uncertain { candidates, unread } => write!(
            f,
            "{what} \"{reference}\"{}: not every workspace could be read ({}), so the \
             match may be ambiguous",
            note(" matched ", candidates.is_empty(), candidates),
            unread.uncertain()
        ),
        LookupMiss::Elsewhere { workspace } => {
            write!(
                f,
                "{what} \"{reference}\" is not in workspace \"{workspace}\""
            )
        }
    })
}

fn note(lead: &str, empty: bool, text: impl fmt::Display) -> impl fmt::Display {
    fmt::from_fn(move |f| {
        if empty {
            Ok(())
        } else {
            write!(f, "{lead}{text}")
        }
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::IntoEnumIterator as _;

    use crate::{
        error::{
            hint::Hint,
            lookup::{
                LookupError,
                LookupKind,
                LookupMiss,
                SkipReason,
                Skipped,
                SkippedWorkspaces,
            },
        },
        fake::candidates,
    };

    fn unread(workspace: &str, status: u16) -> SkippedWorkspaces {
        SkippedWorkspaces(vec![Skipped {
            workspace: workspace.to_owned(),
            reason: SkipReason::NotRead,
            status: Some(status),
        }])
    }

    fn lookup(what: LookupKind, outcome: LookupMiss) -> LookupError {
        LookupError {
            kind: what,
            reference: "Plans".into(),
            miss: outcome,
        }
    }

    fn outcomes() -> [LookupMiss; 4] {
        [
            LookupMiss::not_found(),
            LookupMiss::ambiguous(candidates(&["a", "b"])),
            LookupMiss::Uncertain {
                candidates: candidates(&["a"]),
                unread: unread("team", 500),
            },
            LookupMiss::Elsewhere {
                workspace: "easy".into(),
            },
        ]
    }

    #[rstest]
    #[case::board(LookupKind::Board, "board \"Plans\" not found")]
    #[case::folder(LookupKind::Folder, "folder \"Plans\" not found")]
    #[case::node(LookupKind::Node, "board or folder \"Plans\" not found")]
    #[case::workspace(LookupKind::Workspace, "workspace \"Plans\" not found")]
    #[case::category(
        LookupKind::TemplateCategory,
        "template category \"Plans\" not found"
    )]
    #[case::template(LookupKind::Template, "template \"Plans\" not found")]
    #[case::object(LookupKind::Object, "object \"Plans\" not found on the board")]
    #[case::frame(LookupKind::Frame, "frame \"Plans\" not found on the board")]
    #[case::trash(
        LookupKind::Trash,
        "board or folder \"Plans\" not found in the trash"
    )]
    #[case::backup(LookupKind::Backup, "backup \"Plans\" not found")]
    fn not_found_names_its_kind(#[case] what: LookupKind, #[case] expected: &str) {
        assert_eq!(lookup(what, LookupMiss::not_found()).to_string(), expected);
    }

    #[rstest]
    #[case::everything_read(&[], &[], "board \"Plans\" not found")]
    #[case::partly_read(
        &[("Team", 403)],
        &[],
        "board \"Plans\" not found; could not read: Team: HTTP 403"
    )]
    #[case::known(&[], &["easy"], "board \"Plans\" not found. Available: easy")]
    fn not_found_names_unread_and_known(
        #[case] failed: &[(&str, u16)],
        #[case] known: &[&str],
        #[case] expected: &str,
    ) {
        let outcome = LookupMiss::NotFound {
            unread: SkippedWorkspaces(
                failed
                    .iter()
                    .flat_map(|(name, status)| unread(name, *status).0)
                    .collect(),
            ),
            available: candidates(known),
        };
        assert_eq!(lookup(LookupKind::Board, outcome).to_string(), expected);
    }

    #[test]
    fn ambiguous_names_skipped_workspaces() {
        let outcome = LookupMiss::Ambiguous {
            candidates: candidates(&["a", "b"]),
            unread: unread("team", 500),
        };
        assert_eq!(
            lookup(LookupKind::Board, outcome).to_string(),
            "\"Plans\" matches several boards: a, b; could not read: team: HTTP 500"
        );
    }

    #[test]
    fn other_outcomes_name_their_kind() {
        let texts: Vec<String> = outcomes()
            .into_iter()
            .skip(1)
            .map(|outcome| lookup(LookupKind::Folder, outcome).to_string())
            .collect();
        assert_eq!(
            texts,
            [
                "\"Plans\" matches several folders: a, b",
                "folder \"Plans\" matched a: not every workspace could be read (team: HTTP \
                 500), so the match may be ambiguous",
                "folder \"Plans\" is not in workspace \"easy\"",
            ]
        );
    }

    #[test]
    fn every_kind_and_outcome_has_a_hint_decision_and_its_word() {
        let pairs: Vec<(LookupKind, Option<Hint>, String)> = LookupKind::iter()
            .flat_map(|what| {
                outcomes().into_iter().map(move |outcome| {
                    let err = lookup(what, outcome);
                    (what, err.hint(), err.to_string())
                })
            })
            .collect();
        let unnamed: Vec<&String> = pairs
            .iter()
            .filter(|(what, _, text)| {
                !text.contains(&what.to_string()) && !text.contains(what.plural())
            })
            .map(|(_, _, text)| text)
            .collect();
        assert_eq!(unnamed, Vec::<&String>::new());
        let silent: Vec<(LookupKind, String)> = pairs
            .into_iter()
            .filter(|(_, hint, _)| hint.is_none())
            .map(|(what, _, text)| (what, text))
            .collect();
        assert!(
            silent.iter().all(|(what, text)| matches!(
                what,
                LookupKind::Workspace
                    | LookupKind::TemplateCategory
                    | LookupKind::Thread
                    | LookupKind::Template
            ) && text.contains("not found")),
            "{silent:?}"
        );
    }
}
