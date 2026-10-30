//! `holst comment`: the table's commands.

mod render;

use holst_core::command::{
    comment::{self, CommentReport, CommentVerb},
    table,
};
use serde_json::{Map, Value};

use crate::{
    commands::refused,
    error::Result,
    output::OutputFormat,
    run::{RunSettings, run_signed_in},
};

pub(crate) async fn run(
    verb: CommentVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match verb {
        CommentVerb::Ls => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                render::listing(format, &comment::ls(client, &args).await?)
            })
            .await
        }
        CommentVerb::Add => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                shown(format, &comment::add(client, run.cache(), &args).await?)
            })
            .await
        }
        CommentVerb::Reply => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                shown(format, &comment::reply(client, &args).await?)
            })
            .await
        }
        CommentVerb::Resolve => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                shown(format, &comment::resolve(client, &args).await?)
            })
            .await
        }
        CommentVerb::Rm => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                shown(format, &comment::rm(client, &args).await?)
            })
            .await
        }
        CommentVerb::Read => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                shown(format, &comment::read(client, &args).await?)
            })
            .await
        }
    }
}

/// Prints the change; one held for its confirmation is the exit code after.
fn shown(format: OutputFormat, changed: &CommentReport) -> Result<()> {
    render::change(format, changed)?;
    refused(changed.outcome.refusal())
}
