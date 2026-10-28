//! What the backup commands print.

use holst_core::{
    command::{
        backup::{Fetched, Restored},
        board::picture::Delivery,
        change::ChangeReport,
    },
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

/// Prints where a backup was saved.
pub(super) fn fetched(format: OutputFormat, fetched: &Fetched) -> Result<()> {
    output::print(format, &fetched_text(fetched), fetched)
}

/// Prints what a restore did, or would do.
pub(super) fn restored(format: OutputFormat, restored: &Restored) -> Result<()> {
    output::print(format, &restored_text(restored), restored)
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

pub(super) fn fetched_text(fetched: &Fetched) -> String {
    match &fetched.delivery {
        Delivery::Written { file, bytes } => {
            let before = if fetched.uncompressed_bytes == *bytes {
                String::new()
            } else {
                format!(" ({} before gzip)", fetched.uncompressed_bytes)
            };
            format!("saved {bytes} bytes{before} to {}", file.display())
        }
        Delivery::Inline { bytes } => format!("{} bytes", bytes.len()),
    }
}

pub(super) fn restored_text(restored: &Restored) -> String {
    let written = matches!(restored.outcome, ChangeReport::Sent);
    let receipt = restored
        .receipt
        .filter(|_| written)
        .map(|each| each.to_string());
    let link = written.then(|| restored.link.clone());
    joined(
        [restored.phrase()].into_iter().chain(receipt).chain(link),
        "\n",
    )
    .to_string()
}
