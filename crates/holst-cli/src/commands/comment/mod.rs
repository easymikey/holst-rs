//! `holst comment`: the table's commands.

mod render;

use holst_core::command::{
    comment::{self, CommentVerb},
    table,
};
use serde_json::{Map, Value};

use crate::{
    commands,
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
            commands::change(
                arguments,
                run,
                format,
                async |client, args| comment::add(client, run.cache(), args).await,
                render::change,
            )
            .await
        }
        CommentVerb::Reply => {
            commands::change(arguments, run, format, comment::reply, render::change)
                .await
        }
        CommentVerb::Resolve => {
            commands::change(arguments, run, format, comment::resolve, render::change)
                .await
        }
        CommentVerb::Rm => {
            commands::change(arguments, run, format, comment::rm, render::change).await
        }
        CommentVerb::Read => {
            commands::change(arguments, run, format, comment::read, render::change)
                .await
        }
    }
}
