//! Counts the signals that arrive and marks the time a question is answered.

use std::{
    future::{pending, ready},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

static ANSWERING: AtomicBool = AtomicBool::new(false);

struct AnswerMark;

impl AnswerMark {
    fn start() -> Self {
        ANSWERING.store(true, Ordering::SeqCst);
        Self
    }
}

// GUARD: clears the answering mark that `start` set.
impl Drop for AnswerMark {
    fn drop(&mut self) {
        ANSWERING.store(false, Ordering::SeqCst);
    }
}

pub fn answering<R>(work: impl FnOnce() -> R) -> R {
    let _mark = AnswerMark::start();
    work()
}

/// Whether `answering` work is running now.
#[must_use]
pub fn is_answering() -> bool {
    ANSWERING.load(Ordering::SeqCst)
}

pub fn ctrl_c() -> impl Future<Output = ()> {
    let listener = imp::ctrl_c_listener();
    async move {
        match imp::next(listener).await {
            Some(()) => ready(()).await,
            None => pending().await,
        }
    }
}

static ARRIVED: AtomicBool = AtomicBool::new(false);

static INTERRUPTS: AtomicU64 = AtomicU64::new(0);

pub fn note_arrivals() {
    imp::note_arrivals(&ARRIVED, &INTERRUPTS);
}

#[must_use]
pub fn interrupts() -> u64 {
    INTERRUPTS.load(Ordering::SeqCst)
}

pub fn has_signal_arrived() -> bool {
    ARRIVED.swap(false, Ordering::SeqCst)
}

pub async fn or_interrupted<T>(
    work: impl Future<Output = T>,
    interrupt: impl Future<Output = ()>,
    interrupted: impl FnOnce() -> T,
) -> T {
    tokio::select! {
        biased;
        out = work => out,
        () = interrupt => interrupted(),
    }
}

#[cfg(unix)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use tokio::signal::unix::{Signal, SignalKind, signal};

    pub(super) fn ctrl_c_listener() -> Option<Signal> {
        signal(SignalKind::interrupt()).ok()
    }

    pub(super) async fn next(listener: Option<Signal>) -> Option<()> {
        match listener {
            Some(mut listener) => listener.recv().await,
            None => None,
        }
    }

    pub(super) fn note_arrivals(
        arrived: &'static AtomicBool,
        interrupts: &'static AtomicU64,
    ) {
        [libc::SIGINT, libc::SIGTERM, libc::SIGHUP]
            .into_iter()
            .map(|number| {
                let step = u64::from(number == libc::SIGINT);
                // SAFETY: the action only touches atomics, which is async-signal-safe.
                unsafe {
                    signal_hook_registry::register(number, move || {
                        arrived.store(true, Ordering::SeqCst);
                        interrupts.fetch_add(step, Ordering::SeqCst);
                    })
                }
            })
            // Without the flag a signal during a key read only reads as Ctrl-C.
            .for_each(drop);
    }
}

#[cfg(not(unix))]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU64};

    use tokio::signal::windows::{CtrlC, ctrl_c};

    pub(super) fn ctrl_c_listener() -> Option<CtrlC> {
        ctrl_c().ok()
    }

    pub(super) async fn next(listener: Option<CtrlC>) -> Option<()> {
        match listener {
            Some(mut listener) => listener.recv().await,
            None => None,
        }
    }

    pub(super) fn note_arrivals(_: &'static AtomicBool, _: &'static AtomicU64) {}
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};

    use crate::signals::{answering, is_answering, or_interrupted};

    #[test]
    fn answering_is_marked_only_for_the_time_of_the_work() {
        assert!(answering(is_answering));
        assert!(!is_answering());
    }

    #[tokio::test]
    async fn interruption_wins_only_over_unfinished_work() {
        let done = or_interrupted(ready(1), ready(()), || 0).await;
        let stopped = or_interrupted(pending::<i32>(), ready(()), || 0).await;
        assert_eq!((done, stopped), (1, 0));
    }
}
