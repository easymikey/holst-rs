//! A scripted terminal for tests and a tiny screen emulator.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    io,
};

use crate::form::{
    line::Keystroke,
    terminal::{Terminal, TerminalKind},
};

#[derive(Debug)]
pub(crate) struct Script {
    kind: TerminalKind,
    width: Option<usize>,
    keystrokes: RefCell<VecDeque<Keystroke>>,
    lines: RefCell<VecDeque<String>>,
    written: RefCell<String>,
    hidden: Cell<usize>,
    discarded: Cell<usize>,
    echo_works: bool,
    ctrl_c: CtrlC,
    quieted: Cell<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CtrlC {
    Never,
    AtOnce,
    Before,
}

impl Script {
    #[must_use]
    pub(crate) fn new(
        kind: TerminalKind,
        strokes: impl IntoIterator<Item = Keystroke>,
        lines: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            kind,
            width: Some(80),
            keystrokes: RefCell::new(strokes.into_iter().collect()),
            lines: RefCell::new(lines.into_iter().map(str::to_owned).collect()),
            written: RefCell::new(String::new()),
            hidden: Cell::new(0),
            discarded: Cell::new(0),
            echo_works: true,
            ctrl_c: CtrlC::Never,
            quieted: Cell::new(false),
        }
    }

    #[must_use]
    pub(crate) fn interrupted_before(self) -> Self {
        Self {
            ctrl_c: CtrlC::Before,
            ..self
        }
    }

    #[must_use]
    pub(crate) fn was_quieted(&self) -> bool {
        self.quieted.get()
    }

    #[must_use]
    pub(crate) fn pressing_ctrl_c(self) -> Self {
        Self {
            ctrl_c: CtrlC::AtOnce,
            ..self
        }
    }

    #[must_use]
    pub(crate) fn discards(&self) -> usize {
        self.discarded.get()
    }

    #[must_use]
    pub(crate) fn without_hidden_echo(self) -> Self {
        Self {
            echo_works: false,
            ..self
        }
    }

    #[must_use]
    pub(crate) fn written(&self) -> String {
        self.written.borrow().clone()
    }

    #[must_use]
    pub(crate) fn hidden_reads(&self) -> usize {
        self.hidden.get()
    }
}

impl Terminal for Script {
    type QuietTerminal = ();

    fn quiet(&self) {
        self.quieted.set(true);
    }

    fn interrupts(&self) -> u64 {
        u64::from(self.ctrl_c == CtrlC::Before)
    }

    fn kind(&self) -> TerminalKind {
        self.kind
    }

    fn has_color(&self) -> bool {
        false
    }

    fn width(&self) -> Option<usize> {
        self.width
    }

    fn write(&self, text: &str) -> io::Result<()> {
        self.written.borrow_mut().push_str(text);
        Ok(())
    }

    fn keystroke(&self) -> io::Result<Keystroke> {
        self.keystrokes
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }

    fn line(&self) -> io::Result<Option<String>> {
        Ok(self.lines.borrow_mut().pop_front())
    }

