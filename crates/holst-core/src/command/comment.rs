//! The `comment` group: the threads of a board over its socket room.

use std::{borrow::Cow, collections::HashSet, time::Duration};

use futures_util::future;
use holst_macros::CommandArgs;
use serde::{Deserialize, Serialize};

use crate::{
    command::{
        account::account_id,
        board::show::{Loading, through_cache},
        board_load::{self, CacheUse, RemoteBoard},
        change::{ChangeReport, gate},
        lookup,
        table::{Command, CommandArgs, Confirmation, EnumArgument, not_value, visible},
    },
    domain::{
        board::{
            EditRight,
            comment::{
                CommentChange,
                NewThreadMessage,
                Thread,
                ThreadAnchor,
                ThreadMessage,
                short_id,
            },
            objects,
        },
        lookup::rules::find_id,
        text::{one_line, shortened},
        time::Timestamp,
    },
    effect::{board_write::WriteMode, settings::CacheSettings},
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
        lookup::{
            Candidate,
            Candidates,
            LookupError,
            LookupKind,
            LookupMiss,
            SkippedWorkspaces,
        },
    },
    remote::{
        api::{self, comment::SetReadAt},
        client::Client,
        socket::comments::{CommentRoom, open},
    },
};

/// The verbs of `holst comment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum CommentVerb {
    Ls,
    Add,
    Reply,
    Resolve,
    Edit,
    Rm,
    Read,
}

impl CommentVerb {
    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            CommentVerb::Ls => &LsArgs::COMMAND,
            CommentVerb::Add => &AddArgs::COMMAND,
            CommentVerb::Reply => &ReplyArgs::COMMAND,
            CommentVerb::Resolve => &ResolveArgs::COMMAND,
            CommentVerb::Edit => &EditArgs::COMMAND,
            CommentVerb::Rm => &RemoveArgs::COMMAND,
            CommentVerb::Read => &ReadArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "ls",
    risk = Read,
    about = "List the board's threads and when each was read",
    help = "List the board's comment threads and when each was read.\n\
            It prints one line per thread with its thread id and @N.",
    example = "holst comment ls \"Roadmap\"",
    example = "holst comment ls \"Roadmap\" -o json"
)]
pub struct LsArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "add",
    risk = Write,
    about = "Start a comment thread",
    help = "Start a comment thread, at a corner or on an object.\n\
            It prints the new thread with its thread id.",
    example = "holst comment add \"Roadmap\" \"Is Q3 final?\" --object a1b2c3d4",
    example = "holst comment add \"Roadmap\" \"Move this left\" --at 120,80"
)]
pub struct AddArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(positional, help = "The comment")]
    pub text: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "x,y",
        help = "The corner of the thread in board units; a free corner by default"
    )]
    pub at: Option<String>,
    #[argument(
        value_name = "object",
        help = "The object the thread sits on: id, link or the first 8 characters of its id; not with --at"
    )]
    pub object: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "reply",
    risk = Write,
    about = "Reply in a thread",
    help = "Reply in a comment thread.\n\
            It prints the reply with the thread's thread id.",
    example = "holst comment reply \"Roadmap\" weVza \"Yes, it is final\"",
    example = "holst comment reply \"Roadmap\" @2 \"Checked\" -n"
)]
pub struct ReplyArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(
        positional,
        help = "The thread: its thread id, e.g. weVza, or @N from `holst comment ls`",
        tool_doc = "E.g. weVza."
    )]
    pub thread: String,
    #[argument(positional, help = "The reply")]
    pub text: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "resolve",
    risk = Write,
    about = "Resolve a thread, or reopen it",
    help = "Resolve a comment thread, or with --reopen open it again.\n\
            It prints the thread and its new state.",
    example = "holst comment resolve \"Roadmap\" weVza",
    example = "holst comment resolve \"Roadmap\" weVza --reopen"
)]
pub struct ResolveArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(
        positional,
        help = "The thread: its thread id, e.g. weVza, or @N from `holst comment ls`",
        tool_doc = "E.g. weVza."
    )]
    pub thread: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(help = "Open a resolved thread again")]
    pub reopen: bool,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// The colors of a thread, as the web client's palette names them, and `none`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ThreadColor {
    White1,
    Red10,
    Amber10,
    Yellow10,
    Lime10,
    Green10,
    Blue10,
    Indigo10,
    Pink10,
    Violet10,
    Gray12,
    Gray7,
    Red6,
    Amber5,
    Yellow5,
    Lime5,
    Green6,
    Blue6,
    Indigo6,
    Pink6,
    Violet6,
    Gray8,
    #[serde(rename = "none")]
    #[strum(serialize = "none")]
    Clear,
}

