//! `holst template`: the table's commands.

mod render;

use holst_core::{
    command::{
        table::{self, CommandArgs},
        template::{self, Changed, TemplateVerb},
    },
    remote::client::Client,
};
use serde_json::{Map, Value};

use crate::{
    commands::refused,
    error::Result,
    output::OutputFormat,
    run::{RunSettings, run_signed_in},
};

/// Runs a command of the `template` group with the arguments the command line
/// gave it.
pub(crate) async fn run(
    verb: TemplateVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match verb {
        TemplateVerb::Ls => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::listing(format, &template::ls(client, &args).await?)
            })
            .await
        }
        TemplateVerb::Show => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::shown(format, &template::show(client, &args).await?)
            })
            .await
        }
        TemplateVerb::Create => change(arguments, run, format, template::create).await,
        TemplateVerb::Edit => change(arguments, run, format, template::edit).await,
        TemplateVerb::Rm => change(arguments, run, format, template::rm).await,
    }
}

/// Makes one change of a template and prints it. The plan of a change held for
/// its confirmation is shown first; the refusal is the exit code.
async fn change<A: CommandArgs>(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
    make: impl AsyncFn(&Client, &A) -> holst_core::Result<Changed>,
) -> Result<()> {
    let args = table::parse::<A>(arguments)?;
    run_signed_in(run, async |client| {
        let changed = make(client, &args).await?;
        render::changed(format, &changed)?;
        refused(changed.outcome.refusal())
    })
    .await
}
