//! `holst object`: the objects on a board.

mod items;
mod render;

use holst_core::{
    command::{
        board::picture::Delivery,
        object::{ObjectVerb, add, change, copy, get, group, list},
        table::{self, CommandArgs},
    },
    domain::change::receipt::ChangeReport,
    effect::settings::CacheSettings,
    remote::client::Client,
};
use serde_json::{Map, Value};

use crate::{
    commands::refused,
    error::Result,
    output::{self, OutputFormat, OutputTarget},
    run::{RunSettings, run_signed_in},
};

/// Runs a command of the `object` group with the arguments the command line
/// gave it.
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
        ObjectVerb::Add => changed(arguments, run, format, add::add).await,
        ObjectVerb::Edit => changed(arguments, run, format, change::edit).await,
        ObjectVerb::Cp => changed(arguments, run, format, copy::cp).await,
        ObjectVerb::Group => changed(arguments, run, format, group::group).await,
        ObjectVerb::Ungroup => changed(arguments, run, format, group::ungroup).await,
        ObjectVerb::Rm => remove(arguments, run, format).await,
        ObjectVerb::Get => download(arguments, run, format).await,
    }
}

/// Makes one change of the objects and prints it.
async fn changed<A: CommandArgs>(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
    make: impl AsyncFn(&Client, &CacheSettings, &A) -> holst_core::Result<ChangeReport>,
) -> Result<()> {
    let args = table::parse::<A>(arguments)?;
    run_signed_in(run, async |client| {
        let report = make(client, run.cache(), &args).await?;
        output::board::report(format, &report)
    })
    .await
}

/// Removes objects and prints it. A removal held for its confirmation is shown
/// first as a plan; the refusal is the exit code.
async fn remove(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let args = table::parse(arguments)?;
    run_signed_in(run, async |client| {
        let removed = change::rm(client, run.cache(), &args).await?;
        output::board::warn_if_trashed(&removed.report);
        output::print(format, &removed.report.to_string(), &removed)?;
        refused(removed.refusal.as_ref())
    })
    .await
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
