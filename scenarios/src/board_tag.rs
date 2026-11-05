//! A board's tags on either surface.

use crate::{
    error::ScenarioError,
    holst::On,
    observation::Observation,
    surface::Words,
};

/// What `board tag` is asked to do to a tag.
#[derive(Debug, Clone, Copy, Default)]
pub struct TagEdit<'a> {
    /// The new title.
    pub name: Option<&'a str>,
    /// The new color, a palette token.
    pub color: Option<&'a str>,
    /// Delete the tag.
    pub remove: bool,
}

impl On<'_> {
    /// Renames, recolors or removes the tag `tag` of the board `board`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn edit_tag(
        &self,
        board: &str,
        tag: &str,
        edit: TagEdit<'_>,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            positional: vec![("board", board), ("tag", tag)],
            options: [("name", edit.name), ("color", edit.color)]
                .into_iter()
                .filter_map(|(name, value)| value.map(|value| (name, value)))
                .collect(),
            switches: edit.remove.then_some("remove").into_iter().collect(),
            ..Words::default()
        };
        self.change("tag", words).await
    }
}
