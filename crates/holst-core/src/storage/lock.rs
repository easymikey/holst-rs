//! The `LOCK` file the cache and the session store take across processes.

use std::{
    fs::{File, TryLockError},
    io,
    path::Path,
    thread,
    time::Duration,
};

use crate::storage::file::private_file;

/// How long a lock is awaited before the holder is deemed busy.
pub(crate) const LOCK_WAIT_MILLIS: u64 = 2_000;

const LOCK_RETRY_MILLIS: u64 = 25;

/// The name of the lock file beside what it guards.
pub(crate) const LOCK_FILE: &str = "LOCK";

#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous retry loop with no runtime or clock: the pause blocks the thread"
)]
/// Creates or opens `path` and locks it, retrying for `wait_millis`; `None`
/// when it stayed busy. The lock is released when the file is dropped.
///
/// # Errors
/// The I/O error from opening the file or taking the lock.
pub(crate) fn lock_waiting(path: &Path, wait_millis: u64) -> io::Result<Option<File>> {
    let file = private_file(path)?;
    let pause = Duration::from_millis(LOCK_RETRY_MILLIS);
    (0..=wait_millis / LOCK_RETRY_MILLIS)
        .find_map(|attempt| {
            if attempt > 0 {
                thread::sleep(pause);
            }
            match file.try_lock() {
                Ok(()) => Some(Ok(())),
                Err(TryLockError::WouldBlock) => None,
                Err(TryLockError::Error(err)) => Some(Err(err)),
            }
        })
        .transpose()
        .map(|taken| taken.map(|()| file))
}
