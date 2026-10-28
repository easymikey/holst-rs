//! `holst workspace`: the table's commands.

mod render;

use holst_core::command::{
    table,
    workspace::{self, WorkspaceVerb},
};
use serde_json::{Map, Value};

use crate::{
    commands::refused,
    error::Result,
    output::OutputFormat,
    run::{RunSettings, run_signed_in},
};

/// Runs a command of the `workspace` group with the arguments the command line
/// gave it.
pub(crate) async fn run(
    verb: WorkspaceVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match verb {
        WorkspaceVerb::Ls => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::listing(format, &workspace::list(client, &args).await?)
            })
            .await
        }
        WorkspaceVerb::Show => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::shown(format, &workspace::show(client, &args).await?)
            })
            .await
        }
        WorkspaceVerb::Create => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::change(format, &workspace::create(client, &args).await?)
            })
            .await
        }
        WorkspaceVerb::Rename => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                let change = workspace::rename(client, &args).await?;
                render::change(format, &change)?;
                // The plan is shown first; the refusal is the exit code.
                refused(change.report.refusal())
            })
            .await
        }
        WorkspaceVerb::Log => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::log(format, &workspace::log(client, &args).await?)
            })
            .await
        }
    }
}
