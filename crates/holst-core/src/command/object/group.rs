//! `object group` and `object ungroup`: make a group, dissolve one.

use crate::{
    command::{
        board::show::{account_cache, board_id},
        board_load::CacheUse,
        object::{
            GroupArgs,
            UngroupArgs,
            change::{Sending, named, reported, send},
        },
    },
    domain::change::{receipt::ChangeReport, value::BoardChange},
    effect::{
        board_write::WriteInterpreter,
        cache::CachePort,
        confirmation::ALWAYS,
        settings::CacheSettings,
    },
    error::Result,
    remote::{client::Client, socket::write::interpreter},
    storage::cache::Cache,
};

const GROUP: &str = "object group";
const UNGROUP: &str = "object ungroup";

/// Puts the objects into one new group, as one write.
///
/// # Errors
/// No object, an object that is not found, fewer than two objects that can be
/// grouped, an object of a kind the web client keeps out of groups or already
/// in one, a board that is not found or not editable, and errors of the
/// requests and of the write.
pub async fn group(
    client: &Client,
    cache: &CacheSettings,
    args: &GroupArgs,
) -> Result<ChangeReport> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    grouped(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn grouped<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &GroupArgs,
) -> Result<ChangeReport> {
    let objects = named(&args.board, &args.object, GROUP)?;
    sent(
        client,
        cache,
        interpreter,
        (&args.board, args.workspace.as_deref()),
        &BoardChange::Group(objects),
    )
    .await
}

/// Dissolves the groups: their objects stay where they are.
///
/// # Errors
/// No object, an object that is not found or not a group, a locked group, a
/// board that is not found or not editable, and errors of the requests and of
/// the write.
pub async fn ungroup(
    client: &Client,
    cache: &CacheSettings,
    args: &UngroupArgs,
) -> Result<ChangeReport> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    ungrouped(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn ungrouped<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &UngroupArgs,
) -> Result<ChangeReport> {
    let objects = named(&args.board, &args.object, UNGROUP)?;
    sent(
        client,
        cache,
        interpreter,
        (&args.board, args.workspace.as_deref()),
        &BoardChange::Ungroup(objects),
    )
    .await
}

/// Writes `change` to the board `(name, workspace)` names.
pub(super) async fn sent<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    (name, workspace): (&str, Option<&str>),
    change: &BoardChange,
) -> Result<ChangeReport> {
    let board = board_id(client, name, workspace, CacheUse::Use).await?;
    let written = send(
        client,
        cache,
        interpreter,
        Sending {
            board: &board,
            change,
            release: ALWAYS,
        },
    )
    .await?;
    Ok(reported(client, &board, &written))
}
