//! `holst backup`: a board's backups, listed, downloaded and restored.

mod render;
#[cfg(test)] mod tests;

use holst_core::command::{
    backup::{self, BackupVerb, GetArgs},
    board::picture::Delivery,
    table,
};
use serde_json::{Map, Value};

use crate::{
    commands::refused,
    error::Result,
    output::{self, OutputFormat, OutputTarget},
    run::{RunSettings, run_signed_in},
};

/// The file name that means standard output.
const STDOUT: &str = "-";

/// Runs a command of the `backup` group with the arguments the command line
/// gave it.
pub(crate) async fn run(
    verb: BackupVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match verb {
        BackupVerb::Ls => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::listing(format, &backup::ls(client, &args).await?)
            })
            .await
        }
        BackupVerb::Get => get(table::parse(arguments)?, run, format).await,
        BackupVerb::Restore => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                let restored = backup::restore(client, &args).await?;
                // The plan is shown first; the refusal is the exit code.
                render::restored(format, &restored)?;
                refused(restored.outcome.refusal())
            })
            .await
        }
    }
}

/// Downloads a backup into the file given, `-` for stdout, or into
/// `~/Downloads`. Bytes never go to a terminal: refused before the download.
async fn get(args: GetArgs, run: &RunSettings, format: OutputFormat) -> Result<()> {
    let to_stdout = args
        .file
        .as_ref()
        .is_some_and(|file| file.0.as_os_str() == STDOUT);
    let args = GetArgs {
        file: args.file.filter(|_| !to_stdout),
        ..args
    };
    if to_stdout {
        OutputTarget::of(output::stdout_kind()).checked()?;
    }
    let downloads = if args.file.is_none() && !to_stdout {
        Some(run.downloads()?)
    } else {
        None
    };
    run_signed_in(run, async |client| {
        let fetched = backup::get(client, downloads.as_deref(), &args).await?;
        match &fetched.delivery {
            Delivery::Written { .. } => render::fetched(format, &fetched),
            Delivery::Inline { bytes } => {
                OutputTarget::of(output::stdout_kind()).put(bytes)
            }
        }
    })
    .await
}