impl EnumArgument for ThreadColor {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "edit",
    risk = Write,
    about = "Colour a thread or move it",
    help = "Give a comment thread a color, or move it to a point or onto an object; one of --color, --at and --object is needed.\n\
            It prints the thread and what changed.",
    example = "holst comment edit \"Roadmap\" weVza --color red10",
    example = "holst comment edit \"Roadmap\" weVza --at 120,80",
    example = "holst comment edit \"Roadmap\" @2 --object a1b2c3d4 --color none"
)]
pub struct EditArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(
        positional,
        help = "The thread: its thread id, e.g. weVza, or @N from `holst comment ls`",
        tool_doc = "E.g. weVza."
    )]
    pub thread: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        help = "The color of the thread, or none to clear it",
        tool_doc = "Thread colour, or none."
    )]
    pub color: Option<ThreadColor>,
    #[argument(
        value_name = "x,y",
        help = "Move the thread to this corner in board units; not with --object"
    )]
    pub at: Option<String>,
    #[argument(
        value_name = "object",
        help = "Move the thread onto this object: id, link or the first 8 characters of its id; not with --at"
    )]
    pub object: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "rm",
    risk = Destroy,
    about = "Delete a thread with its replies, or one reply",
    help = "Delete a comment thread with its replies, or with --reply only one reply of your own.\n\
            It removes other people's replies for good, so it needs -y; without it only the dry run is shown.",
    example = "holst comment rm \"Roadmap\" weVza -y",
    example = "holst comment rm \"Roadmap\" weVza --reply 2 -y",
    example = "holst comment rm \"Roadmap\" @2 -n"
)]
pub struct RemoveArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(
        positional,
        help = "The thread: its thread id, e.g. weVza, or @N from `holst comment ls`",
        tool_doc = "E.g. weVza."
    )]
    pub thread: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "reply",
        help = "Delete only this reply, one of your own: its message id (`comment ls -o json`) or N, the Nth reply from 1",
        tool_doc = "Message id or N, the Nth reply from 1."
    )]
    pub reply: Option<String>,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; a terminal asks instead; without one only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Comment,
    verb = "read",
    risk = Write,
    about = "Mark a thread as read",
    help = "Mark a comment thread as read.\n\
            It prints the thread and the time it was marked.",
    example = "holst comment read \"Roadmap\" weVza",
    example = "holst comment read \"Roadmap\" weVza --time 2026-10-27T09:30:00Z"
)]
pub struct ReadArgs {
    #[argument(positional, help = "The board")]
    pub board: String,
    #[argument(
        positional,
        help = "The thread: its thread id, e.g. weVza, or @N from `holst comment ls`",
        tool_doc = "E.g. weVza."
    )]
    pub thread: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "time",
        help = "When it was read, in ISO 8601; now by default",
        tool_doc = "ISO 8601; now by default."
    )]
    pub time: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// A thread with the time the signed-in user read it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListedThread {
    #[serde(flatten)]
    pub thread: Thread,
    /// ISO 8601; none while the user never read it.
    pub read_at: Option<String>,
}

/// The threads of a board, oldest first: `@N` counts from 1 in this order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadList {
    pub board: String,
    pub threads: Vec<ListedThread>,
}

/// What a change did to its thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentAction {
    Add,
    Reply,
    Resolve,
    Reopen,
    Edit,
    Delete,
    DeleteReply,
    Read,
}

/// What the write of a change would do, for a dry run or a held change.
#[derive(Debug, Serialize)]
pub struct CommentPlan {
    pub action: CommentAction,
    pub thread: String,
}

