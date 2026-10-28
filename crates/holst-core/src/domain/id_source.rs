//! The id port: new boards and copies get their ids on the client.

use std::sync::atomic::{AtomicU64, Ordering};

/// A source of fresh ids.
pub trait IdSource: Send + Sync {
    /// The next id.
    fn new_id(&self) -> String;
}

/// Predictable ids `<prefix>-1`, `<prefix>-2`, … for tests.
///
/// ```
/// use holst_core::domain::id_source::{IdSource, SequentialIds};
///
/// let ids = SequentialIds::new("b");
/// assert_eq!([ids.new_id(), ids.new_id()], ["b-1", "b-2"]);
/// ```
#[derive(Debug)]
pub struct SequentialIds {
    prefix: &'static str,
    issued: AtomicU64,
}

impl SequentialIds {
    #[must_use]
    pub const fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            issued: AtomicU64::new(0),
        }
    }
}

impl IdSource for SequentialIds {
    fn new_id(&self) -> String {
        let n = self
            .issued
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        format!("{}-{n}", self.prefix)
    }
}
