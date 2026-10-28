//! `holst board`: boards and folders by id, link or part of the name.

mod render;
mod whole;

use holst_core::{
    command::{
        board::{
            BoardVerb,
            own,
            rights::{self, Shared},
            settings,
            tree,
            tree_change,
            tree_change::Changed,
        },
        table::{self, CommandArgs},
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
        BoardVerb::Create => change(arguments, run, format, tree_change::create).await,
        BoardVerb::Rename => change(arguments, run, format, tree_change::rename).await,
        BoardVerb::Mv => change(arguments, run, format, tree_change::mv).await,
        BoardVerb::Cp => change(arguments, run, format, tree_change::cp).await,
        BoardVerb::Rm => change(arguments, run, format, tree_change::rm).await,
        BoardVerb::Restore => {
            change(arguments, run, format, tree_change::restore).await
        }
        BoardVerb::Favorite => change(arguments, run, format, own::favorite).await,
        BoardVerb::Unfavorite => change(arguments, run, format, own::unfavorite).await,
        BoardVerb::Touch => change(arguments, run, format, own::touch).await,
        BoardVerb::Leave => change(arguments, run, format, own::leave).await,
        BoardVerb::Show => whole::show(arguments, run, format).await,
        BoardVerb::Snap => whole::snap(arguments, run, format).await,
        BoardVerb::Save => whole::save(arguments, run, format).await,
        BoardVerb::Export => whole::export(arguments, run, format).await,
        BoardVerb::Import => whole::import(arguments, run, format).await,
        BoardVerb::Edit => {
            let args = table::parse(arguments)?;
            run_signed_in(run, async |client| {
                let edited = settings::edit(client, run.cache(), &args).await?;
                render::edited(format, &edited)
            })
            .await
        }
        BoardVerb::Grant => share_with(arguments, run, format, rights::grant).await,
        BoardVerb::Revoke => share_with(arguments, run, format, rights::revoke).await,
        BoardVerb::Share => share_with(arguments, run, format, rights::share).await,
        BoardVerb::Transfer => {
            share_with(arguments, run, format, rights::transfer).await
        }
    }
}

/// Changes who may use a board and prints it. As with a tree change, the plan
/// of a change held for its confirmation is shown first.
async fn share_with<A: CommandArgs>(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
    make: impl AsyncFn(&Client, &A) -> holst_core::Result<Shared>,
) -> Result<()> {
    let args = table::parse::<A>(arguments)?;
    run_signed_in(run, async |client| {
        let shared = make(client, &args).await?;
        render::shared(format, &shared)?;
        refused(shared.refusal())
    })
    .await
}

/// Makes one change of the tree and prints it. The plan of a change held for
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
