//! `holst object`: the objects on a board.

mod items;
mod render;

use holst_core::command::{
    board::picture::Delivery,
    object::{
        ObjectVerb,
        add,
        align,
        change::{self, Removed},
        copy,
        get,
        group,
        list,
        lock,
    },
    table,
};
use serde_json::{Map, Value};

use crate::{
    commands,
    error::Result,
    output::{self, OutputFormat, OutputTarget},
    run::{RunSettings, run_signed_in},
};

#[expect(clippy::too_many_lines, reason = "one arm per verb")]
pub(crate) async fn run(
    verb: ObjectVerb,
    mut arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    items::read(&mut arguments)?;
    match verb {
        ObjectVerb::Ls => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                let listing = list::ls(client, run.cache(), &args).await?;
                output::print(format, &render::listing_text(&listing), &listing)
            })
            .await
        }
        ObjectVerb::Add => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| add::add(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Edit => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| change::edit(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Cp => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| copy::cp(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Group => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| group::group(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Ungroup => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| group::ungroup(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Lock => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| lock::lock(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Unlock => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| lock::unlock(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Align => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| align::align(client, run.cache(), args).await,
                output::board::report,
            )
            .await
        }
        ObjectVerb::Rm => {
            commands::change(
                arguments,
                run,
                format,
                async |client, args| change::rm(client, run.cache(), args).await,
                removed,
            )
            .await
        }
        ObjectVerb::Get => download(arguments, run, format).await,
    }
}

/// Prints a removal: the trash warning, then the removal itself.
fn removed(format: OutputFormat, removed: &Removed) -> Result<()> {
    output::board::warn_if_trashed(&removed.report);
    output::print(format, &removed.report.to_string(), removed)
}

/// Downloads the originals of objects; a file of `-` sends the bytes to stdout.
async fn download(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let target = OutputTarget::of(output::stdout_kind());
    if arguments.get("file").and_then(Value::as_str) == Some("-") {
        // The bytes never go to a terminal: refused before any request.
        target.checked()?;
    }
    let args = table::parse(arguments)?;
    let downloads = run.downloads_dir();
    run_signed_in(run, async |client| {
        let downloaded =
            get::get(client, run.cache(), downloads.as_deref(), &args).await?;
        let inline =
            downloaded
                .objects
                .iter()
                .find_map(|object| match &object.delivery {
                    Delivery::Inline { bytes } => Some(bytes),
                    Delivery::Written { .. } => None,
                });
        match inline {
            Some(bytes) => {
                target.put(bytes)?;
                output::stderr_line(render::downloaded_text(&downloaded));
                Ok(())
            }
            None => output::print(
                format,
                &render::downloaded_text(&downloaded),
                &downloaded,
            ),
        }
    })
    .await
}
