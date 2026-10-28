//! What the form puts on the screen, as text with terminal escapes.

use console::measure_text_width;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    error::InvalidAnswer,
    form::line::{QuestionKind, TypedAnswer},
    output::coloring::Coloring,
};

const CLEAR: &str = "\x1b[2K";
const UP: &str = "\x1b[1A";
const DOWN: &str = "\x1b[1B";
const MASK: &str = "▪";
const ELLIPSIS: &str = "…";
const GUTTER: usize = 3;
const SPINNER_GLYPHS: [&str; 4] = ["◒", "◐", "◓", "◑"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuestionFooter {
    Plain,
    Warn(InvalidAnswer),
}

#[must_use]
pub(crate) fn clear_line() -> String {
    format!("\r{CLEAR}")
}

#[must_use]
pub(crate) fn intro(coloring: Coloring, title: &str) -> String {
    format!(
        "{}  {}\n{}\n",
        coloring.dim("┌"),
        coloring.reverse(&format!(" {title} ")),
        coloring.dim("│")
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrawnInput {
    text: String,
    caret: usize,
}

#[must_use]
pub(crate) fn active(
    coloring: Coloring,
    label: &str,
    drawn: &DrawnInput,
    footer: QuestionFooter,
) -> String {
    let bottom = match footer {
        QuestionFooter::Plain => coloring.cyan("└"),
        QuestionFooter::Warn(invalid) => format!(
            "{}  {}",
            coloring.yellow("└"),
            coloring.code_on(&format!("▲ {invalid}"), |part| coloring.yellow(part))
        ),
    };
    format!(
        "{}  {label}\n{}\n{bottom}{UP}{}",
        coloring.cyan("◆"),
        input(coloring, drawn),
        update(coloring, drawn)
    )
}

#[must_use]
pub(crate) fn update(coloring: Coloring, drawn: &DrawnInput) -> String {
    format!(
        "\r{CLEAR}{}\r\x1b[{}C",
        input(coloring, drawn),
        GUTTER + drawn.caret
    )
}

fn input(coloring: Coloring, drawn: &DrawnInput) -> String {
    format!("{}  {}", coloring.cyan("│"), drawn.text)
}

#[must_use]
pub(crate) fn erase_active() -> String {
    format!("{DOWN}\r{CLEAR}{UP}\r{CLEAR}{UP}\r{CLEAR}")
}

#[must_use]
pub(crate) fn draw_input(
    coloring: Coloring,
    answer: &TypedAnswer,
    width: Option<usize>,
) -> DrawnInput {
    let room = width.map(|width| width.saturating_sub(GUTTER + 1));
    let secret =
        |count: usize| std::iter::repeat_n((MASK, 1), count).collect::<Vec<_>>();
    let units = match answer.kind() {
        QuestionKind::LoginCode | QuestionKind::Password => secret(answer.graphemes()),
        QuestionKind::Text => answer
            .text()
            .graphemes(true)
            .map(|grapheme| (grapheme, measure_text_width(grapheme)))
            .collect(),
    };
    match units.as_slice() {
        [] => DrawnInput {
            text: placeholder(coloring, answer.kind().placeholder()),
            caret: 0,
        },
        units => {
            let (text, caret) = window(units, answer.graphemes_before(), room);
            DrawnInput { text, caret }
        }
    }
}

fn placeholder(coloring: Coloring, text: &str) -> String {
    let first = text.graphemes(true).next().unwrap_or_default();
    format!(
        "{}{}",
        coloring.reverse(first),
        coloring.dim(text.get(first.len()..).unwrap_or_default())
    )
}

type DrawnGrapheme<'a> = (&'a str, usize);

fn window(
    units: &[DrawnGrapheme<'_>],
    at: usize,
    room: Option<usize>,
) -> (String, usize) {
    let all = columns(units);
    match room {
        Some(room) if all > room => {
            let tail = fitting_back(units, units.len(), room.saturating_sub(1));
            if at >= tail {
                windowed(units, tail, units.len(), at)
            } else {
                middle(units, at, room)
            }
        }
        _ => windowed(units, 0, units.len(), at),
    }
}

fn middle(units: &[DrawnGrapheme<'_>], at: usize, room: usize) -> (String, usize) {
    let start = fitting_back(units, at, room / 2);
    let lead = usize::from(start > 0);
    let end = fitting_forward(units, start, room.saturating_sub(lead + 1));
    windowed(units, start, end, at)
}

fn windowed(
    units: &[DrawnGrapheme<'_>],
    start: usize,
    end: usize,
    at: usize,
) -> (String, usize) {
    let lead = ELLIPSIS.repeat(usize::from(start > 0));
    let trail = ELLIPSIS.repeat(usize::from(end < units.len()));
    let visible = units.get(start..end).unwrap_or_default();
    let text = visible
        .iter()
        .fold(lead.clone(), |text, (grapheme, _)| text + grapheme);
    let before = units.get(start..at.clamp(start, end)).unwrap_or_default();
    (text + &trail, measure_text_width(&lead) + columns(before))
}

fn columns(units: &[DrawnGrapheme<'_>]) -> usize {
    units.iter().map(|(_, width)| width).sum()
}

/// Counts how many units from the iterator fit in the budget.
fn fitting<'a>(
    units: impl Iterator<Item = &'a DrawnGrapheme<'a>>,
    budget: usize,
) -> usize {
    units
        .scan(0, |used, (_, width)| {
            *used += width;
            Some(*used)
        })
        .take_while(|used| *used <= budget)
        .count()
}

fn fitting_back(units: &[DrawnGrapheme<'_>], end: usize, budget: usize) -> usize {
    end - fitting(units.get(..end).unwrap_or_default().iter().rev(), budget)
}

fn fitting_forward(units: &[DrawnGrapheme<'_>], start: usize, budget: usize) -> usize {
    start + fitting(units.get(start..).unwrap_or_default().iter(), budget)
}

#[must_use]
pub(crate) fn masked(kind: QuestionKind, text: &str) -> String {
    match kind {
        QuestionKind::LoginCode | QuestionKind::Password => {
            MASK.repeat(text.graphemes(true).count())
        }
        QuestionKind::Text => text.to_owned(),
    }
}

#[must_use]
pub(crate) fn answered(coloring: Coloring, label: &str, value: &str) -> String {
    let value = match value {
        "" => String::new(),
        value => format!("{}  {}\n", coloring.dim("│"), coloring.dim(value)),
    };
    format!(
        "{}  {label}\n{value}{}\n",
        coloring.green("◇"),
        coloring.dim("│")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteTone {
    Plain,
    Faint,
    Warn,
}

#[must_use]
pub(crate) fn boxed(
    coloring: Coloring,
    title: &str,
    lines: &[(NoteTone, String)],
    width: Option<usize>,
) -> String {
    // `◇  ` and ` │` around the text, two columns of padding inside.
    let room = width.map(|width| width.saturating_sub(GUTTER + 3));
    let shown: Vec<(NoteTone, String)> = lines
        .iter()
        .map(|(tone, text)| (*tone, clip(&warned(*tone, text), room)))
        .collect();
    let painted: Vec<String> = shown
        .iter()
        .map(|(tone, text)| toned(coloring, *tone, text))
        .collect();
    let inner = painted
        .iter()
        .map(|text| measure_text_width(text))
        .chain([measure_text_width(title)])
        .max()
        .unwrap_or_default()
        + 2;
    let bar = coloring.dim("│");
    let blank = format!("{bar}{}{bar}\n", " ".repeat(inner + 2));
    let body = painted
        .iter()
        .map(|text| {
            let pad = " ".repeat(inner.saturating_sub(measure_text_width(text)));
            format!("{bar}  {text}{pad}{bar}\n")
        })
        .collect::<Vec<_>>()
        .concat();
    let rule = "─".repeat(inner.saturating_sub(measure_text_width(title) + 1).max(1));
    format!(
        "{}  {title} {}\n{blank}{body}{blank}{}\n{bar}\n",
        coloring.green("◇"),
        coloring.dim(&format!("{rule}╮")),
        coloring.dim(&format!("├{}╯", "─".repeat(inner + 2))),
    )
}

fn warned(tone: NoteTone, text: &str) -> String {
    match tone {
        NoteTone::Warn => format!("▲ {text}"),
        NoteTone::Plain | NoteTone::Faint => text.to_owned(),
    }
}

fn toned(coloring: Coloring, tone: NoteTone, text: &str) -> String {
    match tone {
        NoteTone::Plain => coloring.code(text),
        NoteTone::Faint => coloring.code_on(text, |part| coloring.dim(part)),
        NoteTone::Warn => coloring.code_on(text, |part| coloring.yellow(part)),
    }
}

fn clip(text: &str, room: Option<usize>) -> String {
    match room {
        Some(room) if measure_text_width(text) > room => {
            let budget = room.saturating_sub(measure_text_width(ELLIPSIS));
            let head: String = text
                .graphemes(true)
                .scan(0, |used, grapheme| {
                    *used += measure_text_width(grapheme);
                    (*used <= budget).then_some(grapheme)
                })
                .collect();
            head + ELLIPSIS
        }
        _ => text.to_owned(),
    }
}

#[must_use]
pub(crate) fn cancelled(coloring: Coloring, label: &str, shown: &str) -> String {
    let answer = match shown {
        "" => String::new(),
        shown => format!("{}  {}\n", coloring.dim("│"), coloring.struck(shown)),
    };
    format!(
        "{}  {label}\n{answer}{}\n",
        coloring.red("■"),
        coloring.dim("│")
    )
}

#[must_use]
pub(crate) fn waiting(coloring: Coloring, label: &str) -> String {
    format!("{}  {label}…\n", coloring.magenta("◒"))
}

#[must_use]
pub(crate) fn spinning(coloring: Coloring, label: &str, step: usize) -> String {
    let symbol = SPINNER_GLYPHS
        .get(step % SPINNER_GLYPHS.len())
        .copied()
        .unwrap_or_default();
    format!("{UP}\r{CLEAR}{}  {label}…\n", coloring.magenta(symbol))
}

#[must_use]
pub(crate) fn done_waiting() -> String {
    format!("{UP}\r{CLEAR}")
}

#[must_use]
pub(crate) fn outro(coloring: Coloring, message: &str) -> String {
    format!("{}  {}\n\n", coloring.dim("└"), coloring.code(message))
}

#[must_use]
pub(crate) fn error_outro(
    coloring: Coloring,
    error: &str,
    causes: &[String],
) -> String {
    let causes: String = causes
        .iter()
        .map(|detail| format!("   {detail}\n"))
        .collect::<Vec<_>>()
        .concat();
    format!(
        "{}  {}\n{causes}\n",
        coloring.dim("└"),
        coloring.code_on(error, |part| coloring.red(part))
    )
}

#[cfg(test)]
mod tests {
    use console::measure_text_width;
    use rstest::rstest;

    use crate::{
        error::InvalidAnswer,
        form::{
            draw::{
                CLEAR,
                DOWN,
                DrawnInput,
                ELLIPSIS,
                GUTTER,
                NoteTone,
                QuestionFooter,
                UP,
                active,
                answered,
                boxed,
                cancelled,
                draw_input,
                erase_active,
                error_outro,
                masked,
                spinning,
                update,
            },
            line::{EditorStep, Keystroke, QuestionKind, TypedAnswer},
        },
        output::coloring::Coloring,
    };

    const PLAIN: Coloring = Coloring::Plain;

    fn line(field: QuestionKind, text: &str) -> TypedAnswer {
        text.chars().fold(TypedAnswer::new(field), |line, c| {
            match line.press(Keystroke::Char(c)) {
                EditorStep::Typing(line) => line,
                EditorStep::Submit(_) | EditorStep::Quit(_) => TypedAnswer::new(field),
            }
        })
    }

    #[rstest]
    #[case::three("123", "▪▪▪")]
    fn secret_input_has_one_mask_per_character(#[case] code: &str, #[case] mask: &str) {
        let shown =
            draw_input(PLAIN, &line(QuestionKind::LoginCode, code), Some(80)).text;
        assert_eq!(shown, mask);
        assert!(!shown.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn empty_input_shows_the_placeholder_with_the_cursor_on_it() {
        let empty = draw_input(PLAIN, &TypedAnswer::new(QuestionKind::Text), Some(80));
        assert_eq!(empty.text, QuestionKind::Text.placeholder());
        assert_eq!(empty.caret, 0);
        assert!(update(PLAIN, &empty).ends_with("\r\x1b[3C"));
        let typed = draw_input(PLAIN, &line(QuestionKind::Text, "a"), Some(80));
        assert_eq!(typed.caret, 1);
        assert!(update(PLAIN, &typed).ends_with("\r\x1b[4C"));
    }

    fn moved(field: QuestionKind, text: &str, back: usize) -> TypedAnswer {
        std::iter::repeat_n(Keystroke::Left, back).fold(
            line(field, text),
            |line, stroke| match line.press(stroke) {
                EditorStep::Typing(line) => line,
                EditorStep::Submit(_) | EditorStep::Quit(_) => TypedAnswer::new(field),
            },
        )
    }

    #[rstest]
    #[case::ascii_end("abc", 0, 3)]
    #[case::ascii_middle("abc", 2, 1)]
    #[case::wide("界a", 1, 2)]
    #[case::family("a👨‍👩‍👧b", 1, 3)]
    #[case::combining("cafe\u{301}!", 1, 4)]
    fn caret_counts_columns_before_the_cursor(
        #[case] text: &str,
        #[case] back: usize,
        #[case] column: usize,
    ) {
        assert_eq!(
            draw_input(PLAIN, &moved(QuestionKind::Text, text, back), Some(80)).caret,
            column
        );
    }

    #[test]
    fn secret_caret_moves_over_the_mask() {
        let shown = draw_input(
            PLAIN,
            &moved(QuestionKind::LoginCode, "123456", 2),
            Some(80),
        );
        assert_eq!((shown.text.as_str(), shown.caret), ("▪▪▪▪▪▪", 4));
    }

    #[test]
    fn long_text_scrolls_around_the_cursor() {
        let text = "abcdefghijklmnopqrstuvwxyz";
        let start = draw_input(PLAIN, &moved(QuestionKind::Text, text, 26), Some(14));
        assert_eq!((start.text.as_str(), start.caret), ("abcdefghi…", 0));
        let mid = draw_input(PLAIN, &moved(QuestionKind::Text, text, 13), Some(14));
        assert!(
            mid.text.starts_with('…') && mid.text.ends_with('…'),
            "{}",
            mid.text
        );
        assert_eq!(mid.text.chars().nth(mid.caret), Some('n'));
        assert!(measure_text_width(&mid.text) < 14 - GUTTER);
    }

    #[test]
    fn coloured_placeholder_inverts_its_first_character() {
        let empty = draw_input(
            Coloring::Colored,
            &TypedAnswer::new(QuestionKind::Text),
            Some(80),
        );
        assert!(
            empty.text.starts_with("\x1b[7my\x1b[0m"),
            "{:?}",
            empty.text
        );
    }

    #[test]
    fn long_text_shows_its_end_after_an_ellipsis() {
        let shown =
            draw_input(PLAIN, &line(QuestionKind::Text, "abcdefghij"), Some(10)).text;
        assert_eq!(shown, "…fghij");
        assert!(measure_text_width(&shown) < 10 - GUTTER);
    }

    #[test]
    fn wide_characters_are_counted_by_columns() {
        let shown =
            draw_input(PLAIN, &line(QuestionKind::Text, "界界界界"), Some(9)).text;
        assert_eq!(shown, "…界界");
    }

    #[rstest]
    #[case::unknown_width(None)]
    #[case::zero_width(Some(0))]
    fn unknown_or_tiny_width_does_not_break(#[case] width: Option<usize>) {
        let shown = draw_input(PLAIN, &line(QuestionKind::Text, "abc"), width).text;
        assert!(shown == "abc" || shown == ELLIPSIS, "{shown}");
    }

    #[test]
    fn answered_secret_is_a_mask_of_the_same_length() {
        assert_eq!(masked(QuestionKind::LoginCode, "123456"), "▪▪▪▪▪▪");
        assert_eq!(masked(QuestionKind::Password, "pa ss"), "▪▪▪▪▪");
        assert_eq!(
            masked(QuestionKind::Text, "you@example.com"),
            "you@example.com"
        );
    }

    #[test]
    fn warning_sits_in_the_footer() {
        let x = DrawnInput {
            text: "x".into(),
            caret: 1,
        };
        let block = active(
            PLAIN,
            "Email",
            &x,
            QuestionFooter::Warn(InvalidAnswer::NotEmail),
        );
        assert!(block.contains("└  ▲ enter an address like you@example.com"));
        assert!(block.starts_with("◆  Email\n│  x\n"));
    }

    fn note() -> Vec<(NoteTone, String)> {
        vec![
            (NoteTone::Plain, "Mikhail Avdeev".into()),
            (NoteTone::Faint, "saved to Keychain · 2 cookies".into()),
            (
                NoteTone::Warn,
                "could not terminate the previous session".into(),
            ),
        ]
    }

    #[test]
    fn boxed_note_is_a_clack_box() {
        let expected = "\
◇  Logged in ──────────────────────────────────╮
│                                              │
│  Mikhail Avdeev                              │
│  saved to Keychain · 2 cookies               │
│  ▲ could not terminate the previous session  │
│                                              │
├──────────────────────────────────────────────╯
│
";
        assert_eq!(boxed(PLAIN, "Logged in", &note(), Some(120)), expected);
    }

    #[test]
    fn coloured_box_keeps_its_width_and_paints_by_tone() {
        let colour = boxed(Coloring::Colored, "Logged in", &note(), Some(120));
        let widths: Vec<usize> =
            colour.lines().take(7).map(measure_text_width).collect();
        assert!(
            widths.iter().all(|width| Some(width) == widths.first()),
            "{widths:?}"
        );
        assert!(
            colour.contains("\x1b[33m▲ could not terminate"),
            "{colour:?}"
        );
        assert!(colour.contains("\x1b[2msaved to Keychain"), "{colour:?}");
        assert!(!colour.contains("\x1b[1m"), "no bold: {colour:?}");
    }

    #[test]
    fn narrow_terminal_clips_box_lines() {
        let narrow = boxed(PLAIN, "Logged in", &note(), Some(30));
        assert!(
            narrow.lines().all(|line| measure_text_width(line) <= 30),
            "{narrow}"
        );
        assert!(narrow.contains('…'), "{narrow}");
    }

    #[test]
    fn code_in_a_box_is_cyan_without_backticks() {
        let lines = [(NoteTone::Plain, "run `holst logout`".to_owned())];
        let colour = boxed(Coloring::Colored, "T", &lines, Some(80));
        assert!(colour.contains("\x1b[36mholst logout\x1b[0m"), "{colour:?}");
        assert!(boxed(PLAIN, "T", &lines, Some(80)).contains("`holst logout`"));
    }

    #[test]
    fn cancelled_question_stays_struck_through() {
        assert_eq!(cancelled(PLAIN, "Code", "▪▪"), "■  Code\n│  ▪▪\n│\n");
        assert_eq!(cancelled(PLAIN, "Email", ""), "■  Email\n│\n");
        let colour = cancelled(Coloring::Colored, "Email", "you@");
        assert!(colour.contains("\x1b[31m■"), "{colour:?}");
        assert!(colour.contains("\x1b[2m\x1b[9myou@"), "{colour:?}");
        assert!(
            !colour.contains("\x1b[31m│"),
            "the bar stays grey: {colour:?}"
        );
        assert!(colour.contains("\x1b[2m│"), "{colour:?}");
    }

    #[test]
    fn failure_ends_grey_with_a_red_message() {
        let details = ["cause: x".to_owned()];
        assert_eq!(
            error_outro(PLAIN, "login cancelled", &details),
            "└  login cancelled\n   cause: x\n\n"
        );
        let colour = error_outro(Coloring::Colored, "login cancelled", &[]);
        assert!(colour.starts_with("\x1b[2m└"), "{colour:?}");
        assert!(colour.contains("\x1b[31mlogin cancelled"), "{colour:?}");
    }

    #[test]
    fn spinner_turns_through_the_clack_frames() {
        let frames: Vec<String> =
            (0..5).map(|n| spinning(PLAIN, "Sending", n)).collect();
        let symbols: Vec<char> = frames
            .iter()
            .filter_map(|frame| {
                frame
                    .trim_start_matches(UP)
                    .trim_start_matches('\r')
                    .trim_start_matches(CLEAR)
                    .chars()
                    .next()
            })
            .collect();
        assert_eq!(symbols, ['◒', '◐', '◓', '◑', '◒']);
        assert!(frames.iter().all(|frame| frame.ends_with("  Sending…\n")));
    }

    #[test]
    fn no_colour_means_no_escape_codes_but_cursor_moves() {
        let text = answered(PLAIN, "Email", "you@example.com");
        assert_eq!(text, "◇  Email\n│  you@example.com\n│\n");
        assert!(erase_active().contains(DOWN));
    }
}
