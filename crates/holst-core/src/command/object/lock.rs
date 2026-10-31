//! `object lock` and `object unlock`: keep objects as they are, free them.

use crate::{
    command::{
        board_write::Guard,
        object::{LockArgs, UnlockArgs, change::named, group::written},
        table::Validate,
    },
    domain::change::{receipt::ChangeReport, value::BoardChange},
    effect::settings::CacheSettings,
    error::{Result, board_change::BoardChangeError, flag::Flag, input::InputError},
    remote::client::Client,
};

const LOCK: &str = "object lock";
const UNLOCK: &str = "object unlock";

/// `--all` unlocks everything, so no object is named beside it.
impl Validate for UnlockArgs {
    fn validate(&self) -> std::result::Result<(), InputError> {
        if self.all && !self.object.is_empty() {
            Err(InputError::Conflict {
                flag: Flag::Object,
                other: Flag::All,
            })
        } else {
            Ok(())
        }
    }
}

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
        change,
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
        unlock_change(args)?,
        Some(LOCKING),
    )
    .await
}

/// What `args` unlock: everything, or the objects named.
fn unlock_change(args: &UnlockArgs) -> Result<BoardChange> {
    if args.all {
        Ok(BoardChange::UnlockAll)
    } else {
        named(&args.board, &args.object, UNLOCK).map(BoardChange::Unlock)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        Error,
        command::{fixture::arguments, object::UnlockArgs},
        error::{flag::Flag, input::InputError},
    };

    #[test]
    fn parse_refuses_all_beside_an_object() {
        let error = arguments::<UnlockArgs>(json!({ "all": true, "object": ["a1"] }));
        assert!(matches!(
            error,
            Err(Error::Input(InputError::Conflict {
                flag: Flag::Object,
                other: Flag::All
            }))
        ));
    }
}