/// A change of a thread: what it was and whether it went out.
#[derive(Debug, Serialize)]
pub struct CommentReport {
    pub action: CommentAction,
    /// The id of the board.
    pub board: String,
    pub thread: String,
    /// The id of the message a start or a reply writes.
    pub message: Option<String>,
    /// When `read` marked the thread, ISO 8601.
    pub read_at: Option<String>,
    /// What `edit` changes: `color`, `place`.
    pub edited: Vec<&'static str>,
    #[serde(flatten)]
    pub outcome: ChangeReport<CommentPlan>,
}

impl CommentReport {
    /// A sent change of a thread, before the board and the outcome are known.
    fn of(action: CommentAction, thread: String, message: Option<String>) -> Self {
        Self {
            action,
            board: String::new(),
            thread,
            message,
            read_at: None,
            edited: Vec::new(),
            outcome: ChangeReport::Sent,
        }
    }

    /// The change in a few words: what it did, or what a dry run or a held
    /// change would do.
    #[must_use]
    pub fn phrase(&self) -> String {
        let reply = self.message.as_deref().unwrap_or_default();
        let (done, plan) = match self.action {
            CommentAction::Add => ("started", "start"),
            CommentAction::Reply => ("replied in", "reply in"),
            CommentAction::Resolve => ("resolved", "resolve"),
            CommentAction::Reopen => ("reopened", "reopen"),
            CommentAction::Edit => ("changed", "change"),
            CommentAction::Delete | CommentAction::DeleteReply => ("deleted", "delete"),
            CommentAction::Read => ("marked", "mark"),
        };
        let tail = match self.action {
            CommentAction::Read => " read".to_owned(),
            CommentAction::Edit => format!(": {}", self.edited.join(", ")),
            CommentAction::Add
            | CommentAction::Reply
            | CommentAction::Resolve
            | CommentAction::Reopen
            | CommentAction::Delete
            | CommentAction::DeleteReply => String::new(),
        };
        let thread = &self.thread;
        let what = match self.action {
            CommentAction::DeleteReply => format!("reply {reply} in thread {thread}"),
            CommentAction::Add
            | CommentAction::Reply
            | CommentAction::Resolve
            | CommentAction::Reopen
            | CommentAction::Edit
            | CommentAction::Delete
            | CommentAction::Read => format!("thread {thread}"),
        };
        match self.outcome {
            ChangeReport::Sent => format!("{done} {what}{tail}"),
            ChangeReport::DryRun { .. } | ChangeReport::Held { .. } => {
                format!("would {plan} {what}{tail}")
            }
        }
    }
}

impl ThreadList {
    /// A line per thread: its `@N`, id, state, start of the first message,
    /// replies and read mark.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.threads
            .iter()
            .zip(1_u32..)
            .map(|(listed, number)| listed.line(number))
            .collect()
    }
}

impl ListedThread {
    fn line(&self, number: u32) -> String {
        let thread = &self.thread;
        let state = if thread.resolved { "resolved" } else { "open" };
        let start = thread
            .messages
            .first()
            .map(|message| shortened(&one_line(&message.text), 60))
            .unwrap_or_default();
        let replies = match thread.messages.len().saturating_sub(1) {
            0 => "no replies".to_owned(),
            1 => "1 reply".to_owned(),
            more => format!("{more} replies"),
        };
        let read = self
            .read_at
            .as_deref()
            .map_or_else(|| "unread".to_owned(), |at| format!("read {at}"));
        let place = match thread.anchor {
            ThreadAnchor::Area {
                x,
                y,
                width,
                height,
            } => format!("  area {x},{y} {width}×{height}"),
            ThreadAnchor::Point { .. } | ThreadAnchor::Object { .. } => String::new(),
        };
        format!(
            "@{number}  {}  {state}{place}  {start}  ({replies}, {read})",
            thread.id
        )
    }
}

/// The board of a command with its threads loaded and the user's id.
struct Opened {
    board: String,
    room: CommentRoom,
    threads: Vec<Thread>,
    me: String,
}

async fn opened(
    client: &Client,
    board: &str,
    workspace: Option<&str>,
) -> Result<Opened> {
    let board = lookup::board_id(client, board, workspace).await?;
    let (room, user) =
        future::try_join(open(client, &board), account_id(client)).await?;
    Ok(Opened {
        board,
        threads: room.threads(),
        me: user,
        room,
    })
}

