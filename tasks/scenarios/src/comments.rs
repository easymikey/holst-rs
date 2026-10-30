//! The `comment` group on either surface.

use holst_core::fake::{FixtureThread, ThreadAnchor};
use serde_json::{Map, Value};

use crate::{
    error::ScenarioError,
    holst::{Holst, On},
    observation::Observation,
    surface::{Words, arguments},
};

const NO_SUCH_OBJECT: &str = "00000000-0000-4000-8000-0000000000ee";

/// Where a new thread is pinned.
#[derive(Debug, Clone, Copy)]
pub enum ThreadPin<'a> {
    /// Nowhere in particular: the surface picks a free corner.
    Free,
    /// A point of the board, as the user typed it.
    At(&'a str),
    /// The object with this number on the board, counting from 1.
    Object(u32),
    /// The same object, named as `o ls` prints it.
    Listed(u32),
    /// A point and an object together, which the surfaces refuse.
    Both(&'a str, u32),
}

/// A comment command, for a scenario of all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentCommand {
    Ls,
    Add,
    Reply,
    Resolve,
    Remove,
    Read,
}

impl On<'_> {
    /// Lists the threads of the board `board`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_comments(
        &self,
        board: &str,
    ) -> Result<Observation, ScenarioError> {
        self.comment_step(board, "ls", Words::default()).await
    }

    /// Starts a thread on the board `board`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn add_comment(
        &self,
        board: &str,
        text: &str,
        pin: ThreadPin<'_>,
    ) -> Result<Observation, ScenarioError> {
        let (at, on) = self.pinned(board, pin);
        let words = Words {
            positional: vec![("text", text)],
            options: at
                .as_deref()
                .map(|at| ("at", at))
                .into_iter()
                .chain(on.as_deref().map(|on| ("object", on)))
                .collect(),
            ..Words::default()
        };
        self.comment_step(board, "add", words).await
    }

    /// Replies with `text` in the thread `thread` (its id or `@N`).
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn reply_in_comment(
        &self,
        board: &str,
        thread: &str,
        text: &str,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            positional: vec![("thread", thread), ("text", text)],
            ..Words::default()
        };
        self.comment_step(board, "reply", words).await
    }

    /// Resolves the thread `thread`, or with `reopen` opens it again.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn resolve_comment(
        &self,
        board: &str,
        thread: &str,
        reopen: bool,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            positional: vec![("thread", thread)],
            switches: reopen.then_some("reopen").into_iter().collect(),
            ..Words::default()
        };
        self.comment_step(board, "resolve", words).await
    }

    /// Colours the thread `thread` with `color` (`none` clears it) and pins it
    /// as `pin` says.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn edit_comment(
        &self,
        board: &str,
        thread: &str,
        color: Option<&str>,
        pin: ThreadPin<'_>,
    ) -> Result<Observation, ScenarioError> {
        let (at, on) = self.pinned(board, pin);
        let words = Words {
            positional: vec![("thread", thread)],
            options: color
                .map(|color| ("color", color))
                .into_iter()
                .chain(at.as_deref().map(|at| ("at", at)))
                .chain(on.as_deref().map(|on| ("object", on)))
                .collect(),
            ..Words::default()
        };
        self.comment_step(board, "edit", words).await
    }

    /// Deletes the thread `thread` with its replies.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn delete_comment(
        &self,
        board: &str,
        thread: &str,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            positional: vec![("thread", thread)],
            ..Words::default()
        };
        self.comment_step(board, "rm", words).await
    }

    /// Deletes the reply `reply` (a message id or its number) of the thread
    /// `thread`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn delete_reply(
        &self,
        board: &str,
        thread: &str,
        reply: &str,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            positional: vec![("thread", thread)],
            options: vec![("reply", reply)],
            ..Words::default()
        };
        self.comment_step(board, "rm", words).await
    }

    /// Marks the thread `thread` read, now or at `time` (ISO 8601).
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn mark_comment_read(
        &self,
        board: &str,
        thread: &str,
        time: Option<&str>,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            positional: vec![("thread", thread)],
            options: time.map(|time| ("time", time)).into_iter().collect(),
            ..Words::default()
        };
        self.comment_step(board, "read", words).await
    }

    /// Runs `command` on the board `board` with canned words, on the thread
    /// `thread` for those that take one.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn run_comment_command(
        &self,
        board: &str,
        command: CommentCommand,
        thread: &str,
    ) -> Result<Observation, ScenarioError> {
        match command {
            CommentCommand::Ls => self.list_comments(board).await,
            CommentCommand::Add => {
                self.add_comment(board, "Why?", ThreadPin::Free).await
            }
            CommentCommand::Reply => self.reply_in_comment(board, thread, "Yes").await,
            CommentCommand::Resolve => self.resolve_comment(board, thread, false).await,
            CommentCommand::Remove => self.delete_comment(board, thread).await,
            CommentCommand::Read => self.mark_comment_read(board, thread, None).await,
        }
    }

    fn pinned(
        &self,
        board: &str,
        pin: ThreadPin<'_>,
    ) -> (Option<String>, Option<String>) {
        let object = |number: u32| {
            self.holst
                .fake()
                .object_id(board, number)
                .unwrap_or_else(|| NO_SUCH_OBJECT.to_owned())
        };
        match pin {
            ThreadPin::Free => (None, None),
            ThreadPin::At(at) => (Some(at.to_owned()), None),
            ThreadPin::Object(number) => (None, Some(object(number))),
            ThreadPin::Listed(number) => {
                let id = object(number);
                (None, Some(format!("@{}", id.get(..8).unwrap_or(&id))))
            }
            ThreadPin::Both(at, number) => (Some(at.to_owned()), Some(object(number))),
        }
    }

    async fn comment_step(
        &self,
        board: &str,
        verb: &'static str,
        words: Words<'_>,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            confirms: verb == "rm",
            writes: verb != "ls",
            ..words
        };
        let (flags, fields) = arguments(&words, self.mode);
        // Piped, the CLI prints JSON; the scenarios read the text both share.
        let spoken: Vec<&str> = ["comment", verb, board, "-o", "text"]
            .into_iter()
            .chain(flags.iter().map(String::as_str))
            .collect();
        let tool: Map<String, Value> = [
            ("action".to_owned(), Value::from(verb)),
            ("board".to_owned(), Value::from(board)),
        ]
        .into_iter()
        .chain(fields)
        .collect();
        match verb {
            "ls" => self.reading(&spoken, "comment", Value::Object(tool)).await,
            _ => self.changing(&spoken, "comment", Value::Object(tool)).await,
        }
    }
}

