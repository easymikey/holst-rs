//! A change that needs its confirmation first, and what it guards against.

use crate::{
    command::board::rights::Right,
    domain::{board::ObjectKind, text::counted},
    error::hint::Hint,
};

/// What a confirmation guards against; the refusal names it after the action.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display, strum::EnumIter,
)]
pub enum ChangeRisk {
    #[display("cannot be undone")]
    Irreversible,
    #[display("changes who can open or edit it")]
    Sharing,
    #[display("changes it for everyone in the workspace")]
    Everyone,
    #[display("takes everything inside along")]
    Contents,
}

/// A change a confirmation guards, with the outside data its words need.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::Display)]
pub enum RiskyChange {
    #[display("removing the saved login and the board cache")]
    ForgetLocalData,
    #[display("moving a folder with {}", counted(*boards, "board"))]
    MoveFolder { boards: usize },
    #[display("renaming {from} to {to}")]
    RenameWorkspace { from: String, to: String },
    #[display(
        "removing the {} and {} with it",
        kind.as_str(),
        counted(*objects, "object")
    )]
    RemoveContainer { kind: ObjectKind, objects: usize },
    #[display("removing {} with text", counted(*cells, "table cell"))]
    ClearCells { cells: usize },
    #[display("replacing the body of template {template}")]
    ReplaceTemplateBody { template: String },
    #[display("deleting template {template} for good")]
    DeleteTemplate { template: String },
    #[display("deleting the thread {thread} \"{first}\"{}", with_replies(*replies))]
    DeleteThread {
        thread: String,
        first: String,
        replies: usize,
    },
    #[display("deleting the reply {reply} \"{first}\" of thread {thread}")]
    DeleteReply {
        reply: String,
        first: String,
        thread: String,
    },
    #[display("replacing the objects of a board with a backup")]
    RestoreBackup,
    #[display("writing a board file into a board")]
    ImportIntoBoard,
    #[display("removing {}", counted(*objects, "object"))]
    RemoveObjects { objects: usize },
    #[display("deleting {node} for good")]
    DeleteNode { node: String },
    #[display("giving {user} the right {right} on {board}")]
    GrantRight {
        user: String,
        right: Right,
        board: String,
    },
    #[display("taking away the right of {user} on {board}")]
    RevokeRight { user: String, board: String },
    #[display("transferring ownership of {board} to {user}")]
    TransferOwnership { board: String, user: String },
    #[display("changing the sharing settings of {board}")]
    ChangeSharing { board: String },
    #[display(
        "removing the tag \"{tag}\" from {}",
        counted(*objects, "object")
    )]
    RemoveTag { tag: String, objects: usize },
}

impl RiskyChange {
    /// What the change puts at risk.
    #[must_use]
    pub const fn risk(&self) -> ChangeRisk {
        match self {
            Self::MoveFolder { .. } | Self::ClearCells { .. } => ChangeRisk::Contents,
            Self::RenameWorkspace { .. } | Self::ImportIntoBoard => {
                ChangeRisk::Everyone
            }
            Self::GrantRight { .. }
            | Self::RevokeRight { .. }
            | Self::ChangeSharing { .. } => ChangeRisk::Sharing,
            Self::ForgetLocalData
            | Self::RemoveContainer { .. }
            | Self::ReplaceTemplateBody { .. }
            | Self::DeleteTemplate { .. }
            | Self::DeleteThread { .. }
            | Self::DeleteReply { .. }
            | Self::RestoreBackup
            | Self::RemoveObjects { .. }
            | Self::DeleteNode { .. }
            | Self::TransferOwnership { .. }
            | Self::RemoveTag { .. } => ChangeRisk::Irreversible,
        }
    }
}

fn with_replies(replies: usize) -> String {
    match replies {
        0 => String::new(),
        1 => " with 1 reply".to_owned(),
        n => format!(" with {n} replies"),
    }
}

/// A change without its confirmation: the change and the risk.
///
/// ```
/// use holst_core::error::confirmation::{RiskyChange, Unconfirmed};
///
/// let err = Unconfirmed::from(RiskyChange::DeleteTemplate {
///     template: "@7".to_owned(),
/// });
/// assert_eq!(
///     err.to_string(),
///     "deleting template @7 for good cannot be undone"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{change} {}", change.risk())]
pub struct Unconfirmed {
    pub change: RiskyChange,
}

impl Unconfirmed {
    /// What the refused change puts at risk.
    #[must_use]
    pub const fn risk(&self) -> ChangeRisk {
        self.change.risk()
    }

    /// Advice for every unconfirmed action.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        Some(Hint::ConfirmFirst)
    }
}