/// Lists the threads of a board with the time the user read each.
///
/// # Errors
/// A board that is not found, a refused room, and errors of the requests.
pub async fn ls(client: &Client, args: &LsArgs) -> Result<ThreadList> {
    let board =
        lookup::board_id(client, &args.board, args.workspace.as_deref()).await?;
    let (room, marks, user) = future::try_join3(
        open(client, &board),
        api::comment::thread_list(client, &board),
        account_id(client),
    )
    .await?;
    let threads = room
        .threads()
        .into_iter()
        .map(|thread| ListedThread {
            read_at: marks
                .iter()
                .find(|mark| mark.thread_id == thread.id && mark.user_id == user)
                .map(|mark| mark.read_at.clone()),
            thread,
        })
        .collect();
    Ok(ThreadList { board, threads })
}

/// Starts a thread at a point of the board, on an object, or in a free corner.
///
/// # Errors
/// A blank text, `at` and `object` given together, an `at` that is not two
/// numbers, a board or object that is not found, a board that is read-only for
/// the user, and errors of the requests.
pub async fn add(
    client: &Client,
    cache: &CacheSettings,
    args: &AddArgs,
) -> Result<CommentReport> {
    through_cache(client, cache, CacheUse::Use, async |loading| {
        started(client, loading, args).await
    })
    .await
}

async fn started(
    client: &Client,
    loading: Loading<'_>,
    args: &AddArgs,
) -> Result<CommentReport> {
    let text = visible("comment add", "text", &args.text)?;
    let placement =
        placement("comment add", args.at.as_deref(), args.object.as_deref())?;
    let open = opened(client, &args.board, args.workspace.as_deref()).await?;
    let anchor = match placement {
        Placement::Point(x, y) => ThreadAnchor::Point { x, y },
        Placement::Object(reference) => {
            object_anchor(client, loading, &open.board, reference).await?
        }
        Placement::Free => free_corner(&open.threads),
    };
    let message = new_message(client, &open, text);
    let (id, thread) = (message.id.clone(), message.id.clone());
    let change = CommentChange::Start { message, anchor };
    carry(
        client,
        open,
        change,
        CommentReport::of(CommentAction::Add, thread, Some(id)),
        (args.dry_run, None, Confirmation::Missing),
    )
    .await
}

/// Replies in a thread.
///
/// # Errors
/// A blank text, a board or thread that is not found, a board that is
/// read-only for the user, and errors of the requests.
pub async fn reply(client: &Client, args: &ReplyArgs) -> Result<CommentReport> {
    let text = visible("comment reply", "text", &args.text)?;
    let open = opened(client, &args.board, args.workspace.as_deref()).await?;
    let thread = found(&open.threads, &args.thread)?.id.clone();
    let message = new_message(client, &open, text);
    let id = message.id.clone();
    let change = CommentChange::Reply {
        thread: thread.clone(),
        message,
    };
    carry(
        client,
        open,
        change,
        CommentReport::of(CommentAction::Reply, thread, Some(id)),
        (args.dry_run, None, Confirmation::Missing),
    )
    .await
}

/// Resolves a thread, or with `reopen` opens it again.
///
/// # Errors
/// A board or thread that is not found, a board that is read-only for the
/// user, and errors of the requests.
pub async fn resolve(client: &Client, args: &ResolveArgs) -> Result<CommentReport> {
    let open = opened(client, &args.board, args.workspace.as_deref()).await?;
    let thread = found(&open.threads, &args.thread)?.id.clone();
    let (action, change) = if args.reopen {
        (
            CommentAction::Reopen,
            CommentChange::Reopen {
                thread: thread.clone(),
            },
        )
    } else {
        (
            CommentAction::Resolve,
            CommentChange::Resolve {
                thread: thread.clone(),
                by: open.me.clone(),
                at: millis(client.unix_millis()),
            },
        )
    };
    carry(
        client,
        open,
        change,
        CommentReport::of(action, thread, None),
        (args.dry_run, None, Confirmation::Missing),
    )
    .await
}