#[must_use]
pub fn comments_of(holst: &Holst, board: &str) -> CommentsThen {
    CommentsThen {
        threads: holst.fake().threads_of(board),
    }
}

/// Returns when the signed-in user last read the thread `thread` of the board
/// called `board`, as the server holds the time.
#[must_use]
pub fn read_time_of(holst: &Holst, board: &str, thread: &str) -> Option<String> {
    holst.fake().thread_read_at(board, thread)
}

#[derive(Debug)]
pub struct CommentsThen {
    threads: Vec<FixtureThread>,
}

impl CommentsThen {
    /// Expects exactly `count` threads.
    ///
    /// # Panics
    /// When there are others.
    pub fn count(&self, count: usize) {
        assert_eq!(self.threads.len(), count, "the threads: {:?}", self.threads);
    }

    /// Expects the `number`th thread (counting from 1) to be pinned at
    /// `anchor`.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_pinned_at(&self, number: usize, anchor: &ThreadAnchor) {
        assert_eq!(
            self.nth(number).map(|thread| &thread.anchor),
            Some(anchor),
            "the anchor of thread {number}"
        );
    }

    /// Expects the messages of the `number`th thread, the first one and then
    /// the replies.
    ///
    /// # Panics
    /// When they differ.
    pub fn has_messages(&self, number: usize, texts: &[&str]) {
        let held: Option<Vec<&str>> = self.nth(number).map(|thread| {
            thread
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect()
        });
        assert_eq!(
            held.as_deref(),
            Some(texts),
            "the messages of thread {number}"
        );
    }

    /// Expects the `number`th thread to be resolved, or open.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_resolved(&self, number: usize, resolved: bool) {
        assert_eq!(
            self.nth(number).map(|thread| thread.resolved),
            Some(resolved),
            "the resolved flag of thread {number}"
        );
    }

    /// Expects the color of the `number`th thread to be `color`.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_colored(&self, number: usize, color: Option<&str>) {
        assert_eq!(
            self.nth(number).map(|thread| thread.color.as_deref()),
            Some(color),
            "the color of thread {number}"
        );
    }

    fn nth(&self, number: usize) -> Option<&FixtureThread> {
        self.threads.get(number.checked_sub(1)?)
    }
}
