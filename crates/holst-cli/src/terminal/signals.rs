//! Ctrl-C, SIGTERM and SIGHUP: for awaited work and blocked questions.

use std::{
    convert::Infallible,
    future::pending,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    error::{Error, Result},
    output,
    terminal::{awaiting::optional, tty},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub(crate) enum Signal {
    #[display("interrupted")]
    Interrupt,
    #[display("terminated")]
    Terminate,
    #[display("the terminal hung up")]
    HangUp,
}

impl Signal {
    #[must_use]
    pub(crate) const fn exit_code(self) -> u8 {
        match self {
            Signal::Interrupt => 130,
            Signal::Terminate => 143,
            Signal::HangUp => 129,
        }
    }
}

impl From<Signal> for Error {
    fn from(signal: Signal) -> Self {
        match signal {
            Signal::Interrupt => Error::Interrupted,
            Signal::Terminate => Error::Terminated,
            Signal::HangUp => Error::HungUp,
        }
    }
}

#[must_use]
fn farewell(signal: Signal) -> String {
    match signal {
        Signal::Interrupt => format!("\n└  {}", Error::Cancelled),
        signal @ (Signal::Terminate | Signal::HangUp) => format!("\n└  {signal}"),
    }
}

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

pub(crate) fn answering<R>(work: impl FnOnce() -> R) -> R {
    let _mark = AnswerMark::start();
    work()
}

pub(crate) fn ctrl_c() -> impl Future<Output = ()> {
    let listener = imp::ctrl_c_listener();
    async move {
        let deaf = imp::next(listener).await.err();
        optional(deaf.map(|()| pending::<()>())).await;
    }
}

/// Resolves with the first of Ctrl-C, SIGTERM and SIGHUP heard after the call.
pub(crate) fn stopped() -> impl Future<Output = Signal> {
    let mut listeners = imp::SignalListeners::new();
    async move { listeners.next().await }
}

const INNER_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

pub(crate) fn stopped_fallback() -> impl Future<Output = Signal> {
    deferred(stopped(), stopped, INNER_GRACE, &SIGNAL_TAKEN)
}

/// Whether a command's own stop has taken a signal, so it is saving its login.
static SIGNAL_TAKEN: AtomicBool = AtomicBool::new(false);

/// Resolves after the first signal and the grace. When the command took that
/// signal it is left to finish, and only a second signal resolves.
async fn deferred<S, Again: Future<Output = S>>(
    first: impl Future<Output = S>,
    again: impl FnOnce() -> Again,
    grace: std::time::Duration,
    taken: &AtomicBool,
) -> S {
    let heard = first.await;
    let second = again();
    tokio::time::sleep(grace).await;
    optional(taken.load(Ordering::SeqCst).then_some(second))
        .await
        .unwrap_or(heard)
}

static ARRIVED: AtomicBool = AtomicBool::new(false);

static INTERRUPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn note_arrivals() {
    imp::note_arrivals(&ARRIVED, &INTERRUPTS);
}

#[must_use]
pub(crate) fn interrupts() -> u64 {
    INTERRUPTS.load(Ordering::SeqCst)
}

pub(crate) fn has_signal_arrived() -> bool {
    ARRIVED.swap(false, Ordering::SeqCst)
}

/// Watch signals on their own thread: the runtime thread blocks on questions.
///
/// # Errors
///
/// The runtime for the watcher could not be built.
pub(crate) fn watch_apart() -> std::io::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| drop(std::thread::spawn(move || runtime.block_on(watch()))))
}

async fn watch() {
    match leave(first_ending(&mut imp::SignalListeners::new()).await) {}
}

async fn first_ending(listeners: &mut imp::SignalListeners) -> Signal {
    let signal = listeners.next().await;
    // A working command stops itself, so it can save a refreshed login.
    if ANSWERING.load(Ordering::SeqCst) {
        signal
    } else {
        Box::pin(first_ending(listeners)).await
    }
}

const SEIZE_PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

#[expect(
    clippy::exit,
    reason = "a signal ends the process from outside the command: nothing else can stop a blocking read"
)]
fn leave(signal: Signal) -> Infallible {
    // Let a cut read put its settings back, then keep it from doing so.
    let _owned = tty::seize(SEIZE_PATIENCE);
    tty::restore();
    output::stderr_line(farewell(signal));
    std::process::exit(i32::from(signal.exit_code()))
}

