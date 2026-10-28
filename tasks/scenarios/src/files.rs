//! Uploaded files and backups on either surface.

use serde_json::json;

use crate::{
    cli,
    error::ScenarioError,
    holst::{Holst, On, SendMode},
    observation::Observation,
    then::Then,
};

impl On<'_> {
    /// Reads the snapshots of the board called `board`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_backups(
        &self,
        board: &str,
    ) -> Result<Observation, ScenarioError> {
        self.reading(
            &["backup", "ls", board],
            "backup",
            json!({ "action": "ls", "board": board }),
        )
        .await
    }

    /// Saves the snapshot `key` (of `provider`, when given) as the new file
    /// `file_name`: gzip-packed, which adds `.gz` to the name, when `gzipped`,
    /// not packed otherwise.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn download_backup(
        &self,
        key: &str,
        file_name: &str,
        provider: Option<&str>,
        gzipped: bool,
    ) -> Result<Observation, ScenarioError> {
        let path = self.holst.work_file(file_name);
        let path_text = path.to_string_lossy();
        let mut line = vec!["backup", "get", key, &path_text];
        let mut arguments = serde_json::Map::from_iter([
            ("action".to_owned(), json!("get")),
            ("key".to_owned(), json!(key)),
            ("file".to_owned(), json!(path)),
        ]);
        if let Some(provider) = provider {
            line.extend(["--provider", provider]);
            arguments.insert("provider".to_owned(), json!(provider));
        }
        if !gzipped {
            line.push("--uncompressed");
            arguments.insert("uncompressed".to_owned(), json!(true));
        }
        self.changing(&line, "backup", arguments.into()).await
    }

    /// Saves the snapshot `key` without naming a file, not gzip-packed; only
    /// the CLI has a default place, `~/Downloads`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn download_backup_to_downloads(
        &self,
        key: &str,
    ) -> Result<Observation, ScenarioError> {
        let line = ["backup", "get", key, "--uncompressed"];
        self.changing(&line, "backup", json!({})).await
    }

    /// Prints the snapshot `key` to standard output, which only the CLI does.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn download_backup_to_stdout(
        &self,
        key: &str,
    ) -> Result<Observation, ScenarioError> {
        let observation = cli::run(
            self.holst.command("holst")?,
            ["backup", "get", key, "-", "--uncompressed"],
            |stdout| {
                Ok(Observation::Shown(
                    String::from_utf8_lossy(stdout).into_owned(),
                ))
            },
        )
        .await?;
        self.holst.checked(observation)
    }

    /// Writes the snapshot `key` back into the board `board`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn restore_backup(
        &self,
        board: &str,
        key: &str,
    ) -> Result<Observation, ScenarioError> {
        let flags = [
            (self.mode == SendMode::Confirmed).then_some("-y"),
            (self.mode == SendMode::DryRun).then_some("-n"),
        ];
        let line: Vec<&str> = ["backup", "restore", board, key]
            .into_iter()
            .chain(flags.into_iter().flatten())
            .collect();
        let arguments = self.admin_arguments(json!({
            "action": "restore",
            "board": board,
            "key": key,
        }));
        self.changing(&line, "backup", arguments).await
    }
}

impl Then<'_> {
    /// Expects a restore that replaces `before` objects with the `saved` ones
    /// of the backup, shown and not done.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_a_restore_of(&self, saved: usize, before: usize) {
        let counts = |text: &str| {
            serde_json::from_str::<serde_json::Value>(text).map_or_else(
                |_| text.contains(&format!("{saved} objects replace {before}")),
                |json| {
                    json.get("saved_objects") == Some(&saved.into())
                        && json.get("objects_before") == Some(&before.into())
                },
            )
        };
        assert!(
            matches!(self.observation, Observation::Shown(text) if counts(text)),
            "expected a restore of {saved} objects over {before}, got {:?}",
            self.observation
        );
    }
}

/// Expects things of the files the server received.
#[must_use]
pub fn uploads_of(holst: &Holst) -> UploadsThen {
    UploadsThen {
        files: holst
            .fake()
            .uploads()
            .into_iter()
            .map(|upload| (upload.file_name, upload.content))
            .collect(),
    }
}

#[derive(Debug)]
pub struct UploadsThen {
    files: Vec<(String, String)>,
}

impl UploadsThen {
    /// Expects the server to hold this file with this content.
    ///
    /// # Panics
    /// When it does not.
    pub fn holds(&self, file_name: &str, content: &str) {
        assert!(
            self.files
                .iter()
                .any(|(name, held)| name == file_name && held == content),
            "expected {file_name:?} with {content:?}, the server holds {:?}",
            self.files
        );
    }

    /// Expects exactly one file called `file_name` to have been uploaded.
    ///
    /// # Panics
    /// When there are none or several.
    pub fn holds_one_named(&self, file_name: &str) {
        assert_eq!(
            self.files
                .iter()
                .filter(|(name, _)| name == file_name)
                .count(),
            1,
            "{:?}",
            self.files
        );
    }

    /// Expects nothing to have been uploaded.
    ///
    /// # Panics
    /// When something was.
    pub fn is_empty(&self) {
        assert!(self.files.is_empty(), "{:?}", self.files);
    }
}

/// Returns the content of the file `name` a surface saved, if there is one.
#[must_use]
pub fn downloaded(holst: &Holst, name: &str) -> Option<String> {
    std::fs::read_to_string(holst.work_file(name)).ok()
}

/// Returns the bytes of the file `name` a surface saved, if there is one.
#[must_use]
pub fn downloaded_bytes(holst: &Holst, name: &str) -> Option<Vec<u8>> {
    std::fs::read(holst.work_file(name)).ok()
}

/// Returns how many objects the board called `board` holds now.
#[must_use]
pub fn objects_on(holst: &Holst, board: &str) -> usize {
    holst.fake().objects_of(board).len()
}
