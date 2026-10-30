//! `holst board`: boards and folders by id, link or part of the name.

mod render;
mod whole;

use holst_core::command::{
    board::{BoardVerb, own, rights, settings, tree, tree_change},
    table,
};
use serde_json::{Map, Value};

use crate::{
    commands::change,
    error::Result,
    output::OutputFormat,
    run::{RunSettings, run_signed_in},
};

pub(crate) async fn run(
    verb: BoardVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    match verb {
        BoardVerb::Ls => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                let listing = tree::ls(client, &args).await?;
                render::listing(format, &listing, |id| client.board_link(id))
            })
            .await
        }
        BoardVerb::Create => {
            change(arguments, run, format, tree_change::create, render::changed).await
        }
        BoardVerb::Rename => {
            change(arguments, run, format, tree_change::rename, render::changed).await
        }
        BoardVerb::Mv => {
            change(arguments, run, format, tree_change::mv, render::changed).await
        }
        BoardVerb::Cp => {
            change(arguments, run, format, tree_change::cp, render::changed).await
        }
        BoardVerb::Rm => {
            change(arguments, run, format, tree_change::rm, render::changed).await
        }
        BoardVerb::Restore => {
            change(
                arguments,
                run,
                format,
                tree_change::restore,
                render::changed,
            )
            .await
        }
        BoardVerb::Favorite => {
            change(arguments, run, format, own::favorite, render::changed).await
        }
        BoardVerb::Unfavorite => {
            change(arguments, run, format, own::unfavorite, render::changed).await
        }
        BoardVerb::Touch => {
            change(arguments, run, format, own::touch, render::changed).await
        }
        BoardVerb::Leave => {
            change(arguments, run, format, own::leave, render::changed).await
        }
        BoardVerb::Show => whole::show(arguments, run, format).await,
        BoardVerb::Snap => whole::snap(arguments, run, format).await,
        BoardVerb::Save => whole::save(arguments, run, format).await,
        BoardVerb::Export => whole::export(arguments, run, format).await,
        BoardVerb::Import => whole::import(arguments, run, format).await,
        BoardVerb::Edit => {
            change(
                arguments,
                run,
                format,
                async |client, args| settings::edit(client, run.cache(), args).await,
                render::edited,
            )
            .await
        }
        BoardVerb::Grant => {
            change(arguments, run, format, rights::grant, render::shared).await
        }
        BoardVerb::Revoke => {
            change(arguments, run, format, rights::revoke, render::shared).await
        }
        BoardVerb::Share => {
            change(arguments, run, format, rights::share, render::shared).await
        }
        BoardVerb::Transfer => {
            change(arguments, run, format, rights::transfer, render::shared).await
        }
    }
}
