//! `object lock` and `object unlock`: keep objects as they are, free them.

use crate::{
    command::{
        object::{
            LockArgs,
            UnlockArgs,
            change::{Guard, named},
            group::written,
        },
        table::not_value,
    },
    domain::change::{receipt::ChangeReport, value::BoardChange},
    effect::settings::CacheSettings,
    error::{Result, board_change::BoardChangeError},
    remote::client::Client,
};

const LOCK: &str = "object lock";
const UNLOCK: &str = "object unlock";

/// Locking needs the role `who_can_manage_lock_content` asks for.
const LOCKING: Guard = ("who_can_manage_lock_content", |needs, holds| {
    BoardChangeError::LockWithheld { needs, holds }
});

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
    let objects = named(&args.board, &args.object, LOCK)?;
    let change = if args.only {
        BoardChange::LockOnly(objects)
    } else {
        BoardChange::Lock(objects)
    };
    written(
        client,
        cache,
        args.dry_run,
        (&args.board, args.workspace.as_deref()),
        &change,
        Some(LOCKING),
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
    written(
        client,
        cache,
        args.dry_run,
        (&args.board, args.workspace.as_deref()),
        &unlock_change(args)?,
        Some(LOCKING),
    )
    .await
}

/// What `args` unlock: everything, or the objects named.
fn unlock_change(args: &UnlockArgs) -> Result<BoardChange> {
    match (args.all, args.object.is_empty()) {
        (false, _) => named(&args.board, &args.object, UNLOCK).map(BoardChange::Unlock),
        (true, true) => Ok(BoardChange::UnlockAll),
        (true, false) => Err(not_value(UNLOCK, "object", "left out with all")),
    }
}
