//! What the terminal is and whether it shows color, detected once per run.

use std::io::{self, IsTerminal as _};

pub(crate) use clacky::terminal::detect::{StreamKind, TerminalType};

/// Stdin, stdout and stderr, in that order.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "detect is where the CLI asks whether the standard streams are terminals"
)]
pub(crate) fn standard_streams() -> [StreamKind; 3] {
    [
        StreamKind::terminal_if(io::stdin().is_terminal()),
        StreamKind::terminal_if(io::stdout().is_terminal()),
        StreamKind::terminal_if(io::stderr().is_terminal()),
    ]
}

pub(crate) fn mute_color_when(term_type: TerminalType) {
    let capable = term_type == TerminalType::Capable;
    console::set_colors_enabled(console::colors_enabled() && capable);
    console::set_colors_enabled_stderr(console::colors_enabled_stderr() && capable);
}
