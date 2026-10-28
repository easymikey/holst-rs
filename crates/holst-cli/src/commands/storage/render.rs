//! What the storage commands print.

use holst_core::{
    command::{
        change::ChangeReport,
        storage::{Cleaned, Listing},
    },
    domain::text::{joined, printable},
    storage::{local_data::DataPlace, session_store::SessionStoreKind},
};

use crate::{
    error::Result,
    output::{self, OutputFormat, view::size_text},
};

/// Prints every place holst keeps data in, with sizes.
pub(super) fn listing(format: OutputFormat, listing: &Listing) -> Result<()> {
    match format {
        OutputFormat::Text => {
            output::line(listed_text(&listing.places, listing.removable_bytes))
        }
        OutputFormat::Pretty | OutputFormat::Json => {
            output::print_value(format, listing)
        }
    }
}

/// Prints what a clean removed, or would remove; `forgot` lists the logins it
/// forgot.
pub(super) fn cleaned(
    format: OutputFormat,
    cleaned: &Cleaned,
    forgot: Option<&[SessionStoreKind]>,
) -> Result<()> {
    match format {
        OutputFormat::Text => output::line(cleaned_text(cleaned, forgot)),
        OutputFormat::Pretty | OutputFormat::Json => {
            output::print_value(format, cleaned)
        }
    }
}

pub(super) fn listed_text(places: &[DataPlace], removable: u64) -> String {
    let lines = places.iter().map(|place| {
        let size = place.bytes().map_or_else(|| "-".to_owned(), size_text);
        let role = serde_json::to_value(place.role())
            .ok()
            .and_then(|role| role.as_str().map(str::to_owned))
            .unwrap_or_default();
        format!(
            "{size:>10}  {role:<11}  {}  ({})",
            printable(place.name()),
            place.role().description()
        )
    });
    let summary = match removable {
        0 => "nothing to clean".to_owned(),
        bytes => format!("`holst storage clean` frees {}", size_text(bytes)),
    };
    match places {
        [] => "holst keeps nothing on this machine".to_owned(),
        _ => format!("{}\n{summary}", joined(lines, "\n")),
    }
}

pub(super) fn cleaned_text(
    cleaned: &Cleaned,
    forgot: Option<&[SessionStoreKind]>,
) -> String {
    let verb = match cleaned.report {
        ChangeReport::Sent => "removed",
        ChangeReport::DryRun { .. } | ChangeReport::Held { .. } => "would remove",
    };
    let lines = cleaned.removed.iter().map(|place| {
        format!(
            "{verb} {} ({})",
            printable(&place.name),
            size_text(place.bytes)
        )
    });
    let failed = cleaned.failed.iter().map(|failure| {
        format!(
            "not removed {}: {}",
            printable(&failure.name),
            failure.error
        )
    });
    let evicted = cleaned.evicted.map(|usage| {
        format!(
            "{verb} {} of {} cached boards over the limit or unused for a month",
            size_text(usage.bytes),
            usage.boards
        )
    });
    let freed: u64 = cleaned
        .removed
        .iter()
        .map(|place| place.bytes)
        .sum::<u64>()
        .saturating_add(cleaned.evicted.map_or(0, |usage| usage.bytes));
    let forgot = forgot.map(|gone| match gone {
        [] => "there was no saved login".to_owned(),
        gone => format!(
            "forgot the login in: {} (the server session stays; `holst auth logout` ends it)",
            joined(gone.iter().map(|kind| kind.as_str()), ", ")
        ),
    });
    let nothing = cleaned.removed.is_empty()
        && cleaned.failed.is_empty()
        && cleaned.evicted.is_none();
    let summary = match (nothing, &cleaned.report) {
        (true, _) => "nothing to clean".to_owned(),
        (false, ChangeReport::Sent) => {
            format!("removed {} in total", size_text(freed))
        }
        (false, ChangeReport::DryRun { .. } | ChangeReport::Held { .. }) => format!(
            "dry run, nothing removed: {} would be freed",
            size_text(freed)
        ),
    };
    lines
        .chain(failed)
        .chain(evicted)
        .chain(forgot)
        .chain([summary])
        .collect::<Vec<_>>()
        .join("\n")
}
