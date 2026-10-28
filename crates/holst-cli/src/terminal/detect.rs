//! What the terminal is and whether it shows color, detected once per run.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalType {
    Dumb,
    Capable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamKind {
    Terminal,
    Redirected,
}

impl TerminalType {
    pub(crate) const fn dumb_if(dumb: bool) -> Self {
        if dumb {
            TerminalType::Dumb
        } else {
            TerminalType::Capable
        }
    }
}

impl StreamKind {
    pub(crate) const fn terminal_if(terminal: bool) -> Self {
        if terminal {
            StreamKind::Terminal
        } else {
            StreamKind::Redirected
        }
    }
}

pub(crate) fn mute_color_when(term_type: TerminalType) {
    let capable = term_type == TerminalType::Capable;
    console::set_colors_enabled(console::colors_enabled() && capable);
    console::set_colors_enabled_stderr(console::colors_enabled_stderr() && capable);
}
