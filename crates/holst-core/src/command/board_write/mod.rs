//! Board write: load the board, plan the change on it, carry it out.

#[cfg(test)] mod tests;

use futures_util::future::{self, OptionFuture};
use serde_json::Value;
use yrs::Doc;

use crate::{
    command::{
        account,
        board::show::Loading,
        board_load::{
            DocumentRecord,
            channel_or_get,
            forget,
            guarded_loss,
            stored,
            trusted_seed,
            withheld,
        },
    },
    domain::{
        board::EditRight,
        change::{
            receipt::{BoardStatus, ChangeReport, Delivery},
            value::{BoardChange, ChangeSources, RemoveRight},
        },
        node::NodeBrief,
        time::Timestamp,
    },
    effect::{
        board_write::{LoadSource, WriteMode},
        board_write_plan::{LoadedChange, WrittenChange, plan},
        cache::{CachePort, meta::ReadCheck},
        confirmation::ReleaseCheck,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        board_socket::BoardSocketError,
    },
    remote::{
        api,
        client::Client,
        socket::write::{WriteInterpreter, WriteOver, writer_document},
    },
};

/// A right a write needs: its `who_can_*` setting and the error for a refusal.
pub(in crate::command) type Guard =
    (&'static str, fn(String, &'static str) -> BoardChangeError);

/// Refuses on `board` when the setting of `guard` stands above the role the
/// caller's permits give there, as the web client does.
pub(in crate::command) async fn refuse_withheld(
    client: &Client,
    board: &str,
    guard: Guard,
) -> Result<()> {
    refuse_in(&api::channel::get(client, board).await?, guard)
}

/// As [`refuse_withheld`], over a `channel/{id}` reply already read.
pub(in crate::command) fn refuse_in(
    channel: &Value,
    (setting, refused): Guard,
) -> Result<()> {
    withheld(channel, setting).map_or(Ok(()), |(needs, holds)| {
        Err(Error::BoardChange(refused(needs, holds)))
    })
}

/// One change to one board, as a command asks for it.
pub(in crate::command) struct BoardWrite<'a> {
    /// The id; the command finds it.
    pub(in crate::command) board: &'a str,
    pub(in crate::command) change: BoardChange,
    pub(in crate::command) release: ReleaseCheck,
    /// A right checked on `board` before anything else.
    pub(in crate::command) guard: Option<Guard>,
    pub(in crate::command) mode: WriteMode,
}

/// Checks the guard, writes as the logged-in account through the cache of
/// `loading` (sent, or only built), and reports each object with its link.
pub(in crate::command) async fn change_board(
    client: &Client,
    loading: Loading<'_>,
    write: BoardWrite<'_>,
) -> Result<ChangeReport> {
    change_board_with(client, loading, write, None).await
}

/// As [`change_board`], given the `channel/{id}` reply of `write.board` when
/// the caller read it. A guard asks for the reply once, and the write takes it.
pub(in crate::command) async fn change_board_with(
    client: &Client,
    loading: Loading<'_>,
    write: BoardWrite<'_>,
    known: Option<Value>,
) -> Result<ChangeReport> {
    let BoardWrite {
        board,
        change,
        release,
        guard,
        mode,
    } = write;
    let known = match guard {
        Some(guard) => {
            let channel = channel_or_get(client, board, known).await?;
            refuse_in(&channel, guard)?;
            Some(channel)
        }
        None => known,
    };
    let account_id = account::account_id(client).await?;
    let over = match mode {
        WriteMode::Send => WriteOver::sender(loading.connector),
        WriteMode::DryRun => WriteOver::reporter(loading.connector),
    };
    let changed = write_cached_known(
        client,
        &over,
        WriteRequest {
            board,
            change: &change,
            send_if: release,
            account_id: &account_id,
            kept: None,
        },
        loading.cache,
        known,
    )
    .await
    .map_err(Error::from)?;
    let base = client.base_url();
    Ok(changed
        .change
        .written
        .report(board, |object| base.object_link(board, object)))
}

/// Whether a write started from a kept or cached copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seeding {
    Seeded,
    Unseeded,
}

/// A change to make to a board by a user, from the caller's copy if any.
#[derive(Debug, Clone)]
pub struct WriteRequest<'a> {
    pub board: &'a str,
    pub change: &'a BoardChange,
    pub send_if: ReleaseCheck,
    pub account_id: &'a str,
    pub kept: Option<Doc>,
}

/// A change with the document it was built on and how the board loaded.
#[derive(Debug, Clone)]
pub struct BuiltChange {
    pub written: WrittenChange,
    pub document: Doc,
    pub source: LoadSource,
}

/// A failed change; `lost` is whether the board turned out closed to the user.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct NotChanged {
    pub error: Error,
    pub lost: bool,
}

impl From<Error> for NotChanged {
    fn from(error: Error) -> Self {
        Self { error, lost: false }
    }
}

impl From<NotChanged> for Error {
    fn from(failed: NotChanged) -> Self {
        failed.error
    }
}

/// A change and how it started.
#[derive(Debug, Clone)]
pub struct ChangedBoard {
    pub change: BuiltChange,
    pub seeding: Seeding,
}

