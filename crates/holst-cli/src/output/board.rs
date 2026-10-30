//! Board output shared by commands: what a board change reports.

use holst_core::domain::{
    change::receipt::{BoardStatus, ChangeReport},
    text::printable,
};

use crate::{error::Result, output, output::OutputFormat};

/// Prints what a change of a board did, or would do.
pub(crate) fn report(format: OutputFormat, change: &ChangeReport) -> Result<()> {
    warn_if_trashed(change);
    // Ids and kinds come from the board document, which others write.
    output::report(format, change)?;
    warnings(change);
    Ok(())
}

/// After the result, on stderr: stdout keeps the result alone.
pub(crate) fn warnings(change: &ChangeReport) {
    change
        .warnings
        .iter()
        .for_each(|warning| output::warn(printable(warning)));
}

/// Warns when the changed board is in the trash.
pub(crate) fn warn_if_trashed(change: &ChangeReport) {
    // The board server takes writes into a trashed board without a word.
    matches!(change.board_status, BoardStatus::InTrash)
        .then(|| in_trash_note(&change.board))
        .into_iter()
        .for_each(output::warn);
}

pub(crate) fn in_trash_note(id: &str) -> String {
    format!("the board is in the trash; `holst board restore {id}` brings it back")
}
