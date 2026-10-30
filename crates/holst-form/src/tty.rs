//! The settings of stdin's terminal, restored however the program ends.

use std::{
    io::{self, Write as _},
    sync::{Mutex, MutexGuard, PoisonError, TryLockError},
    time::Duration,
};

static TERMINAL: Mutex<()> = Mutex::new(());

const SEIZE_STEP: Duration = Duration::from_millis(5);

pub fn exclusive<R>(work: impl FnOnce() -> R) -> R {
    let _held = TERMINAL.lock().unwrap_or_else(PoisonError::into_inner);
    work()
}

#[must_use = "the terminal is owned only while the guard lives"]
#[expect(
    clippy::disallowed_methods,
    reason = "called from the signal thread, outside the async runtime: the wait blocks the thread"
)]
pub fn seize(patience: Duration) -> Option<MutexGuard<'static, ()>> {
    let tries = patience.as_millis() / SEIZE_STEP.as_millis();
    (0..=tries).find_map(|_| match TERMINAL.try_lock() {
        Ok(held) => Some(held),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => {
            std::thread::sleep(SEIZE_STEP);
            None
        }
    })
}

pub fn remember() {
    imp::remember();
}

/// Puts the settings back; the warning says why they were not.
#[must_use]
pub fn restore() -> Option<String> {
    imp::restore()
}

pub fn discard_typeahead() {
    imp::discard_typeahead();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalQuiet {
    Echo,
    Keys,
}

#[must_use = "the terminal is back to normal as soon as the guard is dropped"]
pub struct QuietedTerminal(imp::TerminalSettings);

impl std::fmt::Debug for QuietedTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Quieted")
    }
}

/// # Errors
///
/// The settings of stdin's terminal could not be read or changed.
pub fn quiet(quiet: TerminalQuiet) -> io::Result<QuietedTerminal> {
    exclusive(|| imp::quiet(quiet)).map(QuietedTerminal)
}

// GUARD: puts the terminal settings back that `quiet` changed.
impl Drop for QuietedTerminal {
    fn drop(&mut self) {
        // Deliver what was written first: a later Ctrl-C would flush it.
        imp::discard_input();
        warn(exclusive(|| imp::put_back(self.0)));
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "a drop has no caller to return the warning to: stderr is the only place left"
)]
fn warn(warning: Option<String>) {
    // A stderr that cannot be written has nowhere to report to.
    warning
        .map_or(Ok(()), |text| writeln!(io::stderr().lock(), "{text}"))
        .unwrap_or_default();
}

#[cfg(unix)]
mod imp {
    use std::{io, mem::MaybeUninit, sync::OnceLock};

    pub(super) type TerminalSettings = libc::termios;

    static ORIGINAL: OnceLock<TerminalSettings> = OnceLock::new();

    fn current() -> io::Result<TerminalSettings> {
        // imperative: `tcgetattr` fills the value through a pointer.
        let mut out = MaybeUninit::<TerminalSettings>::zeroed();
        // SAFETY: `tcgetattr` only writes a `termios` into a sized, aligned local.
        let status = unsafe { libc::tcgetattr(libc::STDIN_FILENO, out.as_mut_ptr()) };
        (status == 0)
            // SAFETY: the call succeeded, so the (zeroed before) value is initialised.
            .then(|| unsafe { out.assume_init() })
            .ok_or_else(io::Error::last_os_error)
    }

    fn write_settings(settings: &TerminalSettings) -> io::Result<()> {
        // SAFETY: `tcsetattr` only reads the `termios` behind the reference.
        let status =
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, settings) };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn remember() {
        if let Ok(settings) = current() {
            ORIGINAL.get_or_init(|| settings);
        }
    }

    pub(super) fn restore() -> Option<String> {
        discard_input();
        ORIGINAL.get().and_then(|settings| put_back(*settings))
    }

    pub(super) fn discard_input() {
        // SAFETY: `tcdrain` takes a descriptor; not a terminal is an ignored error.
        unsafe { libc::tcdrain(libc::STDERR_FILENO) };
    }

    pub(super) fn put_back(settings: TerminalSettings) -> Option<String> {
        write_settings(&settings).err().map(|error| {
            format!("warning: the terminal settings were not put back: {error}")
        })
    }

    pub(super) fn quiet(
        quiet: crate::tty::TerminalQuiet,
    ) -> io::Result<TerminalSettings> {
        // NOFLSH: a Ctrl-C must not flush what the form already wrote.
        let (off, on) = match quiet {
            crate::tty::TerminalQuiet::Echo => (libc::ECHO, libc::NOFLSH),
            crate::tty::TerminalQuiet::Keys => {
                (libc::ECHO | libc::ISIG | libc::ICANON | libc::IEXTEN, 0)
            }
        };
        current().and_then(|before| {
            write_settings(&TerminalSettings {
                c_lflag: (before.c_lflag & !off) | on,
                ..before
            })
            .map(|()| before)
        })
    }

    pub(super) fn discard_typeahead() {
        // SAFETY: `tcflush` takes a descriptor and a constant; failure is ignored.
        unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) };
    }
}

#[cfg(not(unix))]
mod imp {
    use std::io;

    /// None on Windows: `quiet` always fails.
    #[derive(Clone, Copy)]
    pub(super) enum TerminalSettings {}

    pub(super) fn remember() {}

    pub(super) fn restore() -> Option<String> {
        None
    }

    pub(super) fn put_back(_: TerminalSettings) -> Option<String> {
        None
    }

    pub(super) fn quiet(_: crate::tty::TerminalQuiet) -> io::Result<TerminalSettings> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub(super) fn discard_typeahead() {}

    pub(super) fn discard_input() {}
}
