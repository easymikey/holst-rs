//! `object align`: line objects up against a side, or spread them evenly.
use strum::VariantNames as _;

use crate::{
    command::{
        board::show::account_cache,
        board_load::CacheUse,
        object::{AlignArgs, change::named, group::sent},
        table::not_value,
    },
    domain::change::{
        receipt::ChangeReport,
        value::{Alignment, Arrangement, BoardChange},
    },
    effect::{
        board_write::WriteInterpreter,
        cache::CachePort,
        settings::CacheSettings,
    },
    error::Result,
    remote::{client::Client, socket::write::interpreter},
    storage::cache::Cache,
};

const ALIGN: &str = "object align";

/// Aligns the objects to one side of the box they cover together, or spreads
/// them so that the gaps between are equal; each moves with what it holds.
///
/// # Errors
/// No object, an object that is not found, neither or both of a side and a
/// spread, a locked object, a table cell, a mind-map node or a card in a kanban
/// column, too few objects that can be moved, a board that is not found or not
/// editable, and errors of the requests and of the write.
pub async fn align(
    client: &Client,
    cache: &CacheSettings,
    args: &AlignArgs,
) -> Result<ChangeReport> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    aligned(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn aligned<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &AlignArgs,
) -> Result<ChangeReport> {
    let how = arrangement(args)?;
    let objects = named(&args.board, &args.object, ALIGN)?;
    sent(
        client,
        cache,
        interpreter,
        (&args.board, args.workspace.as_deref()),
        &BoardChange::Arrange { objects, how },
    )
    .await
}

/// What `args` ask: a side, or a direction to spread in, not both.
fn arrangement(args: &AlignArgs) -> Result<Arrangement> {
    match (&args.to, args.distribute) {
        (Some(side), None) => side
            .trim()
            .parse::<Alignment>()
            .map(Arrangement::Align)
            .map_err(|_| not_value(ALIGN, "to", &Alignment::VARIANTS.join(", "))),
        (None, Some(axis)) => Ok(Arrangement::Distribute(axis)),
        (None, None) => Err(not_value(
            ALIGN,
            "to",
            "left, center, right, top, middle or bottom, unless --distribute is given",
        )),
        (Some(_), Some(_)) => Err(not_value(ALIGN, "distribute", "left out with --to")),
    }
}
