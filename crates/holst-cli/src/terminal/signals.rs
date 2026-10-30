//! Ctrl-C, SIGTERM and SIGHUP: for awaited work and blocked questions.

use std::sync::atomic::{AtomicBool, Ordering};

use holst_form::signals::{Listeners, leave};
pub(crate) use holst_form::signals::{Signal, note_arrivals};

use crate::{
    error::{Error, Result},
    terminal::awaiting::optional,
};

impl From<Signal> for Error {
    fn from(signal: Signal) -> Self {
        match signal {
            Signal::Interrupt => Error::Interrupted,
            Signal::Terminate => Error::Terminated,
            Signal::HangUp => Error::HungUp,
        }
    }
}

fn farewell(signal: Signal) -> String {
    match signal {
        Signal::Interrupt => Error::Cancelled.to_string(),
        signal @ (Signal::Terminate | Signal::HangUp) => signal.to_string(),
    }
}

/// Resolves with the first of Ctrl-C, SIGTERM and SIGHUP heard after the call.
pub(crate) fn stopped() -> impl Future<Output = Signal> {
    let mut listeners = Listeners::default();
    async move { listeners.next().await }
}

const INNER_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

pub(crate) fn stopped_fallback() -> impl Future<Output = Signal> {
    deferred(stopped(), stopped, INNER_GRACE, &SIGNAL_TAKEN)
}

/// Whether a command's own stop has taken a signal, so it is saving its login.
static SIGNAL_TAKEN: AtomicBool = AtomicBool::new(false);

/// Resolves after the first signal and grace; a taken one waits for a second.
#[expect(
    clippy::disallowed_methods,
    reason = "the grace period is a real wait on the tokio timer, not a time reading"
)]
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
    let signal = first_ending(&mut Listeners::default()).await;
    match leave(&farewell(signal), signal.exit_code()) {}
}

async fn first_ending(listeners: &mut Listeners) -> Signal {
    let signal = listeners.next().await;
    // A working command stops itself, so it can save a refreshed login.
    if holst_form::signals::is_answering() {
        signal
    } else {
        Box::pin(first_ending(listeners)).await
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
    #![expect(
        clippy::disallowed_methods,
        reason = "tests measure and wait on real time"
    )]

    use std::{
        future::{pending, ready},
        sync::atomic::AtomicBool,
    };

    use holst_form::signals::or_interrupted;
    use rstest::rstest;

    use crate::terminal::signals::{
        INNER_GRACE,
        Signal,
        deferred,
        farewell,
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
    fn farewell_names_the_signal() {
        assert_eq!(farewell(Signal::Interrupt), "login cancelled");
        assert_eq!(farewell(Signal::HangUp), "the terminal hung up");
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
}
