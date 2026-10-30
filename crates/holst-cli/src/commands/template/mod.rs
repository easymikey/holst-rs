//! `holst template`: the table's commands.

mod render;

use holst_core::command::{
    table,
    template::{self, TemplateVerb},
};
use serde_json::{Map, Value};

use crate::{
    commands::change,
    error::Result,
    output::OutputFormat,
    run::{RunSettings, run_signed_in},
};

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
        TemplateVerb::Create => {
            change(arguments, run, format, template::create, render::changed).await
        }
        TemplateVerb::Edit => {
            change(arguments, run, format, template::edit, render::changed).await
        }
        TemplateVerb::Rm => {
            change(arguments, run, format, template::rm, render::changed).await
        }
    }
}
