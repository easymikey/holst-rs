//! Edits behind the client's back, and what a scenario reads of a board.

use serde_json::Value;

use super::FixtureObject;
use crate::{Fake, FixtureError, fixture::unknown};

impl Fake {
    /// Adds `objects` to the board called `board` as another client would,
    /// numbered after the ones it holds; what a client cached of the board is
    /// stale from now on. The socket does not count it as a write.
    ///
    /// # Errors
    /// [`FixtureError`] when no board has that name.
    pub fn edit_board(
        &self,
        board: &str,
        objects: Vec<FixtureObject>,
    ) -> Result<(), FixtureError> {
        self.locked()
            .tree
            .append(board, objects)
            .then_some(())
            .ok_or_else(|| unknown("board", board))
    }

    /// Returns the fields of every object of the board called `board`, as the
    /// document holds them now, in the order they came.
    #[must_use]
    pub fn objects_of(&self, board: &str) -> Vec<Value> {
        let state = self.locked();
        state
            .tree
            .id_of(board)
            .map(|id| state.tree.objects_json(&id))
            .unwrap_or_default()
    }
}
