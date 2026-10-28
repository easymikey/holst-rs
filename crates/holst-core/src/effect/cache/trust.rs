//! A cached board's trust as types: readers take only a served copy.

use std::marker::PhantomData;

use yrs::{Doc, StateVector, updates::decoder::Decode as _};

use crate::{
    domain::{
        board::{
            EditRight,
            state_vector::{ahead, holds},
        },
        board_text::index::BoardIndex,
    },
    effect::cache::{
        CachePort,
        meta::{CacheMeta, ReadCheck},
    },
    error::Result,
};

/// A board's cached copy in the trust state `Trust`, holding `Body`.
#[derive(Debug)]
pub struct CachedCopy<Trust, Body = ()> {
    board: String,
    meta: CacheMeta,
    body: Body,
    trust: PhantomData<Trust>,
}

/// Read from the cache: maybe past the trust window, maybe corrupt.
#[derive(Debug)]
pub enum Unverified {}

/// Within the trust window, not yet checked with the server: a seed only.
#[derive(Debug)]
pub enum Trusted {}

/// Confirmed by the server's state vector.
#[derive(Debug)]
pub enum Verified {}

/// Read without the network, as `--offline` asks: served unchecked.
#[derive(Debug)]
pub enum Offline {}

/// Why a cached copy cannot serve: the board must load again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleCopy {
    Expired,
    Corrupt,
    Ahead,
    Behind,
}

mod sealed {
    /// A trust state of this module.
    pub trait Sealed {}
}

/// The trust states whose body a reader may take.
pub trait ServableTrust: sealed::Sealed {
    /// How the read is noted in the entry.
    const READ_CHECK: ReadCheck;
}

impl sealed::Sealed for Verified {}
impl sealed::Sealed for Offline {}

impl ServableTrust for Verified {
    const READ_CHECK: ReadCheck = ReadCheck::Verified;
}

impl ServableTrust for Offline {
    const READ_CHECK: ReadCheck = ReadCheck::Read;
}

impl CachedCopy<Unverified> {
    /// Returns the cached entry of `board`; `None` when it is not cached.
    #[must_use]
    pub fn read(cache: &dyn CachePort, board: &str) -> Option<Self> {
        cache.meta(board).map(|meta| Self {
            board: board.to_owned(),
            meta,
            body: (),
            trust: PhantomData,
        })
    }
}

impl<Body> CachedCopy<Unverified, Body> {
    /// Returns the copy as a seed while it is in the trust window.
    ///
    /// # Errors
    /// [`StaleCopy::Expired`] past the window.
    pub fn trusted(
        self,
        cache: &dyn CachePort,
    ) -> std::result::Result<CachedCopy<Trusted, Body>, StaleCopy> {
        cache
            .trusts(&self.meta)
            .then(|| self.into_state())
            .ok_or(StaleCopy::Expired)
    }

    /// Returns the copy as read offline: served without a check, as asked.
    #[must_use]
    pub fn offline(self) -> CachedCopy<Offline, Body> {
        self.into_state()
    }
}

impl<Trust, Body> CachedCopy<Trust, Body> {
    /// Returns how long ago the board was last downloaded whole, at `at`.
    #[must_use]
    pub const fn age_millis(&self, at: u64) -> u64 {
        self.meta.age_millis(at)
    }

    fn into_state<Next>(self) -> CachedCopy<Next, Body> {
        CachedCopy {
            board: self.board,
            meta: self.meta,
            body: self.body,
            trust: PhantomData,
        }
    }

    fn holding<Next>(self, body: Next) -> CachedCopy<Trust, Next> {
        CachedCopy {
            board: self.board,
            meta: self.meta,
            body,
            trust: PhantomData,
        }
    }
}

impl<Trust> CachedCopy<Trust> {
    /// Returns the copy with its document built by `restore`.
    ///
    /// # Errors
    /// [`StaleCopy::Corrupt`] for a missing or broken state; broken is dropped.
    pub fn with_document(
        self,
        cache: &dyn CachePort,
        restore: impl FnOnce(&[u8]) -> Result<Doc>,
    ) -> std::result::Result<CachedCopy<Trust, Doc>, StaleCopy> {
        let state = cache.load_document(&self.board).ok_or(StaleCopy::Corrupt)?;
        let document = restore(&state)
            .inspect_err(|err| {
                tracing::debug!(
                    board = self.board,
                    %err,
                    "dropped a cached board that does not decode"
                );
                cache
                    .discard(&self.board)
                    .inspect_err(
                        |discard_error| tracing::debug!(%discard_error, "board cache: not updated"),
                    )
                    .unwrap_or_default();
            })
            .map_err(|_| StaleCopy::Corrupt)?;
        Ok(self.holding(document))
    }

    /// Returns the copy with its index if built from the same document.
    #[must_use]
    pub fn with_index(
        self,
        cache: &dyn CachePort,
    ) -> Option<CachedCopy<Trust, BoardIndex>> {
        let committed = committed(&self.meta)?;
        let index = cache
            .load_index(&self.board)
            .and_then(|bytes| BoardIndex::decode(&bytes).ok())
            .filter(|index| index.board().eq_ignore_ascii_case(&self.board))
            .filter(|index| {
                StateVector::decode_v1(&index.stamp().state_vector)
                    .ok()
                    .as_ref()
                    == Some(&committed)
            })?;
        Some(self.holding(index))
    }
}

impl CachedCopy<Trusted, Doc> {
    /// Returns the document as a seed for a load the server checks.
    #[must_use]
    pub fn into_seed(self) -> Doc {
        self.body
    }
}

impl CachedCopy<Trusted, BoardIndex> {
    /// Returns the copy confirmed by the server's state vector `announced`.
    ///
    /// # Errors
    /// [`StaleCopy::Ahead`], [`StaleCopy::Behind`] or [`StaleCopy::Corrupt`].
    pub fn verify(
        self,
        announced: Option<&StateVector>,
    ) -> std::result::Result<CachedCopy<Verified, BoardIndex>, StaleCopy> {
        let have = committed(&self.meta).ok_or(StaleCopy::Corrupt)?;
        match announced {
            Some(server) if ahead(&have, server) => Err(StaleCopy::Ahead),
            Some(server) if holds(&have, server) => Ok(self.into_state()),
            Some(_) | None => Err(StaleCopy::Behind),
        }
    }
}

impl<Trust: ServableTrust, Body> CachedCopy<Trust, Body> {
    /// Returns the user's right on the board when it was cached.
    #[must_use]
    pub fn right(&self) -> EditRight {
        self.meta.right.unwrap_or(EditRight::ReadOnly)
    }

    /// Returns how the read is noted in the entry.
    #[must_use]
    pub const fn read_check(&self) -> ReadCheck {
        Trust::READ_CHECK
    }

    /// Returns the document or index for the reader.
    #[must_use]
    pub fn into_body(self) -> Body {
        self.body
    }
}

/// Returns whether the cached index was built from the document at `after`.
#[must_use]
pub fn is_indexed(cache: &dyn CachePort, board: &str, after: &StateVector) -> bool {
    cache
        .load_index(board)
        .and_then(|bytes| BoardIndex::peek(&bytes).ok())
        .is_some_and(|stamp| {
            stamp.board.eq_ignore_ascii_case(board)
                && StateVector::decode_v1(&stamp.state_vector).ok().as_ref()
                    == Some(after)
        })
}

fn committed(meta: &CacheMeta) -> Option<StateVector> {
    meta.state_vector()
        .and_then(|bytes| StateVector::decode_v1(&bytes).ok())
}

#[cfg(test)] pub(crate) mod tests;
