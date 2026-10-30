//! What the backup commands print.

use holst_core::{
    command::backup::{Fetched, Restored},
    domain::{
        backup::Backups,
        text::{EMPTY, joined, printable},
    },
};

use crate::{
    error::Result,
    output::{self, OutputFormat},
};

/// Prints a board's backups, one line each as the server lists them.
pub(super) fn listing(format: OutputFormat, backups: &Backups) -> Result<()> {
    match format {
        OutputFormat::Text => output::line(listing_text(backups)),
        OutputFormat::Pretty | OutputFormat::Json => {
            output::print_value(format, &backups.as_slice())
        }
    }
}

pub(super) fn fetched(format: OutputFormat, fetched: &Fetched) -> Result<()> {
    output::print(format, &fetched.to_string(), fetched)
}

/// Prints what a restore did, or would do.
pub(super) fn restored(format: OutputFormat, restored: &Restored) -> Result<()> {
    output::print(format, &restored.to_string(), restored)
}

pub(super) fn listing_text(backups: &Backups) -> String {
    match backups.as_slice() {
        [] => EMPTY.to_owned(),
        all => joined(
            all.iter().map(|each| printable(&each.line()).into_owned()),
            "\n",
        )
        .to_string(),
    }
}
