//! What the terminal is and whether it shows color, detected once per run.

use std::io::{self, IsTerminal as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalType {
    Dumb,
    Capable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Terminal,
    Redirected,
}

impl TerminalType {
    #[must_use]
    pub const fn dumb_if(dumb: bool) -> Self {
        if dumb {
            TerminalType::Dumb
        } else {
            TerminalType::Capable
        }
    }
}

impl StreamKind {
    #[must_use]
    pub const fn terminal_if(terminal: bool) -> Self {
        if terminal {
            StreamKind::Terminal
        } else {
            StreamKind::Redirected
        }
    }
}

/// Stdin, stdout and stderr, in that order.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "detect is where the CLI asks whether the standard streams are terminals"
)]
pub fn standard_streams() -> [StreamKind; 3] {
    [
        StreamKind::terminal_if(io::stdin().is_terminal()),
        StreamKind::terminal_if(io::stdout().is_terminal()),
        StreamKind::terminal_if(io::stderr().is_terminal()),
    ]
}

pub fn mute_color_when(term_type: TerminalType) {
    let capable = term_type == TerminalType::Capable;
    console::set_colors_enabled(console::colors_enabled() && capable);
    console::set_colors_enabled_stderr(console::colors_enabled_stderr() && capable);
}
