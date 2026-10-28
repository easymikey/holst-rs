//! What a scenario expects of an observation.

use holst_core::fake::ListedNode;
use serde_json::Value;

use crate::{
    holst::Holst,
    observation::{ErrorExit, Observation},
};

#[must_use]
pub const fn then(observation: &Observation) -> Then<'_> {
    Then { observation }
}

#[derive(Debug)]
pub struct Then<'observation> {
    pub(crate) observation: &'observation Observation,
}

impl Then<'_> {
    /// Expects exactly these workspace names, in this order.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_workspaces(&self, names: &[&str]) {
        assert_eq!(
            self.observation,
            &Observation::Workspaces(
                names.iter().map(|name| (*name).to_owned()).collect()
            )
        );
    }

    /// Expects a failure whose hint contains `words`.
    ///
    /// # Panics
    /// When the observation differs.
    #[must_use]
    pub fn failed_with_hint(&self, words: &str) -> &Self {
        assert!(
            matches!(self.observation, Observation::Failed { hint, .. } if hint.contains(words)),
            "expected a failure with the hint {words:?}, got {:?}",
            self.observation
        );
        self
    }

    /// Expects a failure whose message contains `words`.
    ///
    /// # Panics
    /// When the observation differs.
    #[must_use]
    pub fn failed_saying(&self, words: &str) -> &Self {
        self.fails_saying(words);
        self
    }

    /// As [`Self::failed_saying`], ending the chain.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn fails_saying(&self, words: &str) {
        assert!(
            matches!(self.observation, Observation::Failed { message, .. } if message.contains(words)),
            "expected a failure saying {words:?}, got {:?}",
            self.observation
        );
    }

    /// Expects a failure with exactly this message.
    ///
    /// # Panics
    /// When the observation differs.
    #[must_use]
    pub fn failed_with_message(&self, message: &str) -> &Self {
        assert!(
            matches!(self.observation, Observation::Failed { message: seen, .. } if seen == message),
            "expected a failure with the message {message:?}, got {:?}",
            self.observation
        );
        self
    }

    /// Expects a preview of a request whose path ends in `path` and whose
    /// body has `field` set to `value`.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn showed_dry_run(&self, path: &str, field: &str, value: &Value) {
        assert!(
            matches!(self.observation, Observation::DryRun { path: seen, body }
                if seen.ends_with(path) && body.get(field) == Some(value)),
            "expected a preview of {path} with {field} = {value}, got {:?}",
            self.observation
        );
    }

    /// Expects a preview of a request whose path holds `path`.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn showed_a_preview(&self, path: &str) {
        assert!(
            matches!(self.observation, Observation::DryRun { path: seen, .. } if seen.contains(path)),
            "expected a preview of {path}, got {:?}",
            self.observation
        );
    }

    /// Expects the failure to be signaled the way `signal` says.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn signalled_as(&self, signal: ErrorExit) {
        assert!(
            matches!(self.observation, Observation::Failed { signal: seen, .. } if *seen == signal),
            "expected the signal {signal:?}, got {:?}",
            self.observation
        );
    }
}

/// Expects things of the folders and boards a workspace holds.
#[must_use]
pub fn tree_of(holst: &Holst, workspace: &str) -> TreeThen {
    TreeThen {
        entries: holst.fake().tree_of(workspace),
    }
}

/// Expects things of the folders and boards in the trash of a workspace.
#[must_use]
pub fn trash_of(holst: &Holst, workspace: &str) -> TreeThen {
    TreeThen {
        entries: holst.fake().trash_of(workspace),
    }
}

/// Expects things of the favorites of a workspace.
#[must_use]
pub fn favorites_of(holst: &Holst, workspace: &str) -> FavoritesThen {
    FavoritesThen {
        names: holst.fake().favorites_of(workspace),
    }
}

#[derive(Debug)]
pub struct FavoritesThen {
    names: Vec<String>,
}

impl FavoritesThen {
    /// Expects exactly these favorites, in this order.
    ///
    /// # Panics
    /// When they differ.
    pub fn are(&self, names: &[&str]) {
        assert_eq!(self.names, names, "the favorites");
    }
}

#[derive(Debug)]
pub struct TreeThen {
    entries: Vec<ListedNode>,
}

impl TreeThen {
    /// Expects a folder with this name in the folder `parent`, or at the root.
    ///
    /// # Panics
    /// When there is none.
    pub fn has_folder(&self, name: &str, parent: Option<&str>) {
        self.expects(name, "folder", parent);
    }

    /// Expects a board with this name in the folder, or at the root.
    ///
    /// # Panics
    /// When there is none.
    pub fn has_board(&self, name: &str, folder: Option<&str>) {
        self.expects(name, "board", folder);
    }

    /// Expects nothing of this name.
    ///
    /// # Panics
    /// When there is something.
    pub fn has_nothing_named(&self, name: &str) {
        assert!(
            self.entries.iter().all(|entry| entry.name != name),
            "expected nothing named {name:?}, the tree is {:?}",
            self.entries
        );
    }

    fn expects(&self, name: &str, kind: &str, parent: Option<&str>) {
        let wanted = ListedNode {
            name: name.to_owned(),
            kind: kind.to_owned(),
            parent: parent.map(str::to_owned),
        };
        assert!(
            self.entries.contains(&wanted),
            "expected {wanted:?}, the tree is {:?}",
            self.entries
        );
    }
}
