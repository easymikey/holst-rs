//! `object lock` and `object unlock`: keep objects as they are, free them.

use crate::{
    command::{
        board::show::{account_cache, board_id},
        board_load::{CacheUse, withheld},
        object::{
            LockArgs,
            UnlockArgs,
            change::{Sending, named, reported, send},
        },
        table::not_value,
    },
    domain::change::{receipt::ChangeReport, value::BoardChange},
    effect::{
        board_write::WriteInterpreter,
        cache::CachePort,
        confirmation::ALWAYS,
        settings::CacheSettings,
    },
    error::{Error, Result, board_change::BoardChangeError},
    remote::{api, client::Client, socket::write::interpreter},
    storage::cache::Cache,
};

const LOCK: &str = "object lock";
const UNLOCK: &str = "object unlock";

/// Locks the objects: into one new locked group, or, with `only`, the flag on
/// each frame and table.
///
/// # Errors
/// No object, an object that is not found, locked already or of a kind the web
/// client does not lock, a board that withholds locking from the caller, is not
/// found or not editable, and errors of the requests and of the write.
pub async fn lock(
    client: &Client,
    cache: &CacheSettings,
    args: &LockArgs,
) -> Result<ChangeReport> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    locked(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn locked<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &LockArgs,
) -> Result<ChangeReport> {
    let objects = named(&args.board, &args.object, LOCK)?;
    let change = if args.only {
        BoardChange::LockOnly(objects)
    } else {
        BoardChange::Lock(objects)
    };
    sent(
        client,
        cache,
        interpreter,
        (&args.board, args.workspace.as_deref()),
        &change,
    )
    .await
}

/// Unlocks the named groups, frames and tables, or all that are locked.
///
/// # Errors
/// Neither an object nor `--all`, both, an object that is not found or not
/// locked itself, nothing locked under `--all`, a board that withholds locking
/// from the caller, is not found or not editable, and errors of the requests
/// and of the write.
pub async fn unlock(
    client: &Client,
    cache: &CacheSettings,
    args: &UnlockArgs,
) -> Result<ChangeReport> {
    let account = account_cache(client, cache, CacheUse::Use).await?;
    unlocked(
        client,
        account.as_ref().map(Cache::as_port),
        interpreter(args.dry_run),
        args,
    )
    .await
}

async fn unlocked<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    args: &UnlockArgs,
) -> Result<ChangeReport> {
    let change = unlock_change(args)?;
    sent(
        client,
        cache,
        interpreter,
        (&args.board, args.workspace.as_deref()),
        &change,
    )
    .await
}

/// What `args` unlock: everything, or the objects named.
fn unlock_change(args: &UnlockArgs) -> Result<BoardChange> {
    match (args.all, args.object.is_empty()) {
        (false, _) => named(&args.board, &args.object, UNLOCK).map(BoardChange::Unlock),
        (true, true) => Ok(BoardChange::UnlockAll),
        (true, false) => Err(not_value(UNLOCK, "object", "left out with --all")),
    }
}

/// Writes `change` to the board `(name, workspace)` names, if its
/// `who_can_manage_lock_content` is not above the caller's role.
async fn sent<I: WriteInterpreter + ?Sized>(
    client: &Client,
    cache: Option<&dyn CachePort>,
    interpreter: &I,
    (name, workspace): (&str, Option<&str>),
    change: &BoardChange,
) -> Result<ChangeReport> {
    let board = board_id(client, name, workspace, CacheUse::Use).await?;
    refuse_withheld(client, &board).await?;
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

/// Refuses locking on `board` when its `who_can_manage_lock_content` stands
/// above the role the caller's permits give there, as the web client does.
async fn refuse_withheld(client: &Client, board: &str) -> Result<()> {
    let channel = api::channel::get(client, board).await?;
    withheld(&channel, "who_can_manage_lock_content").map_or(
        Ok(()),
        |(needs, holds)| {
            Err(Error::BoardChange(BoardChangeError::LockWithheld {
                needs,
                holds,
            }))
        },
    )
}
