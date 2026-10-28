//! The terminal port of the form, its `console` adapter, and the key mapping.

use std::{io, io::IsTerminal as _};

use console::{Key, Term};

use crate::{
    form::line::{Csi, EscapeTail, Keystroke, csi_final},
    terminal::{
        detect::{StreamKind, TerminalType},
        signals,
        tty,
    },
};

const SIGNAL_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalKind {
    Live,
    Dumb,
    Redirected,
}

pub(crate) trait Terminal {
    /// The terminal kept quiet for the whole form; dropping it restores it.
    type QuietTerminal: std::fmt::Debug;

    /// Quiet the terminal until the form ends (live: no echo, so no `^C`).
    fn quiet(&self) -> Self::QuietTerminal;

    /// How many Ctrl-C signals came since the program started.
    fn interrupts(&self) -> u64;

    /// How the form can talk to the person.
    fn kind(&self) -> TerminalKind;

    fn has_color(&self) -> bool;

    /// The width in columns; `None` when unknown.
    fn width(&self) -> Option<usize>;

    /// Write text as is (it may hold escapes) and flush.
    fn write(&self, text: &str) -> io::Result<()>;

    /// The next keystroke ([`TerminalKind::Live`] only).
    fn keystroke(&self) -> io::Result<Keystroke>;

    /// The next line of stdin without its line ending; `None` at the end.
    fn line(&self) -> io::Result<Option<String>>;

    /// Run `work` with the echo off; it comes back however `work` ends.
    fn hidden<R>(&self, work: impl FnOnce(&Self) -> R) -> io::Result<R>;

    /// Run a blocking read; a signal during it ends the process cleanly.
    fn answering<R>(&self, work: impl FnOnce(&Self) -> R) -> R;

    /// Drop unread input so a paste's rest reaches no question and no shell.
    fn discard_typeahead(&self);

    /// Ctrl-C while the form awaits a request; listening starts with the call.
    fn interrupt(&self) -> impl Future<Output = ()>;
}

#[derive(Debug)]
pub(crate) struct Console {
    stderr: Term,
    kind: TerminalKind,
    color: bool,
}

impl Console {
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "the form console is where the CLI asks whether stdin is a terminal"
    )]
    pub(crate) fn detect(term_type: TerminalType) -> Self {
        tty::remember();
        let stderr = Term::stderr();
        let kind = kind_of(
            StreamKind::terminal_if(io::stdin().is_terminal()),
            StreamKind::terminal_if(stderr.is_term()),
            term_type,
        );
        let color = console::colors_enabled_stderr();
        Self {
            stderr,
            kind,
            color,
        }
    }
}

#[must_use]
const fn kind_of(
    stdin: StreamKind,
    stderr: StreamKind,
    term_type: TerminalType,
) -> TerminalKind {
    match (stdin, stderr, term_type) {
        (StreamKind::Terminal, StreamKind::Terminal, TerminalType::Capable) => {
            TerminalKind::Live
        }
        (StreamKind::Terminal, _, _) => TerminalKind::Dumb,
        (StreamKind::Redirected, _, _) => TerminalKind::Redirected,
    }
}

impl Terminal for Console {
    type QuietTerminal = Option<tty::QuietedTerminal>;

    fn quiet(&self) -> Self::QuietTerminal {
        // Questions switch to raw keys on top; between them the echo stays off.
        match self.kind {
            TerminalKind::Live => tty::quiet(tty::TerminalQuiet::Echo).ok(),
            TerminalKind::Dumb | TerminalKind::Redirected => None,
        }
    }

    fn interrupts(&self) -> u64 {
        signals::interrupts()
    }

    fn kind(&self) -> TerminalKind {
        self.kind
    }

    fn has_color(&self) -> bool {
        self.color
    }

    fn width(&self) -> Option<usize> {
        self.stderr
            .size_checked()
            .map(|(_, columns)| usize::from(columns))
            .filter(|columns| *columns > 0)
    }

