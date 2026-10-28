//! The clock port: the only place the core reads the time or waits.

use std::time::Duration;

use crate::{
    domain::{id_source::IdSource, node::CreationStamp, time::Timestamp},
    effect::http::BoxFuture,
};

/// A clock: the current time in Unix milliseconds and pauses.
pub trait Clock: Send + Sync {
    /// Milliseconds since 1970-01-01 UTC.
    fn unix_millis(&self) -> u64;

    /// A pause of `wait`, e.g. before a retry; by default none, a clock that
    /// really waits overrides it.
    fn sleep(&self, _wait: Duration) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

impl<C: Clock + ?Sized> Clock for std::sync::Arc<C> {
    fn unix_millis(&self) -> u64 {
        (**self).unix_millis()
    }

    fn sleep(&self, wait: Duration) -> BoxFuture<'static, ()> {
        (**self).sleep(wait)
    }
}

/// A stopped clock — for reproducible tests.
///
/// ```
/// use holst_core::effect::clock::{Clock, FixedClock};
///
/// assert_eq!(FixedClock { unix_millis: 42 }.unix_millis(), 42);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock {
    pub unix_millis: u64,
}

impl Clock for FixedClock {
    fn unix_millis(&self) -> u64 {
        self.unix_millis
    }
}

impl CreationStamp {
    #[must_use]
    pub fn new(clock: &(impl Clock + ?Sized), ids: &(impl IdSource + ?Sized)) -> Self {
        Self::at(ids.new_id(), Timestamp::from_millis(clock.unix_millis()))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{id_source::SequentialIds, node::CreationStamp},
        effect::clock::FixedClock,
    };

    #[test]
    fn fixed_clock_and_sequence_give_reproducible_stamps() {
        let clock = FixedClock {
            unix_millis: 1_792_577_145_404,
        };
        let stamps = [
            CreationStamp::new(&clock, &SequentialIds::new("id")),
            CreationStamp::new(&clock, &SequentialIds::new("id")),
        ];
        assert_eq!(stamps[0], stamps[1]);
        assert_eq!(
            (stamps[0].id.as_str(), stamps[0].at.iso8601().as_str()),
            ("id-1", "2026-10-21T10:05:45.404Z")
        );
    }
}
