//! Pure line editor: a [`TypedAnswer`] and a [`Keystroke`] give [`EditorStep`].

#[cfg(test)] mod tests;

use unicode_segmentation::UnicodeSegmentation as _;
use zeroize::Zeroizing;

const CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuestionKind {
    Text,
    LoginCode,
    Password,
}

impl QuestionKind {
    #[must_use]
    pub(crate) const fn placeholder(self) -> &'static str {
        match self {
            QuestionKind::Text => "you@example.com",
            QuestionKind::LoginCode => "sent to your email",
            QuestionKind::Password => "type the password — it stays hidden",
        }
    }

    fn is_kept(self, c: char) -> bool {
        !c.is_control()
            && match self {
                QuestionKind::LoginCode => !c.is_whitespace(),
                QuestionKind::Text | QuestionKind::Password => true,
            }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keystroke {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
    ClearBefore,
    ClearAfter,
    DeleteWord,
    EndOfInput,
    Interrupt,
    Cancel,
    Escape(EscapeTail),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapeTail {
    Done,
    Csi(Csi),
    Ss3,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Csi {
    parameters: [u16; 2],
    at: usize,
}

impl Csi {
    #[must_use]
    pub(crate) fn feed(self, c: char) -> Self {
        match (c.to_digit(10), c) {
            (Some(digit), _) => Self {
                parameters: with_digit(self.parameters, self.at, digit),
                ..self
            },
            (None, ';') => Self {
                at: self.at.saturating_add(1),
                ..self
            },
            (None, _) => self,
        }
    }

    #[must_use]
    pub(crate) fn key(self, last: char) -> Keystroke {
        let [first, modifier] = self.parameters;
        let by_word = matches!(modifier, 3 | 5 | 7 | 9);
        match (last, by_word, first) {
            ('D', true, _) => Keystroke::WordLeft,
            ('C', true, _) => Keystroke::WordRight,
            ('D', false, _) => Keystroke::Left,
            ('C', false, _) => Keystroke::Right,
            ('H', _, _) | ('~', _, 1 | 7) => Keystroke::Home,
            ('F', _, _) | ('~', _, 4 | 8) => Keystroke::End,
            ('~', _, 3) => Keystroke::Delete,
            _ => Keystroke::Ignored,
        }
    }
}

fn with_digit(mut numbers: [u16; 2], at: usize, digit: u32) -> [u16; 2] {
    numbers.get_mut(at).into_iter().for_each(|param| {
        *param = param
            .saturating_mul(10)
            .saturating_add(u16::try_from(digit).unwrap_or(0));
    });
    numbers
}

#[must_use]
const fn ss3_key(last: char) -> Keystroke {
    match last {
        'H' => Keystroke::Home,
        'F' => Keystroke::End,
        'D' => Keystroke::Left,
        'C' => Keystroke::Right,
        _ => Keystroke::Ignored,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorState {
    Typing,
    Csi(Csi),
    Ss3,
}

#[derive(Debug)]
pub(crate) struct TypedAnswer {
    kind: QuestionKind,
    text: Zeroizing<String>,
    cursor: usize,
    state: EditorState,
}

#[derive(Debug)]
pub(crate) enum EditorStep {
    Typing(TypedAnswer),
    Submit(Zeroizing<String>),
    Quit(Cancellation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cancellation {
    Interrupt,
    Cancel,
}

impl TypedAnswer {
    #[must_use]
    pub(crate) fn new(kind: QuestionKind) -> Self {
        Self {
            kind,
            text: Zeroizing::new(String::with_capacity(CAPACITY)),
            cursor: 0,
            state: EditorState::Typing,
        }
    }

    #[must_use]
    pub(crate) const fn kind(&self) -> QuestionKind {
        self.kind
    }

    #[must_use]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    #[cfg(test)]
    const fn cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub(crate) fn graphemes(&self) -> usize {
        self.text.graphemes(true).count()
    }

    #[must_use]
    pub(crate) fn graphemes_before(&self) -> usize {
        boundaries(&self.text)
            .take_while(|at| *at < self.cursor)
            .count()
    }

    #[must_use]
    pub(crate) fn press(self, keystroke: Keystroke) -> EditorStep {
        match (self.state, keystroke) {
            (EditorState::Csi(csi), Keystroke::Char(c)) if csi_parameter(c) => {
                EditorStep::Typing(self.with_state(EditorState::Csi(csi.feed(c))))
            }
            (EditorState::Csi(csi), Keystroke::Char(c)) if csi_final(c) => {
                self.with_state(EditorState::Typing).edit(csi.key(c))
            }
            (EditorState::Ss3, Keystroke::Char(c)) => {
                self.with_state(EditorState::Typing).edit(ss3_key(c))
            }
            (_, keystroke) => self.with_state(EditorState::Typing).edit(keystroke),
        }
    }

    fn edit(self, keystroke: Keystroke) -> EditorStep {
        let (text, at) = (&*self.text, self.cursor);
        match keystroke {
            Keystroke::Enter => EditorStep::Submit(self.answer()),
            Keystroke::Interrupt => EditorStep::Quit(Cancellation::Interrupt),
            Keystroke::Cancel => EditorStep::Quit(Cancellation::Cancel),
            Keystroke::EndOfInput if text.is_empty() => {
                EditorStep::Quit(Cancellation::Cancel)
            }
            Keystroke::Backspace => {
                let from = before(text, at);
                EditorStep::Typing(self.cut(from, at))
            }
            Keystroke::Delete => {
                let to = after(text, at);
                EditorStep::Typing(self.cut(at, to))
            }
            Keystroke::ClearBefore => EditorStep::Typing(self.cut(0, at)),
            Keystroke::ClearAfter => {
                let end = text.len();
                EditorStep::Typing(self.cut(at, end))
            }
            Keystroke::DeleteWord => {
                let from = spaced_word_start(text, at);
                EditorStep::Typing(self.cut(from, at))
            }
            Keystroke::Left => self.moved_by(before),
            Keystroke::Right => self.moved_by(after),
            Keystroke::WordLeft => self.moved_by(word_start),
            Keystroke::WordRight => self.moved_by(word_end),
            Keystroke::Home => EditorStep::Typing(self.moved(0)),
            Keystroke::End => {
                let end = text.len();
                EditorStep::Typing(self.moved(end))
            }
            Keystroke::Char(c) if self.kind.is_kept(c) => {
                EditorStep::Typing(self.inserted(c))
            }
            Keystroke::Escape(EscapeTail::Csi(csi)) => {
                EditorStep::Typing(self.with_state(EditorState::Csi(csi)))
            }
            Keystroke::Escape(EscapeTail::Ss3) => {
                EditorStep::Typing(self.with_state(EditorState::Ss3))
            }
            Keystroke::Char(_)
            | Keystroke::EndOfInput
            | Keystroke::Escape(EscapeTail::Done)
            | Keystroke::Ignored => EditorStep::Typing(self),
        }
    }

    fn moved_by(self, target: fn(&str, usize) -> usize) -> EditorStep {
        let to = target(&self.text, self.cursor);
        EditorStep::Typing(self.moved(to))
    }

    fn answer(self) -> Zeroizing<String> {
        Zeroizing::new(self.text.trim().to_owned())
    }

    fn with_state(self, mode: EditorState) -> Self {
        Self {
            state: mode,
            ..self
        }
    }

    fn moved(self, to: usize) -> Self {
        Self { cursor: to, ..self }
    }

    fn inserted(self, c: char) -> Self {
        let text = inserted_at(with_room(self.text, c.len_utf8()), self.cursor, c);
        let cursor = snap(&text, self.cursor + c.len_utf8());
        Self {
            text,
            cursor,
            ..self
        }
    }

    fn cut(self, from: usize, to: usize) -> Self {
        let text = without_range(self.text, from, to);
        let cursor = snap(&text, from);
        Self {
            text,
            cursor,
            ..self
        }
    }
}

fn inserted_at(mut text: Zeroizing<String>, at: usize, c: char) -> Zeroizing<String> {
    text.insert(at, c);
    text
}

fn without_range(
    mut text: Zeroizing<String>,
    from: usize,
    to: usize,
) -> Zeroizing<String> {
    text.replace_range(from..to, "");
    text
}

const fn csi_parameter(c: char) -> bool {
    matches!(c, ' '..='?')
}

pub(crate) const fn csi_final(c: char) -> bool {
    matches!(c, '@'..='~')
}

fn boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.grapheme_indices(true)
        .map(|(at, _)| at)
        .chain([text.len()])
}

fn snap(text: &str, at: usize) -> usize {
    boundaries(text).find(|b| *b >= at).unwrap_or(text.len())
}

fn before(text: &str, at: usize) -> usize {
    boundaries(text).take_while(|b| *b < at).last().unwrap_or(0)
}

fn after(text: &str, at: usize) -> usize {
    boundaries(text).find(|b| *b > at).unwrap_or(text.len())
}

fn is_word(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_alphanumeric)
}

fn word_start(text: &str, at: usize) -> usize {
    let back: Vec<(usize, &str)> = text
        .grapheme_indices(true)
        .take_while(|(start, _)| *start < at)
        .collect();
    back.iter()
        .rev()
        .skip_while(|(_, g)| !is_word(g))
        .take_while(|(_, g)| is_word(g))
        .last()
        .map_or(0, |(start, _)| *start)
}

fn word_end(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .skip_while(|(start, _)| *start < at)
        .skip_while(|(_, g)| !is_word(g))
        .take_while(|(_, g)| is_word(g))
        .last()
        .map_or(text.len(), |(start, g)| start + g.len())
}

fn spaced_word_start(text: &str, at: usize) -> usize {
    let back: Vec<(usize, &str)> = text
        .grapheme_indices(true)
        .take_while(|(start, _)| *start < at)
        .collect();
    let blank = |g: &str| g.chars().all(char::is_whitespace);
    back.iter()
        .rev()
        .skip_while(|(_, g)| blank(g))
        .take_while(|(_, g)| !blank(g))
        .last()
        .map_or(0, |(start, _)| *start)
}

fn with_room(text: Zeroizing<String>, extra: usize) -> Zeroizing<String> {
    match text.capacity().checked_sub(text.len()) {
        Some(room) if room >= extra => text,
        _ => {
            let size = text.capacity().saturating_mul(2).max(text.len() + extra);
            Zeroizing::new(String::with_capacity(size) + &text)
        }
    }
}
