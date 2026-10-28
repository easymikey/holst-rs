//! A board's cover on either surface.

use holst_fake::Fake;
use serde_json::{Map, Value};

use crate::{
    error::ScenarioError,
    holst::{Holst, On},
    observation::Observation,
    surface::Words,
};

/// The cover `board edit` is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverEdit<'a> {
    /// A picture file of the working folder.
    File(&'a str),
    /// The automatic preview, `reset`.
    Reset,
}

impl On<'_> {
    /// Edits the board `board`, setting its cover when one is given.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn edit_board_settings(
        &self,
        board: &str,
        cover: Option<CoverEdit<'_>>,
    ) -> Result<Observation, ScenarioError> {
        let cover = cover.map(|choice| match choice {
            CoverEdit::File(name) => {
                self.holst.work_file(name).to_string_lossy().into_owned()
            }
            CoverEdit::Reset => "reset".to_owned(),
        });
        let words = Words {
            positional: vec![("board", board)],
            options: cover
                .as_deref()
                .map(|cover| ("cover", cover))
                .into_iter()
                .collect(),
            ..Words::default()
        };
        self.change("edit", words).await
    }
}

/// Expects things of the cover of a board, as its channel holds it.
#[must_use]
pub fn cover_of(holst: &Holst, board: &str) -> CoverThen {
    CoverThen {
        fields: holst.fake().cover_of(board),
        stored: stored_names(holst.fake()),
    }
}

/// The note of the board, as its document holds it; none without the board.
#[must_use]
pub fn note_of(holst: &Holst, board: &str) -> Option<String> {
    holst.fake().note_of(board)
}

fn stored_names(fake: &Fake) -> Vec<(String, String)> {
    fake.uploads()
        .into_iter()
        .map(|upload| (upload.file_name, upload.stored))
        .collect()
}

#[derive(Debug)]
pub struct CoverThen {
    fields: Map<String, Value>,
    /// The uploads as (file name, stored name).
    stored: Vec<(String, String)>,
}

impl CoverThen {
    fn thumbnail(&self) -> Option<&str> {
        self.fields.get("thumbnail").and_then(Value::as_str)
    }

    fn fixed(&self) -> bool {
        self.fields.get("thumbnail_fixed") == Some(&Value::Bool(true))
    }

    /// Expects the cover to be the upload of `file_name`, chosen by the user
    /// when `fixed`, drawn by holst otherwise.
    fn is_upload(&self, file_name: &str, fixed: bool) {
        let stored = self
            .stored
            .iter()
            .find(|(name, _)| name == file_name)
            .map(|(_, stored)| stored.as_str());
        assert!(
            stored.is_some() && self.thumbnail() == stored && self.fixed() == fixed,
            "expected the cover to be the upload of {file_name:?} (chosen: {fixed}), got {:?} of {:?}",
            self.fields,
            self.stored
        );
    }

    /// Expects the cover to be the upload of the file `file_name`, chosen by
    /// the user.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_the_upload_of(&self, file_name: &str) {
        self.is_upload(file_name, true);
    }

    /// Expects the cover to be the preview holst drew and uploaded, which the
    /// server keeps up to date.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_the_automatic_preview(&self) {
        self.is_upload("preview.png", false);
    }

    /// Expects the board to have no cover.
    ///
    /// # Panics
    /// When it has one.
    pub fn is_unset(&self) {
        assert_eq!(self.thumbnail(), None, "{:?}", self.fields);
    }
}
