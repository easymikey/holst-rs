//! Counts the signals that arrive and marks the time a question is answered.

use std::{
    convert::Infallible,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

pub use imp::Listeners;

use crate::{draw, tty};

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
    let mut listeners = Listeners::interrupt_only();
    async move {
        listeners.next().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub enum Signal {
    #[display("interrupted")]
    Interrupt,
    #[display("terminated")]
    Terminate,
    #[display("the terminal hung up")]
    HangUp,
}

impl Signal {
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Signal::Interrupt => 130,
            Signal::Terminate => 143,
            Signal::HangUp => 129,
        }
    }
}

const SEIZE_PATIENCE: Duration = Duration::from_millis(500);

/// Restores the terminal, closes the form with `text`, ends the process.
#[must_use]
#[expect(
    clippy::exit,
    clippy::disallowed_methods,
    reason = "a signal ends the process from outside the command: nothing else can stop a blocking read"
)]
pub fn leave(text: &str, code: u8) -> Infallible {
    // Let a cut read put its settings back, then keep it from doing so.
    let _owned = tty::seize(SEIZE_PATIENCE);
    tty::warn(tty::restore().as_deref());
    tty::stderr_line(&farewell(text));
    std::process::exit(i32::from(code))
}

fn farewell(text: &str) -> String {
    format!("{}\n└  {text}", draw::SHOW_CURSOR)
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
    use std::{
        future::{pending, ready},
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use tokio::signal::unix::{Signal, SignalKind, signal};

    /// Listens for Ctrl-C, SIGTERM and SIGHUP.
    #[derive(Debug)]
    pub struct Listeners {
        interrupt: Option<Signal>,
        terminate: Option<Signal>,
        hangup: Option<Signal>,
    }

    impl Default for Listeners {
        fn default() -> Self {
            Self {
                terminate: signal(SignalKind::terminate()).ok(),
                hangup: signal(SignalKind::hangup()).ok(),
                ..Self::interrupt_only()
            }
        }
    }

    impl Listeners {
        #[must_use]
        pub fn interrupt_only() -> Self {
            Self {
                interrupt: signal(SignalKind::interrupt()).ok(),
                terminate: None,
                hangup: None,
            }
        }

        pub async fn next(&mut self) -> crate::signals::Signal {
            tokio::select! {
                () = heard(self.interrupt.as_mut()) => crate::signals::Signal::Interrupt,
                () = heard(self.terminate.as_mut()) => crate::signals::Signal::Terminate,
                () = heard(self.hangup.as_mut()) => crate::signals::Signal::HangUp,
            }
        }
    }

    async fn heard(listener: Option<&mut Signal>) {
        let got = match listener {
            Some(listener) => listener.recv().await,
            None => None,
        };
        match got {
            Some(()) => ready(()).await,
            None => pending().await,
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
    use std::{
        future::pending,
        sync::atomic::{AtomicBool, AtomicU64},
    };

    use tokio::signal::windows::{CtrlC, ctrl_c};

    /// Listens for Ctrl-C, the one signal Windows has.
    #[derive(Debug)]
    pub struct Listeners {
        interrupt: Option<CtrlC>,
    }

    impl Default for Listeners {
        fn default() -> Self {
            Self::interrupt_only()
        }
    }

    impl Listeners {
        #[must_use]
        pub fn interrupt_only() -> Self {
            Self {
                interrupt: ctrl_c().ok(),
            }
        }

        pub async fn next(&mut self) -> crate::signals::Signal {
            let got = match self.interrupt.as_mut() {
                Some(listener) => listener.recv().await,
                None => None,
            };
            match got {
                Some(()) => crate::signals::Signal::Interrupt,
                None => pending().await,
            }
        }
    }

    pub(super) fn note_arrivals(_: &'static AtomicBool, _: &'static AtomicU64) {}
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};

    use crate::signals::{answering, farewell, is_answering, or_interrupted};

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

    #[test]
    fn farewell_closes_the_form_with_the_cursor_back() {
        assert_eq!(farewell("login cancelled"), "\x1b[?25h\n└  login cancelled");
    }
}