/// Deletes a thread with its replies, or with `reply` only one reply of the
/// user's own; it stays unsent without its confirmation.
///
/// # Errors
/// A board or thread that is not found, a `reply` that is not a reply of the
/// thread or was written by someone else, a board that is read-only for the
/// user, and errors of the requests.
pub async fn rm(client: &Client, args: &RemoveArgs) -> Result<CommentReport> {
    let open = opened(client, &args.board, args.workspace.as_deref()).await?;
    let found = found(&open.threads, &args.thread)?.clone();
    match args.reply.as_deref() {
        Some(reference) => removed_reply(client, open, args, &found, reference).await,
        None => removed_thread(client, open, args, &found).await,
    }
}

async fn removed_thread(
    client: &Client,
    open: Opened,
    args: &RemoveArgs,
    found: &Thread,
) -> Result<CommentReport> {
    let thread = found.id.clone();
    let first = found
        .messages
        .first()
        .and_then(|message| message.text.lines().next())
        .unwrap_or_default();
    let change_text: Cow<'static, str> = match found.messages.len() {
        0 | 1 => format!("deleting the thread {thread} \"{first}\"").into(),
        more => format!(
            "deleting the thread {thread} \"{first}\" with {} replies",
            more - 1
        )
        .into(),
    };
    let unconfirmed = Unconfirmed::new(change_text, ChangeRisk::Irreversible);
    let change = CommentChange::Delete {
        thread: thread.clone(),
    };
    carry(
        client,
        open,
        change,
        CommentReport::of(CommentAction::Delete, thread, None),
        (args.dry_run, Some(unconfirmed), args.confirm),
    )
    .await
}

async fn removed_reply(
    client: &Client,
    open: Opened,
    args: &RemoveArgs,
    found: &Thread,
    reference: &str,
) -> Result<CommentReport> {
    let reply = replied(found, reference)?.clone();
    (open.room.author_of(&open.me) == Some(reply.author))
        .then_some(())
        .ok_or_else(|| {
            not_value(
                "comment rm",
                "reply",
                "a reply of your own; another author's reply stays",
            )
        })?;
    let first = reply.text.lines().next().unwrap_or_default();
    let change_text: Cow<'static, str> = format!(
        "deleting the reply {} \"{first}\" of thread {}",
        reply.id, found.id
    )
    .into();
    let unconfirmed = Unconfirmed::new(change_text, ChangeRisk::Irreversible);
    let change = CommentChange::DeleteReply {
        thread: found.id.clone(),
        reply: reply.id.clone(),
    };
    carry(
        client,
        open,
        change,
        CommentReport::of(CommentAction::DeleteReply, found.id.clone(), Some(reply.id)),
        (args.dry_run, Some(unconfirmed), args.confirm),
    )
    .await
}

/// The reply a reference names: its message id, or N for the Nth reply
/// counting from 1.
fn replied<'a>(thread: &'a Thread, reference: &str) -> Result<&'a ThreadMessage> {
    let replies = thread.messages.get(1..).unwrap_or_default();
    let reference = reference.trim();
    replies
        .iter()
        .find(|message| message.id == reference)
        .or_else(|| {
            let number = reference.parse::<usize>().ok()?;
            replies.get(number.checked_sub(1)?)
        })
        .ok_or_else(|| {
            Error::from(InputError::ReplyNotFound {
                reply: reference.to_owned(),
                thread: thread.id.clone(),
            })
        })
}

/// Colours a thread, moves it, or both in one write.
///
/// # Errors
/// Nothing to change, `at` and `object` given together, an `at` that is not
/// two numbers, a board, thread or object that is not found, a board that is
/// read-only for the user, and errors of the requests.
pub async fn edit(
    client: &Client,
    cache: &CacheSettings,
    args: &EditArgs,
) -> Result<CommentReport> {
    through_cache(client, cache, CacheUse::Use, async |loading| {
        edited(client, loading, args).await
    })
    .await
}