    fn write(&self, text: &str) -> io::Result<()> {
        self.stderr
            .write_str(text)
            .and_then(|()| self.stderr.flush())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "a blocking terminal read loop with no clock: the grace pause blocks the thread"
    )]
    fn keystroke(&self) -> io::Result<Keystroke> {
        // `console` reports a read cut by a signal as Ctrl-C: skip it after a pause.
        std::iter::repeat_with(|| tty::exclusive(|| self.stderr.read_key_raw()))
            .find(|read| {
                let cut_short =
                    matches!(read, Ok(Key::CtrlC)) && signals::has_signal_arrived();
                if cut_short {
                    std::thread::sleep(SIGNAL_GRACE);
                }
                !cut_short
            })
            .unwrap_or(Ok(Key::CtrlC))
            .map(keystroke_of)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the form console is where the CLI reads answers from stdin"
    )]
    fn line(&self) -> io::Result<Option<String>> {
        io::stdin()
            .lines()
            .next()
            .transpose()
            .map(|line| line.map(|line| line.trim_end_matches('\r').to_owned()))
    }

    fn hidden<R>(&self, work: impl FnOnce(&Self) -> R) -> io::Result<R> {
        // Only live reads key by key; there Ctrl-C and Ctrl-U stay keys between reads.
        let quiet = match self.kind {
            TerminalKind::Live => tty::TerminalQuiet::Keys,
            TerminalKind::Dumb | TerminalKind::Redirected => tty::TerminalQuiet::Echo,
        };
        tty::quiet(quiet).map(|quieted| {
            let out = work(self);
            drop(quieted);
            out
        })
    }

    fn answering<R>(&self, work: impl FnOnce(&Self) -> R) -> R {
        signals::answering(|| work(self))
    }

    fn discard_typeahead(&self) {
        tty::discard_typeahead();
    }

    fn interrupt(&self) -> impl Future<Output = ()> {
        signals::ctrl_c()
    }
}

#[must_use]
fn keystroke_of(key: Key) -> Keystroke {
    match key {
        Key::Char('\u{1}') | Key::Home => Keystroke::Home,
        Key::Char('\u{2}') | Key::ArrowLeft => Keystroke::Left,
        Key::Char('\u{4}') => Keystroke::EndOfInput,
        Key::Char('\u{5}') | Key::End => Keystroke::End,
        Key::Char('\u{6}') | Key::ArrowRight => Keystroke::Right,
        Key::Char('\u{b}') => Keystroke::ClearAfter,
        Key::Char('\u{15}') => Keystroke::ClearBefore,
        Key::Char('\u{17}') => Keystroke::DeleteWord,
        Key::Char('\u{8}' | '\u{7f}') | Key::Backspace => Keystroke::Backspace,
        Key::Del => Keystroke::Delete,
        Key::Char(c) => Keystroke::Char(c),
        Key::Enter => Keystroke::Enter,
        Key::CtrlC => Keystroke::Interrupt,
        Key::Escape => Keystroke::Cancel,
        Key::UnknownEscSeq(sequence) => escape_of(&sequence),
        // `Key` is non-exhaustive in `console`: the known rest and any later key.
        Key::Unknown
        | Key::ArrowUp
        | Key::ArrowDown
        | Key::Tab
        | Key::BackTab
        | Key::Alt
        | Key::Shift
        | Key::Insert
        | Key::PageUp
        | Key::PageDown
        | _ => Keystroke::Ignored,
    }
}

fn escape_of(sequence: &[char]) -> Keystroke {
    match sequence {
        ['b'] => Keystroke::WordLeft,
        ['f'] => Keystroke::WordRight,
        ['O'] => Keystroke::Escape(EscapeTail::Ss3),
        ['[', rest @ ..] => {
            let csi = rest
                .iter()
                .take_while(|c| !csi_final(**c))
                .fold(Csi::default(), |csi, c| csi.feed(*c));
            rest.iter()
                .copied()
                .find(|c| csi_final(*c))
                .map_or(Keystroke::Escape(EscapeTail::Csi(csi)), |last| {
                    csi.key(last)
                })
        }
        _ => Keystroke::Escape(EscapeTail::Done),
    }
}