    fn hidden<R>(&self, work: impl FnOnce(&Self) -> R) -> io::Result<R> {
        if self.echo_works {
            self.hidden.set(self.hidden.get() + 1);
            Ok(work(self))
        } else {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    fn answering<R>(&self, work: impl FnOnce(&Self) -> R) -> R {
        work(self)
    }

    fn discard_typeahead(&self) {
        self.discarded.set(self.discarded.get() + 1);
    }

    fn interrupt(&self) -> impl Future<Output = ()> {
        let never = (self.ctrl_c == CtrlC::Never).then(std::future::pending::<()>);
        async move {
            crate::terminal::awaiting::optional(never).await;
        }
    }
}

#[must_use]
pub(crate) fn screen(raw: &str) -> String {
    let end = raw.chars().fold(Screen::default(), Screen::feed);
    end.lines
        .iter()
        .map(|line| line.iter().collect::<String>().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

#[must_use]
pub(crate) fn without_escapes(raw: &str) -> String {
    raw.chars()
        .scan(false, |in_escape, c| {
            let (next, keep) = match (*in_escape, c) {
                (false, '\x1b') => (true, None),
                (false, c) => (false, Some(c)),
                (true, 'A'..='Z' | 'a'..='z') => (false, None),
                (true, _) => (true, None),
            };
            *in_escape = next;
            Some(keep)
        })
        .flatten()
        .collect()
}

#[derive(Debug, Default)]
struct Screen {
    lines: Vec<Vec<char>>,
    line_index: usize,
    column: usize,
    escape: Option<String>,
}

impl Screen {
    fn feed(self, c: char) -> Self {
        match (self.escape.as_ref(), c) {
            (Some(_), c) => self.in_escape(c),
            (None, '\x1b') => Self {
                escape: Some(String::new()),
                ..self
            },
            (None, '\n') => {
                let next = self.line_index + 1;
                self.at(next, 0)
            }
            (None, '\r') => {
                let current = self.line_index;
                self.at(current, 0)
            }
            (None, c) => self.put(c),
        }
    }

    fn in_escape(self, c: char) -> Self {
        let seq = self.escape.clone().unwrap_or_default() + c.encode_utf8(&mut [0; 4]);
        match c {
            'A'..='Z' | 'a'..='z' => Self {
                escape: None,
                ..self
            }
            .follow(&seq),
            _ => Self {
                escape: Some(seq),
                ..self
            },
        }
    }

    fn follow(self, seq: &str) -> Self {
        let count = |s: &str| s.parse::<usize>().unwrap_or(1);
        let (line_index, col) = (self.line_index, self.column);
        match seq.strip_prefix('[') {
            Some("2K") => self.clear_line(),
            Some(rest) if rest.ends_with('A') => {
                let up = count(rest.trim_end_matches('A'));
                self.at(line_index.saturating_sub(up), col)
            }
            Some(rest) if rest.ends_with('B') => {
                let down = count(rest.trim_end_matches('B'));
                self.at(line_index + down, col)
            }
            Some(rest) if rest.ends_with('C') => {
                let right = count(rest.trim_end_matches('C'));
                self.at(line_index, col + right)
            }
            _ => self,
        }
    }

    fn at(self, line_index: usize, col: usize) -> Self {
        let lines = grown(self.lines, line_index + 1);
        Self {
            lines,
            line_index,
            column: col,
            ..self
        }
    }

    fn clear_line(self) -> Self {
        let lines = edited(self.lines, self.line_index, Vec::clear);
        Self { lines, ..self }
    }

    fn put(self, c: char) -> Self {
        let col = self.column;
        let lines = edited(self.lines, self.line_index, |line| {
            line.resize(line.len().max(col + 1), ' ');
            line.get_mut(col).into_iter().for_each(|cell| *cell = c);
        });
        Self {
            lines,
            column: col + 1,
            ..self
        }
    }
}

fn grown(mut lines: Vec<Vec<char>>, len: usize) -> Vec<Vec<char>> {
    lines.resize(lines.len().max(len), Vec::new());
    lines
}

fn edited(
    mut lines: Vec<Vec<char>>,
    index: usize,
    edit: impl FnMut(&mut Vec<char>),
) -> Vec<Vec<char>> {
    lines = grown(lines, index + 1);
    lines.get_mut(index).into_iter().for_each(edit);
    lines
}

#[cfg(test)]
mod tests {
    use crate::form::fake::screen;

    #[test]
    fn screen_follows_cursor_moves_and_clears() {
        assert_eq!(screen("a\nb\x1b[1A\r\x1b[2Kc"), "c\nb");
        assert_eq!(screen("\x1b[2mdim\x1b[0m"), "dim");
        assert_eq!(screen("x\n\ny\x1b[1B"), "x\n\ny");
    }
}