async fn edited(
    client: &Client,
    loading: Loading<'_>,
    args: &EditArgs,
) -> Result<CommentReport> {
    let placement =
        placement("comment edit", args.at.as_deref(), args.object.as_deref())?;
    (args.color.is_some() || placement != Placement::Free)
        .then_some(())
        .ok_or_else(|| not_value("comment edit", "color", "given, or at or object"))?;
    let open = opened(client, &args.board, args.workspace.as_deref()).await?;
    let thread = found(&open.threads, &args.thread)?.id.clone();
    let anchor = match placement {
        Placement::Point(x, y) => Some(ThreadAnchor::Point { x, y }),
        Placement::Object(reference) => {
            Some(object_anchor(client, loading, &open.board, reference).await?)
        }
        Placement::Free => None,
    };
    let edited = args
        .color
        .map(|_| "color")
        .into_iter()
        .chain(anchor.as_ref().map(|_| "place"))
        .collect();
    let color = args.color.map(|color| {
        (color != ThreadColor::Clear).then(|| <&str>::from(color).to_owned())
    });
    let change = CommentChange::Several(
        color
            .map(|color| CommentChange::Color {
                thread: thread.clone(),
                color,
            })
            .into_iter()
            .chain(anchor.map(|anchor| CommentChange::Move {
                thread: thread.clone(),
                anchor,
            }))
            .collect(),
    );
    carry(
        client,
        open,
        change,
        CommentReport {
            edited,
            ..CommentReport::of(CommentAction::Edit, thread, None)
        },
        (args.dry_run, None, Confirmation::Missing),
    )
    .await
}

/// Marks a thread as read, now or at `time`.
///
/// # Errors
/// A board or thread that is not found, and errors of the requests.
pub async fn read(client: &Client, args: &ReadArgs) -> Result<CommentReport> {
    let at = match args.time.as_deref() {
        None => client.now_iso8601(),
        Some(text) => Timestamp::parse_iso8601(text)
            .map(|_| text.to_owned())
            .ok_or_else(|| {
                not_value(
                    "comment read",
                    "time",
                    "ISO 8601, e.g. 2026-10-27T09:30:00Z",
                )
            })?,
    };
    let open = opened(client, &args.board, args.workspace.as_deref()).await?;
    let thread = found(&open.threads, &args.thread)?.id.clone();
    let plan = CommentPlan {
        action: CommentAction::Read,
        thread: thread.clone(),
    };
    let outcome = match args.dry_run {
        WriteMode::DryRun => ChangeReport::DryRun { request: plan },
        WriteMode::Send => {
            let read = SetReadAt {
                channel_id: open.board.clone(),
                thread_id: thread.clone(),
                read_at: at.clone(),
            };
            api::comment::set_read_at_time(client, &read).await?;
            ChangeReport::Sent
        }
    };
    Ok(CommentReport {
        action: CommentAction::Read,
        board: open.board,
        thread,
        message: None,
        read_at: Some(at),
        edited: Vec::new(),
        outcome,
    })
}

/// Sends the change, reports it for a dry run, or holds it for want of its
/// confirmation (a dry run needs none).
async fn carry(
    client: &Client,
    open: Opened,
    change: CommentChange,
    changed: CommentReport,
    gate: (WriteMode, Option<Unconfirmed>, Confirmation),
) -> Result<CommentReport> {
    match open.room.right() {
        EditRight::ReadOnly => Err(Error::BoardChange(BoardChangeError::ReadOnly)),
        EditRight::Editable => released(client, open, change, changed, gate).await,
    }
}

async fn released(
    client: &Client,
    open: Opened,
    change: CommentChange,
    changed: CommentReport,
    (dry_run, unconfirmed, confirm): (WriteMode, Option<Unconfirmed>, Confirmation),
) -> Result<CommentReport> {
    let plan = || CommentPlan {
        action: changed.action,
        thread: changed.thread.clone(),
    };
    let outcome = match gate(unconfirmed, (dry_run, confirm), plan) {
        Err(stays) => Ok(stays),
        Ok(()) => open
            .room
            .send(client, &change)
            .await
            .map(|()| ChangeReport::Sent),
    }?;
    Ok(CommentReport {
        board: open.board,
        outcome,
        ..changed
    })
}

/// Where `comment add` puts a thread.
#[derive(Debug, PartialEq)]
enum Placement<'a> {
    Free,
    Point(f64, f64),
    Object(&'a str),
}

