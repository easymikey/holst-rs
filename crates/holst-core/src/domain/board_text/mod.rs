//! Board content: compact indexes, search across them, live loading.

pub mod index;
pub mod search;

use std::{collections::HashMap, sync::Arc};

use crate::{
    domain::{
        board_text::{
            index::BoardIndex,
            search::{ObjectFilter, TextCorpus, TextHit},
        },
        limit::Limit,
    },
    error::Result,
};

/// Boards that loaded, and the rest with why; fatal stops all.
///
/// # Errors
/// The first fatal error ([`crate::Error::is_fatal`]).
pub fn split_loaded<T>(loaded: Vec<(String, Result<T>)>) -> Result<BoardLoads<T>> {
    loaded.into_iter().try_fold(
        BoardLoads {
            read: Vec::new(),
            unread: Vec::new(),
        },
        |mut loads, (board, outcome)| match outcome {
            Ok(index) => {
                loads.read.push(index);
                Ok(loads)
            }
            Err(err) if err.is_fatal() => Err(err),
            Err(err) => {
                loads.unread.push(SkippedBoard {
                    board,
                    reason: err.to_string(),
                });
                Ok(loads)
            }
        },
    )
}

/// Boards that loaded, and the rest with why.
#[derive(Debug)]
pub struct BoardLoads<T> {
    pub read: Vec<T>,
    pub unread: Vec<SkippedBoard>,
}

/// A board the text search could not read, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedBoard {
    pub board: String,
    pub reason: String,
}

/// What to find in the text of boards.
#[derive(Debug, Clone)]
pub struct BoardTextQuery<'a> {
    pub query: &'a str,
    pub filter: ObjectFilter,
    pub limit: Limit,
}

/// The best text hits across boards and how many matched before the limit.
#[derive(Debug, Clone)]
pub struct BoardTextHits {
    pub hits: Vec<TextHit>,
    pub total: usize,
    pub boards_read: usize,
    pub names: HashMap<String, String>,
    pub unread: Vec<SkippedBoard>,
}

impl BoardTextHits {
    /// Returns the name of `board`, its id when no name is known.
    #[must_use]
    pub fn board_name(&self, board: &str) -> String {
        self.names
            .get(board)
            .cloned()
            .unwrap_or_else(|| board.to_owned())
    }
}

/// Ranks and limits the text hits of the boards that were read.
#[must_use]
pub fn found_text(
    read: &[Arc<BoardIndex>],
    names: impl IntoIterator<Item = (String, String)>,
    unread: Vec<SkippedBoard>,
    search: &BoardTextQuery<'_>,
) -> BoardTextHits {
    let (hits, total) =
        TextCorpus::from(read).search(search.query, &search.filter, search.limit);
    BoardTextHits {
        hits,
        total,
        boards_read: read.len(),
        names: names.into_iter().collect(),
        unread,
    }
}