/// Load the board through `interpreter`, plan the request's operation on it
/// and carry the plan out; `known` is the board's `channel/{id}` reply if the
/// caller has it, and then none is asked.
async fn write<I: WriteInterpreter + ?Sized>(
    client: &Client,
    interpreter: &I,
    request: WriteRequest<'_>,
    known: Option<Value>,
) -> Result<BuiltChange> {
    let WriteRequest {
        board,
        change,
        send_if,
        account_id,
        kept,
    } = request;
    let seed = kept.map_or_else(|| writer_document(client, None), Ok)?;
    let asking = known
        .is_none()
        .then(|| Box::pin(api::channel::get(client, board)));
    let (asked, opened) = future::join(
        OptionFuture::from(asking),
        interpreter.open(client, board, seed),
    )
    .await;
    let reply = known.or_else(|| asked.and_then(Result::ok));
    let trashed = reply
        .as_ref()
        .and_then(|reply| NodeBrief::of(reply).ok())
        .is_some_and(|node| node.in_trash());
    // From the reply asked for above; with none, nothing is withheld.
    let removal = if reply
        .as_ref()
        .and_then(|reply| withheld(reply, "who_can_delete_content"))
        .is_some()
    {
        RemoveRight::OwnOnly
    } else {
        RemoveRight::Anyone
    };
    let placement = if trashed {
        BoardStatus::InTrash
    } else {
        BoardStatus::Live
    };
    let opened = opened?;
    let context = ChangeSources {
        built_at: Timestamp::from_millis(client.unix_millis()),
        ids: client.ids(),
    };
    let planned = plan(&LoadedChange {
        document: &opened.document,
        right: opened.right,
        placement,
        change,
        removal,
        account_id,
        context: &context,
        release_check: send_if,
    })?;
    let accepted = interpreter
        .carry(client, opened.connection, &planned)
        .await?;
    Ok(BuiltChange {
        written: planned.written(accepted),
        document: opened.document,
        source: opened.source,
    })
}

/// Change the board starting from the kept copy, else the cached one; after
/// the ack the changed document is cached. A copy ahead of the server goes
/// before anything is sent, and the change starts from the server's document.
///
/// # Errors
/// [`crate::error::board_change::BoardChangeError::ReadOnly`], the errors of
/// [`crate::domain::change::build::build`], the load's errors,
/// [`crate::error::board_change::BoardChangeError::AcceptTimeout`]
/// or [`crate::error::board_change::BoardChangeError::ClosedBeforeAccept`],
/// with [`NotChanged::lost`].
pub async fn write_cached<I: WriteInterpreter + ?Sized>(
    client: &Client,
    interpreter: &I,
    request: WriteRequest<'_>,
    cache: Option<&dyn CachePort>,
) -> std::result::Result<ChangedBoard, NotChanged> {
    write_cached_known(client, interpreter, request, cache, None).await
}

/// As [`write_cached`], given the board's `channel/{id}` reply if the caller
/// has it.
async fn write_cached_known<I: WriteInterpreter + ?Sized>(
    client: &Client,
    interpreter: &I,
    request: WriteRequest<'_>,
    cache: Option<&dyn CachePort>,
    known: Option<Value>,
) -> std::result::Result<ChangedBoard, NotChanged> {
    let WriteRequest {
        board,
        change,
        send_if,
        account_id,
        kept,
    } = request;
    let seed = kept.or_else(|| {
        cache.and_then(|cache| {
            trusted_seed(cache, board, client.unix_millis(), |state| {
                writer_document(client, Some(state))
            })
        })
    });
    let had_seed = seed.as_ref().map_or(Seeding::Unseeded, |_| Seeding::Seeded);
    let starting = |from: Option<Doc>| {
        Ok::<_, Error>(WriteRequest {
            board,
            change,
            send_if,
            account_id,
            kept: Some(from.map_or_else(|| writer_document(client, None), Ok)?),
        })
    };
    let first = write(client, interpreter, starting(seed)?, known.clone()).await;
    let (changed, had_seed) = match first {
        Err(Error::BoardSocket(BoardSocketError::CopyAhead)) => {
            tracing::debug!(board, "board cache: the copy is ahead of the server");
            OptionFuture::from(cache.map(|cache| forget(cache, board))).await;
            let retried = write(client, interpreter, starting(None)?, known);
            (retried.await, Seeding::Unseeded)
        }
        other => (other, had_seed),
    };
    let (changed, lost) = guarded_loss(client, cache, board, changed).await;
    let changed = changed.map_err(|error| NotChanged { error, lost })?;
    Ok(cached_change(cache, board, changed, had_seed).await)
}

async fn cached_change(
    cache: Option<&dyn CachePort>,
    board: &str,
    changed: BuiltChange,
    had_seed: Seeding,
) -> ChangedBoard {
    let accepted = changed.written.delivery == Delivery::Accepted;
    let whole =
        had_seed == Seeding::Unseeded || !matches!(changed.source, LoadSource::Skipped);
    let storing = cache.filter(|_| accepted).map(|cache| {
        let snapshot = DocumentRecord {
            doc: &changed.document,
            access: EditRight::Editable,
            downloaded: whole,
            read_check: ReadCheck::Verified,
        };
        stored(cache, board, snapshot)
    });
    OptionFuture::from(storing).await;
    ChangedBoard {
        change: changed,
        seeding: had_seed,
    }
}