fn placement<'a>(
    command: &str,
    at: Option<&str>,
    object: Option<&'a str>,
) -> Result<Placement<'a>> {
    match (at, object) {
        (Some(_), Some(_)) => {
            Err(not_value(command, "object", "left out when --at is given"))
        }
        (Some(at), None) => at
            .split_once(',')
            .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
            .map(|(x, y)| Placement::Point(x, y))
            .ok_or_else(|| not_value(command, "at", "two numbers like 120,80")),
        (None, Some(object)) => Ok(Placement::Object(object)),
        (None, None) => Ok(Placement::Free),
    }
}

/// The object the reference names on the board: its whole id, a link to it or
/// the first 8 characters of its id, with the `@` that `o ls` prints; the
/// thread sits at the middle of it.
async fn object_anchor(
    client: &Client,
    loading: Loading<'_>,
    board: &str,
    reference: &str,
) -> Result<ThreadAnchor> {
    let (loaded, _) = board_load::load(
        RemoteBoard {
            client,
            connector: loading.connector,
            board,
        },
        loading.cache,
        loading.cache_use,
    )
    .await?;
    let ids: Vec<String> = objects(&loaded.document)
        .into_iter()
        .map(|object| object.id)
        .collect();
    find_id(ids.iter().map(String::as_str), reference).map(|id| ThreadAnchor::Object {
        id,
        x: 0.5,
        y: 0.5,
    })
}

/// The first point on the diagonal of the board that no thread holds.
fn free_corner(threads: &[Thread]) -> ThreadAnchor {
    let held = |step: u32| {
        threads.iter().any(|thread| {
            matches!(
                thread.anchor,
                ThreadAnchor::Point { x, y }
                    if (x - f64::from(step) * 40.0).abs() < 1.0
                        && (y - f64::from(step) * 40.0).abs() < 1.0
            )
        })
    };
    let step = (0..u32::MAX).find(|step| !held(*step)).unwrap_or_default();
    ThreadAnchor::Point {
        x: f64::from(step) * 40.0,
        y: f64::from(step) * 40.0,
    }
}

fn new_message(client: &Client, open: &Opened, text: String) -> NewThreadMessage {
    let taken: HashSet<&str> = open
        .threads
        .iter()
        .flat_map(|thread| thread.messages.iter().map(|message| message.id.as_str()))
        .collect();
    NewThreadMessage {
        id: short_id(&client.new_id(), |id| taken.contains(id)),
        author: open.me.clone(),
        created_at: millis(client.unix_millis()),
        text,
    }
}

/// Unix milliseconds as the web client keeps them: a float.
fn millis(unix_millis: u64) -> f64 {
    let whole = Duration::from_millis(unix_millis);
    f64::from(u32::try_from(whole.as_secs()).unwrap_or(u32::MAX)) * 1000.0
        + f64::from(whole.subsec_millis())
}

/// The thread a reference names: its id, or `@N` for the Nth of the listing.
fn found<'a>(threads: &'a [Thread], reference: &str) -> Result<&'a Thread> {
    let reference = reference.trim();
    let by_number = reference
        .strip_prefix('@')
        .and_then(|number| number.parse::<usize>().ok())
        .and_then(|number| threads.get(number.checked_sub(1)?));
    threads
        .iter()
        .find(|thread| thread.id == reference)
        .or(by_number)
        .ok_or_else(|| {
            Error::Lookup(LookupError {
                kind: LookupKind::Thread,
                reference: reference.to_owned(),
                miss: LookupMiss::NotFound {
                    unread: SkippedWorkspaces::default(),
                    available: Candidates::capped(
                        threads.iter().zip(1_u32..).map(listed),
                        5,
                    ),
                },
            })
        })
}

/// A thread as a lookup lists it: its `@N`, first line and id.
fn listed((thread, number): (&Thread, u32)) -> Candidate {
    Candidate {
        alias: Some(format!("@{number}")),
        name: thread
            .messages
            .first()
            .and_then(|message| message.text.lines().next())
            .unwrap_or_default()
            .to_owned(),
        id: Some(thread.id.clone()),
        ..Candidate::default()
    }
}

#[cfg(test)] mod tests;