impl From<RiskyChange> for Unconfirmed {
    fn from(change: RiskyChange) -> Self {
        Self { change }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::IntoEnumIterator as _;

    use crate::{
        command::board::rights::Right,
        domain::board::ObjectKind,
        error::{
            confirmation::{ChangeRisk, RiskyChange, Unconfirmed},
            hint::Hint,
        },
    };

    fn text(value: &str) -> String {
        value.to_owned()
    }

    #[rstest]
    #[case::forget(
        RiskyChange::ForgetLocalData,
        "removing the saved login and the board cache cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::move_one(
        RiskyChange::MoveFolder { boards: 1 },
        "moving a folder with 1 board takes everything inside along",
        ChangeRisk::Contents
    )]
    #[case::move_two(
        RiskyChange::MoveFolder { boards: 2 },
        "moving a folder with 2 boards takes everything inside along",
        ChangeRisk::Contents
    )]
    #[case::rename(
        RiskyChange::RenameWorkspace { from: text("Old"), to: text("New") },
        "renaming Old to New changes it for everyone in the workspace",
        ChangeRisk::Everyone
    )]
    #[case::container_one(
        RiskyChange::RemoveContainer { kind: ObjectKind::Shape, objects: 1 },
        "removing the shape and 1 object with it cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::container_two(
        RiskyChange::RemoveContainer { kind: ObjectKind::Shape, objects: 2 },
        "removing the shape and 2 objects with it cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::cells_one(
        RiskyChange::ClearCells { cells: 1 },
        "removing 1 table cell with text takes everything inside along",
        ChangeRisk::Contents
    )]
    #[case::cells_two(
        RiskyChange::ClearCells { cells: 2 },
        "removing 2 table cells with text takes everything inside along",
        ChangeRisk::Contents
    )]
    #[case::replace_body(
        RiskyChange::ReplaceTemplateBody { template: text("@7") },
        "replacing the body of template @7 cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::delete_template(
        RiskyChange::DeleteTemplate { template: text("@7") },
        "deleting template @7 for good cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::thread_no_reply(
        RiskyChange::DeleteThread { thread: text("@3"), first: text("Hi"), replies: 0 },
        "deleting the thread @3 \"Hi\" cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::thread_one_reply(
        RiskyChange::DeleteThread { thread: text("@3"), first: text("Hi"), replies: 1 },
        "deleting the thread @3 \"Hi\" with 1 reply cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::thread_two_replies(
        RiskyChange::DeleteThread { thread: text("@3"), first: text("Hi"), replies: 2 },
        "deleting the thread @3 \"Hi\" with 2 replies cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::reply(
        RiskyChange::DeleteReply { reply: text("@4"), first: text("Ok"), thread: text("@3") },
        "deleting the reply @4 \"Ok\" of thread @3 cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::restore(
        RiskyChange::RestoreBackup,
        "replacing the objects of a board with a backup cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::import(
        RiskyChange::ImportIntoBoard,
        "writing a board file into a board changes it for everyone in the workspace",
        ChangeRisk::Everyone
    )]
    #[case::objects_one(
        RiskyChange::RemoveObjects { objects: 1 },
        "removing 1 object cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::objects_two(
        RiskyChange::RemoveObjects { objects: 2 },
        "removing 2 objects cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::node(
        RiskyChange::DeleteNode { node: text("@9") },
        "deleting @9 for good cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::grant(
        RiskyChange::GrantRight { user: text("Ann"), right: Right::Editor, board: text("@2") },
        "giving Ann the right editor on @2 changes who can open or edit it",
        ChangeRisk::Sharing
    )]
    #[case::revoke(
        RiskyChange::RevokeRight { user: text("Ann"), board: text("@2") },
        "taking away the right of Ann on @2 changes who can open or edit it",
        ChangeRisk::Sharing
    )]
    #[case::transfer(
        RiskyChange::TransferOwnership { board: text("@2"), user: text("Ann") },
        "transferring ownership of @2 to Ann cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::sharing(
        RiskyChange::ChangeSharing { board: text("@2") },
        "changing the sharing settings of @2 changes who can open or edit it",
        ChangeRisk::Sharing
    )]
    #[case::tag_one(
        RiskyChange::RemoveTag { tag: text("red"), objects: 1 },
        "removing the tag \"red\" from 1 object cannot be undone",
        ChangeRisk::Irreversible
    )]
    #[case::tag_two(
        RiskyChange::RemoveTag { tag: text("red"), objects: 2 },
        "removing the tag \"red\" from 2 objects cannot be undone",
        ChangeRisk::Irreversible
    )]
    fn each_change_says_its_words(
        #[case] change: RiskyChange,
        #[case] said: &str,
        #[case] risk: ChangeRisk,
    ) {
        assert_eq!(change.risk(), risk);
        let refusal = Unconfirmed::from(change);
        assert_eq!(refusal.risk(), risk);
        assert_eq!(refusal.to_string(), said);
    }

    #[test]
    fn every_refusal_hints_to_confirm() {
        let err = Unconfirmed::from(RiskyChange::RestoreBackup);
        assert_eq!(err.hint(), Some(Hint::ConfirmFirst));
    }

    #[test]
    fn each_risk_has_its_words() {
        let said: Vec<String> =
            ChangeRisk::iter().map(|risk| risk.to_string()).collect();
        let undone = said.iter().filter(|text| text.contains("cannot be undone"));
        assert_eq!(undone.count(), 1, "{said:?}");
    }
}
