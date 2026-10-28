//! Text measured and wrapped as the web client lays it out in Inter; no I/O.

use itertools::Itertools as _;

const STICKER_FONT_SIZES: [f64; 31] = [
    64.0, 62.0, 60.0, 58.0, 56.0, 54.0, 52.0, 50.0, 48.0, 46.0, 44.0, 42.0, 40.0, 38.0,
    36.0, 34.0, 32.0, 30.0, 28.0, 26.0, 24.0, 22.0, 20.0, 18.0, 16.0, 14.0, 12.0, 10.0,
    8.0, 6.0, 4.0,
];

const SHORT_WORD: usize = 16;

pub const STICKER_LINE_HEIGHT: f64 = 1.25;

/// Width of a character in Inter, as a share of the font size.
#[must_use]
pub fn advance(c: char) -> f64 {
    match c {
        ' ' | '\u{a0}' | 'i' | 'l' | 'j' | '!' | '|' | '.' | ',' | ':' | ';' | '\''
        | 'I' => 0.27,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '"' | '-' => 0.37,
        'm' | 'w' | 'ш' | 'щ' | 'ж' | 'ю' | 'ы' | 'м' => 0.82,
        'M' | 'W' | 'Ш' | 'Щ' | 'Ж' | 'Ю' | 'Ы' | 'М' | '@' | '%' => 0.9,
        '0'..='9' => 0.62,
        'a'..='z' => 0.56,
        'A'..='Z' | 'А'..='Я' | 'Ё' => 0.7,
        'а'..='я' | 'ё' => 0.58,
        c if u32::from(c) >= 0x1100 => 1.1,
        _ => 0.6,
    }
}

/// Width of `text` at `font_size` pixels.
#[must_use]
pub fn width(text: &str, font_size: f64) -> f64 {
    text.chars().map(advance).sum::<f64>() * font_size
}

/// Lines of wrapped text and whether a short word had to be split.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WrappedText {
    pub lines: Vec<String>,
    pub split_short: bool,
}

pub const PARAGRAPH_BREAKS: [char; 5] =
    ['\n', '\u{b}', '\u{c}', '\u{2028}', '\u{2029}'];

/// The lines of `text` as the author broke them, before wrapping.
pub fn paragraphs(text: &str) -> impl Iterator<Item = &str> {
    text.split(PARAGRAPH_BREAKS)
}

/// `text` wrapped to `max_width` pixels at `font_size`: paragraphs on their own
/// lines, words greedily, a word wider than a line split by characters.
#[must_use]
pub fn wrap(text: &str, max_width: f64, font_size: f64) -> WrappedText {
    paragraphs(text)
        .map(|paragraph| wrap_paragraph(paragraph, max_width, font_size))
        .fold(WrappedText::default(), |mut all, part| {
            all.lines.extend(part.lines);
            all.split_short |= part.split_short;
            all
        })
}

#[derive(Debug, Default)]
struct FillingLine {
    lines: Vec<String>,
    current: String,
    advances: f64,
    split_short: bool,
}

impl FillingLine {
    fn break_before(mut self, word: &str, advances: f64) -> Self {
        self.lines.push(std::mem::take(&mut self.current));
        self.current.push_str(word);
        self.advances = advances;
        self
    }
}

fn wrap_paragraph(paragraph: &str, max_width: f64, font_size: f64) -> WrappedText {
    let filled = paragraph.split(' ').filter(|word| !word.is_empty()).fold(
        FillingLine::default(),
        |mut line, word| {
            let gap = (!line.current.is_empty()).then_some(advance(' '));
            let joined = word
                .chars()
                .fold(line.advances + gap.unwrap_or_default(), |sum, c| {
                    sum + advance(c)
                });
            let alone = word.chars().map(advance).sum::<f64>();
            match (joined * font_size <= max_width, line.current.is_empty()) {
                (true, _) => {
                    line.current.extend(gap.map(|_| ' '));
                    line.current.push_str(word);
                    line.advances = joined;
                    line
                }
                (false, false) if alone * font_size <= max_width => {
                    line.break_before(word, alone)
                }
                (false, empty) => {
                    let pieces = split_word(word, max_width, font_size);
                    let Piece {
                        text: last,
                        advances: last_advances,
                    } = pieces.last().cloned().unwrap_or_default();
                    let leading = pieces
                        .iter()
                        .dropping_back(1)
                        .map(|piece| piece.text.clone());
                    let short = word.chars().count() <= SHORT_WORD;
                    let started = if empty {
                        None
                    } else {
                        Some(std::mem::take(&mut line.current))
                    };
                    line.lines.extend(started);
                    line.lines.extend(leading);
                    line.current = last;
                    line.advances = last_advances;
                    line.split_short |= short;
                    line
                }
            }
        },
    );
    WrappedText {
        lines: filled.lines.into_iter().chain([filled.current]).collect(),
        split_short: filled.split_short,
    }
}