#[cfg(unix)]
mod imp {
    use std::{
        future::pending,
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use tokio::signal::unix::{Signal, SignalKind, signal};

    use crate::terminal::awaiting::optional;

    pub(super) struct SignalListeners {
        interrupt: Option<Signal>,
        terminate: Option<Signal>,
        hangup: Option<Signal>,
    }

    impl SignalListeners {
        pub(super) fn new() -> Self {
            Self {
                interrupt: signal(SignalKind::interrupt()).ok(),
                terminate: signal(SignalKind::terminate()).ok(),
                hangup: signal(SignalKind::hangup()).ok(),
            }
        }

        pub(super) async fn next(&mut self) -> crate::terminal::signals::Signal {
            tokio::select! {
                () = heard(self.interrupt.as_mut()) => crate::terminal::signals::Signal::Interrupt,
                () = heard(self.terminate.as_mut()) => crate::terminal::signals::Signal::Terminate,
                () = heard(self.hangup.as_mut()) => crate::terminal::signals::Signal::HangUp,
            }
        }
    }

    async fn heard(listener: Option<&mut Signal>) {
        let got = optional(listener.map(Signal::recv)).await.flatten();
        optional(got.is_none().then(pending::<()>)).await;
    }

    pub(super) fn ctrl_c_listener() -> Option<Signal> {
        signal(SignalKind::interrupt()).ok()
    }

    pub(super) async fn next(listener: Option<Signal>) -> Result<(), ()> {
        match listener {
            Some(mut listener) => listener.recv().await.ok_or(()),
            None => Err(()),
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
    use std::{future::pending, sync::atomic::AtomicBool};

    use crate::terminal::awaiting::optional;

    pub(super) struct SignalListeners;

    impl SignalListeners {
        pub(super) const fn new() -> Self {
            Self
        }

        pub(super) async fn next(&mut self) -> crate::terminal::signals::Signal {
            let deaf = tokio::signal::ctrl_c().await.is_err();
            optional(deaf.then(pending::<()>)).await;
            crate::terminal::signals::Signal::Interrupt
        }
    }

    pub(super) const fn ctrl_c_listener() -> Option<()> {
        Some(())
    }

    pub(super) async fn next(listener: Option<()>) -> Result<(), ()> {
        match listener {
            Some(()) => tokio::signal::ctrl_c().await.map_err(drop),
            None => Err(()),
        }
    }

    pub(super) fn note_arrivals(
        _: &'static AtomicBool,
        _: &'static std::sync::atomic::AtomicU64,
    ) {
    }
}

pub(crate) async fn or_interrupted<T>(
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

/// Ends the work with the signal as an error, so the normal exit path still
/// runs.
pub(crate) async fn or_stopped<T>(
    work: impl Future<Output = Result<T>>,
    stop: impl Future<Output = Signal>,
) -> Result<T> {
    tokio::select! {
        biased;
        out = work => out,
        signal = stop => {
            SIGNAL_TAKEN.store(true, Ordering::SeqCst);
            Err(Error::from(signal))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::{pending, ready},
        sync::atomic::{AtomicBool, Ordering},
    };

    use rstest::rstest;

    use crate::terminal::signals::{
        ANSWERING,
        INNER_GRACE,
        Signal,
        answering,
        deferred,
        farewell,
        or_interrupted,
        or_stopped,
    };

    #[rstest]
    #[case::interrupt(Signal::Interrupt, 130)]
    #[case::terminate(Signal::Terminate, 143)]
    #[case::hangup(Signal::HangUp, 129)]
    #[tokio::test]
    async fn a_signal_while_working_becomes_an_error_with_its_exit_code(
        #[case] signal: Signal,
        #[case] code: u8,
    ) {
        let outcome =
            or_stopped(pending::<crate::error::Result<()>>(), ready(signal)).await;
        assert_eq!(outcome.map_err(|err| crate::exit::code(&err)), Err(code));
    }

    #[test]
    fn farewell_closes_the_form() {
        assert_eq!(farewell(Signal::Interrupt), "\n└  login cancelled");
        assert_eq!(farewell(Signal::HangUp), "\n└  the terminal hung up");
    }

    #[test]
    fn answering_is_marked_only_for_the_time_of_the_work() {
        assert!(answering(|| ANSWERING.load(Ordering::SeqCst)));
        assert!(!ANSWERING.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn fallback_lets_the_inner_listener_finish_first() {
        let inner = async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            "closed by the form"
        };
        let idle = AtomicBool::new(false);
        let fallback = deferred(ready(()), pending::<()>, INNER_GRACE, &idle);
        let out = or_interrupted(inner, fallback, || "cut short").await;
        assert_eq!(out, "closed by the form");
    }

    #[tokio::test(start_paused = true)]
    async fn fallback_answers_a_second_signal_while_the_command_saves() {
        let saving = AtomicBool::new(true);
        let idle = AtomicBool::new(false);
        let second =
            deferred(ready("first"), || ready("second"), INNER_GRACE, &saving).await;
        let alone = deferred(ready("first"), pending::<&str>, INNER_GRACE, &idle).await;
        let silent = deferred(ready("first"), pending::<&str>, INNER_GRACE, &saving);
        let waited =
            tokio::time::timeout(std::time::Duration::from_hours(1), silent).await;
        assert_eq!((second, alone, waited.is_err()), ("second", "first", true));
    }

    #[tokio::test]
    async fn interruption_wins_only_over_unfinished_work() {
        let done = or_interrupted(ready(1), ready(()), || 0).await;
        let stopped = or_interrupted(pending::<i32>(), ready(()), || 0).await;
        assert_eq!((done, stopped), (1, 0));
    }
}
