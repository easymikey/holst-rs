//! `holst workspace`: the table's commands.

mod render;

use holst_core::command::{
    table,
    workspace::{self, WorkspaceVerb},
};
use serde_json::{Map, Value};

use crate::{
    commands,
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
            commands::change(arguments, run, format, workspace::create, render::change)
                .await
        }
        WorkspaceVerb::Rename => {
            commands::change(arguments, run, format, workspace::rename, render::change)
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