/// A piece of a word that did not fit a line, with its width in em.
#[derive(Debug, Clone, Default)]
struct Piece {
    text: String,
    advances: f64,
}

fn split_word(word: &str, max_width: f64, font_size: f64) -> Vec<Piece> {
    word.chars().fold(vec![Piece::default()], |mut pieces, c| {
        let fits = pieces.last().is_some_and(|piece| {
            piece.text.is_empty()
                || piece.advances * font_size + advance(c) * font_size <= max_width
        });
        let start = (!fits).then(Piece::default);
        pieces.extend(start);
        pieces.last_mut().into_iter().for_each(|piece| {
            piece.text.push(c);
            piece.advances += advance(c);
        });
        pieces
    })
}

/// The sticker font size: the largest that fits the text box without
/// splitting a short word, at most `forced` when set; 4 when nothing fits.
#[must_use]
pub fn sticker_font_size(
    text: &str,
    width: f64,
    height: f64,
    forced: Option<f64>,
) -> f64 {
    // Greedy wrap: a smaller size never needs more lines.
    let first_fit = STICKER_FONT_SIZES
        .partition_point(|font_size| !sticker_fits(text, width, height, *font_size));
    let fit = STICKER_FONT_SIZES.get(first_fit).copied().unwrap_or(4.0);
    forced.map_or(fit, |forced| forced.min(fit))
}

/// Whether `text` at `font_size` fits the sticker's text box without splitting
/// a short word.
#[must_use]
pub fn sticker_fits(text: &str, width: f64, height: f64, font_size: f64) -> bool {
    let WrappedText { lines, split_short } = wrap(text, width, font_size);
    count(lines.len()) * font_size * STICKER_LINE_HEIGHT <= height && !split_short
}

/// The sticker sizes tried, largest first.
#[cfg(test)]
#[must_use]
pub const fn sticker_font_sizes() -> &'static [f64] {
    &STICKER_FONT_SIZES
}

/// A count as a float; counts here are small.
#[must_use]
pub fn count(length: usize) -> f64 {
    u32::try_from(length).map_or(f64::from(u32::MAX), f64::from)
}

#[cfg(test)]
mod tests {
    use crate::domain::text_layout::{sticker_font_size, width, wrap};

    #[test]
    fn words_wrap_greedily_and_paragraphs_break() {
        let wrapped = wrap("one two three\nfour", width("one two", 10.0) + 0.1, 10.0);
        assert_eq!(wrapped.lines, ["one two", "three", "four"]);
        assert!(!wrapped.split_short);
    }

    #[test]
    fn a_long_word_is_split_and_flagged_when_short() {
        let wrapped = wrap("abcdefgh", width("abc", 10.0) + 0.1, 10.0);
        assert_eq!(wrapped.lines, ["abc", "def", "gh"]);
        assert!(wrapped.split_short);
        assert_eq!(wrap("", 50.0, 10.0).lines, [""]);
    }

    #[test]
    fn soft_breaks_from_office_apps_start_a_line() {
        assert_eq!(
            wrap("one\u{b}two\u{c}three\u{2028}four", 1000.0, 10.0).lines,
            ["one", "two", "three", "four"]
        );
    }

    #[test]
    fn wrapping_a_long_line_is_linear() {
        let text = "word ".repeat(80_000);
        let started = std::time::Instant::now();
        let wrapped = wrap(&text, 1e12, 14.0);
        let narrow = wrap(&"x".repeat(400_000), 1e12, 14.0);
        assert_eq!((wrapped.lines.len(), narrow.lines.len()), (1, 1));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn sticker_font_shrinks_with_more_text() {
        let short = sticker_font_size("Hi", 160.0, 160.0, None);
        let long = sticker_font_size(&"word ".repeat(40), 160.0, 160.0, None);
        assert_eq!(short.to_string(), "64");
        assert!(long < 20.0, "{long}");
        assert_eq!(
            sticker_font_size("Hi", 160.0, 160.0, Some(18.0)).to_string(),
            "18"
        );
    }
}
