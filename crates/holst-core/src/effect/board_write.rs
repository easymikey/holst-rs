use serde::Deserialize;
use yrs::Doc;

use crate::{
    domain::{board::EditRight, change::receipt::Delivery},
    effect::{board_write_plan::WritePlan, http::BoxFuture},
    error::Result,
    remote::client::Client,
};

/// Whether the load downloaded a board link or the seed already covered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSource {
    Skipped,
    Downloaded { bytes: u64 },
}

impl LoadSource {
    pub(crate) const fn plus(self, bytes: u64) -> Self {
        match (self, bytes) {
            (LoadSource::Skipped, 0) => LoadSource::Skipped,
            (LoadSource::Skipped, bytes) => LoadSource::Downloaded { bytes },
            (LoadSource::Downloaded { bytes: before }, more) => {
                LoadSource::Downloaded {
                    bytes: before.saturating_add(more),
                }
            }
        }
    }
}

/// Whether a change is only shown or also sent: it picks the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "bool")]
pub enum WriteMode {
    DryRun,
    Send,
}

impl From<bool> for WriteMode {
    fn from(dry_run: bool) -> Self {
        if dry_run { Self::DryRun } else { Self::Send }
    }
}

/// A board loaded for a write and what its interpreter keeps open with it.
#[derive(Debug)]
pub struct OpenedBoard<Session> {
    pub connection: Session,
    pub document: Doc,
    pub right: EditRight,
    pub source: LoadSource,
}

/// Loads a board for a write, then sends, reports or records its plan.
pub trait WriteInterpreter: Sync {
    /// What stays open between the load and the plan.
    type Connection: Send;

    /// Loads `board` into `seed` (see
    /// [`writer_document`](fn@crate::remote::socket::write::writer_document)).
    ///
    /// # Errors
    /// As [`load_with`](fn@crate::remote::socket::load::load_with).
    fn open<'a>(
        &'a self,
        client: &'a Client,
        board: &'a str,
        seed: Doc,
    ) -> BoxFuture<'a, Result<OpenedBoard<Self::Connection>>>;

    /// Carries out `plan`: accepted once it went out, built while it stays.
    ///
    /// # Errors
    /// As [`send`](fn@crate::remote::socket::write::ConnectedBoard::send) and
    /// [`accepted`](fn@crate::remote::socket::write::SentUpdate::accepted).
    fn carry<'a>(
        &'a self,
        client: &'a Client,
        connection: Self::Connection,
        plan: &'a WritePlan,
    ) -> BoxFuture<'a, Result<Delivery>>;
}