#[cfg(test)]
mod tests {
    use console::Key;
    use rstest::rstest;

    use crate::{
        form::{
            line::{Csi, EscapeTail, Keystroke},
            terminal::{TerminalKind, keystroke_of, kind_of},
        },
        terminal::detect::{StreamKind, TerminalType},
    };

    #[rstest]
    #[case::ctrl_d(Key::Char('\u{4}'), Keystroke::EndOfInput)]
    #[case::ctrl_u(Key::Char('\u{15}'), Keystroke::ClearBefore)]
    #[case::ctrl_k(Key::Char('\u{b}'), Keystroke::ClearAfter)]
    #[case::ctrl_a(Key::Char('\u{1}'), Keystroke::Home)]
    #[case::ctrl_e(Key::Char('\u{5}'), Keystroke::End)]
    #[case::ctrl_b(Key::Char('\u{2}'), Keystroke::Left)]
    #[case::arrow_right(Key::ArrowRight, Keystroke::Right)]
    #[case::alt_b(Key::UnknownEscSeq(vec!['b']), Keystroke::WordLeft)]
    #[case::alt_f(Key::UnknownEscSeq(vec!['f']), Keystroke::WordRight)]
    #[case::arrow_up(Key::ArrowUp, Keystroke::Ignored)]
    #[case::ctrl_w(Key::Char('\u{17}'), Keystroke::DeleteWord)]
    #[case::backspace(Key::Backspace, Keystroke::Backspace)]
    #[case::windows_backspace(Key::Char('\u{8}'), Keystroke::Backspace)]
    #[case::del_char(Key::Char('\u{7f}'), Keystroke::Backspace)]
    #[case::ctrl_c(Key::CtrlC, Keystroke::Interrupt)]
    #[case::escape(Key::Escape, Keystroke::Cancel)]
    #[case::enter(Key::Enter, Keystroke::Enter)]
    #[case::arrow(Key::ArrowLeft, Keystroke::Left)]
    #[case::delete(Key::Del, Keystroke::Delete)]
    #[case::tab(Key::Tab, Keystroke::Ignored)]
    #[case::ctrl_arrow(Key::UnknownEscSeq(vec!['[', '1', ';']), Keystroke::Escape(EscapeTail::Csi(Csi::default().feed('1').feed(';'))))]
    #[case::f1(Key::UnknownEscSeq(vec!['O']), Keystroke::Escape(EscapeTail::Ss3))]
    #[case::alt_x(Key::UnknownEscSeq(vec!['x']), Keystroke::Escape(EscapeTail::Done))]
    #[case::finished_csi(Key::UnknownEscSeq(vec!['[', '9', '~']), Keystroke::Ignored)]
    #[case::letter(Key::Char('ж'), Keystroke::Char('ж'))]
    fn keys_map_to_strokes(#[case] key: Key, #[case] stroke: Keystroke) {
        assert_eq!(keystroke_of(key), stroke);
    }

    #[rstest]
    #[case::interactive(
        StreamKind::Terminal,
        StreamKind::Terminal,
        TerminalType::Capable,
        TerminalKind::Live
    )]
    #[case::stderr_redirected(
        StreamKind::Terminal,
        StreamKind::Redirected,
        TerminalType::Capable,
        TerminalKind::Dumb
    )]
    #[case::dumb_terminal(
        StreamKind::Terminal,
        StreamKind::Terminal,
        TerminalType::Dumb,
        TerminalKind::Dumb
    )]
    #[case::piped(
        StreamKind::Redirected,
        StreamKind::Terminal,
        TerminalType::Capable,
        TerminalKind::Redirected
    )]
    #[case::piped_everything(
        StreamKind::Redirected,
        StreamKind::Redirected,
        TerminalType::Capable,
        TerminalKind::Redirected
    )]
    fn streams_decide_the_kind(
        #[case] stdin: StreamKind,
        #[case] stderr: StreamKind,
        #[case] term_type: TerminalType,
        #[case] kind: TerminalKind,
    ) {
        assert_eq!(kind_of(stdin, stderr, term_type), kind);
    }
}
