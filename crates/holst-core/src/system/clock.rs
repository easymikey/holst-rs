#![expect(
    clippy::disallowed_methods,
    reason = "the system clock is the one place that reads wall time and sleeps"
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::effect::{clock::Clock, http::BoxFuture};

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            })
    }

    fn sleep(&self, wait: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(wait))
    }
}
