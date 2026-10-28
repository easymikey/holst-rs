//! What holst keeps on the machine, and cleaning it: the CLI only.

use serde_json::Value;

use crate::{
    cli,
    error::ScenarioError,
    holst::{Holst, On, SendMode},
    observation::Observation,
    then::Then,
};

/// How far a `storage clean` reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleaning {
    /// Old logins, partial files and stale copies.
    Leftovers,
    /// Those and the board cache.
    Cache,
    /// Everything, the saved login included.
    All,
}

impl On<'_> {
    /// Lists what holst keeps.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_storage(&self) -> Result<Observation, ScenarioError> {
        let observation =
            cli::run(self.holst.command("holst")?, ["storage", "ls"], |stdout| {
                Ok(Observation::Shown(
                    String::from_utf8_lossy(stdout).into_owned(),
                ))
            })
            .await?;
        self.holst.checked(observation)
    }

    /// Cleans what holst keeps, as far as `cleaning` says.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn clean_storage(
        &self,
        cleaning: Cleaning,
    ) -> Result<Observation, ScenarioError> {
        let reach = match cleaning {
            Cleaning::Leftovers => None,
            Cleaning::Cache => Some("--cache"),
            Cleaning::All => Some("--all"),
        };
        let flags = [
            reach,
            (self.mode == SendMode::Confirmed).then_some("-y"),
            (self.mode == SendMode::DryRun).then_some("-n"),
        ];
        let line: Vec<&str> = ["storage", "clean"]
            .into_iter()
            .chain(flags.into_iter().flatten())
            .collect();
        let observation = cli::run(self.holst.command("holst")?, line, |stdout| {
            Ok(Observation::Shown(
                String::from_utf8_lossy(stdout).into_owned(),
            ))
        })
        .await?;
        self.holst.checked(observation)
    }
}

impl Then<'_> {
    /// Expects the listing to name a place with this role and size in bytes.
    ///
    /// # Panics
    /// When it does not.
    pub fn lists_storage(&self, role: &str, bytes: u64) {
        assert!(
            self.places().iter().any(|place| {
                place.get("role") == Some(&role.into())
                    && place.get("bytes") == Some(&bytes.into())
            }),
            "expected a {role} place of {bytes} bytes, got {:?}",
            self.observation
        );
    }

    /// Expects the listing to name a place with this role, of any size.
    ///
    /// # Panics
    /// When it does not.
    pub fn lists_a_place_of(&self, role: &str) {
        assert!(
            self.places()
                .iter()
                .any(|place| place.get("role") == Some(&role.into())),
            "expected a {role} place, got {:?}",
            self.observation
        );
    }

    /// Expects the listing to name no place at all.
    ///
    /// # Panics
    /// When it names one.
    pub fn lists_no_storage(&self) {
        assert!(
            matches!(self.observation, Observation::Shown(_))
                && self.places().is_empty(),
            "expected no places, got {:?}",
            self.observation
        );
    }

    /// The places of a listing printed as JSON.
    fn places(&self) -> Vec<Value> {
        self.observation
            .shown()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|json| json.get("places").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    }
}

/// Expects things of the files holst keeps, by path from the root of the run's
/// folders (`home`, `holst`, `cache`).
#[must_use]
pub fn kept_in(holst: &Holst) -> KeptThen<'_> {
    KeptThen { holst }
}

#[derive(Debug)]
pub struct KeptThen<'holst> {
    holst: &'holst Holst,
}

impl KeptThen<'_> {
    /// Expects the file or folder at `path` to be there.
    ///
    /// # Panics
    /// When it is not.
    pub fn has(&self, path: &str) {
        assert!(self.holst.path(path).exists(), "{path} is gone");
    }

    /// Expects nothing at `path`.
    ///
    /// # Panics
    /// When something is.
    pub fn lacks(&self, path: &str) {
        assert!(!self.holst.path(path).exists(), "{path} is still there");
    }

    /// Expects at least one file below the folder at `path`.
    ///
    /// # Panics
    /// When there is none.
    pub fn has_files_in(&self, path: &str) {
        assert!(
            files_below(&self.holst.path(path)) > 0,
            "{path} holds no file"
        );
    }

    /// Expects no file below the folder at `path`.
    ///
    /// # Panics
    /// When there is one.
    pub fn has_no_files_in(&self, path: &str) {
        assert_eq!(files_below(&self.holst.path(path)), 0, "{path} holds files");
    }
}

fn files_below(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() { files_below(&path) } else { 1 }
        })
        .sum()
}
