//! Joining, text safe for a terminal; the empty-reply marker.

use std::{borrow::Cow, fmt};

use serde_json::Value;

pub const EMPTY: &str = "(empty)";

/// A count with its noun, plural unless the count is one.
///
/// ```
/// use holst_core::domain::text::counted;
/// assert_eq!(counted(1_usize, "object"), "1 object");
/// assert_eq!(counted(0_usize, "object"), "0 objects");
/// ```
#[must_use]
pub fn counted<N>(count: N, noun: &str) -> String
where
    N: Copy + fmt::Display + PartialEq + From<u8>,
{
    let s = if count == N::from(1) { "" } else { "s" };
    format!("{count} {noun}{s}")
}

/// `text` cut after `max_chars` characters, with `…` when something was cut.
///
/// ```
/// use holst_core::domain::text::shortened;
/// assert_eq!(shortened("hello", 5), "hello");
/// assert_eq!(shortened("héllo!", 5), "héllo…");
/// ```
#[must_use]
pub fn shortened(text: &str, max_chars: usize) -> String {
    text.char_indices().nth(max_chars).map_or_else(
        || text.to_owned(),
        |(cut, _)| format!("{}…", text.get(..cut).unwrap_or(text)),
    )
}

/// Items joined by `separator`, written lazily; can be displayed repeatedly.
///
/// ```
/// use holst_core::domain::text::joined;
///
/// let list = joined(["a", "b", "c"], ", ");
/// assert_eq!(format!("{list} / {list}"), "a, b, c / a, b, c");
/// ```
pub fn joined<I>(items: I, separator: &str) -> impl fmt::Display
where
    I: IntoIterator<IntoIter: Clone>,
    I::Item: fmt::Display,
{
    let items = items.into_iter();
    fmt::from_fn(move |f| {
        items.clone().enumerate().try_for_each(|(i, item)| match i {
            0 => write!(f, "{item}"),
            _ => write!(f, "{separator}{item}"),
        })
    })
}

pub const REPLACEMENT: char = '\u{fffd}';

/// Whether a terminal could act on `c`: C0 and C1 controls but tab and newline,
/// DEL, bidi overrides and isolates.
#[must_use]
pub const fn is_unsafe(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{8}'
            | '\u{b}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Untrusted text, e.g. from a board, made safe to print on a terminal.
///
/// ```
/// use holst_core::domain::text::printable;
///
/// assert_eq!(
///     printable("ok\u{1b}]52;c;x\u{7}"),
///     "ok\u{fffd}]52;c;x\u{fffd}"
/// );
/// assert_eq!(printable("two\nlines"), "two\nlines");
/// ```
#[must_use]
pub fn printable(text: &str) -> Cow<'_, str> {
    if text.contains(is_unsafe) {
        Cow::Owned(
            text.chars()
                .map(|c| if is_unsafe(c) { REPLACEMENT } else { c })
                .collect(),
        )
    } else {
        Cow::Borrowed(text)
    }
}

/// `value` with every string that starts with a personal access token prefix
/// replaced by `***`, at any depth.
///
/// ```
/// use holst_core::domain::text::hide_tokens;
/// use serde_json::json;
///
/// assert_eq!(
///     hide_tokens(json!([{ "name": "ci", "token": "holst_pat_abc" }])),
///     json!([{ "name": "ci", "token": "***" }])
/// );
/// ```
#[must_use]
// Recursion is bounded by the depth of the server's reply.
pub fn hide_tokens(value: Value) -> Value {
    match value {
        Value::String(text) if text.starts_with("holst_pat_") => {
            Value::String("***".into())
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(hide_tokens).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| (key, hide_tokens(item)))
                .collect(),
        ),
        other
        @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
    }
}

/// Most bytes of a file name part, below ext4's 255.
pub const FILE_PART_BYTES: usize = 200;

/// Returns `text` as one part of a file name: path separators, characters
/// systems forbid and controls become `-`, spaces run together, dots and
/// dashes are trimmed, at most [`FILE_PART_BYTES`] bytes, cut on a character;
/// never empty. A Windows device name before the first dot gets a `-` after it.
///
/// ```
/// use holst_core::domain::text::file_part;
///
/// assert_eq!(file_part("Планы Q4 / итоги"), "Планы Q4 - итоги");
/// assert_eq!(file_part("../../etc/passwd"), "etc-passwd");
/// assert_eq!(file_part("  \u{1b}  "), "holst");
/// assert_eq!(file_part("Con"), "Con-");
/// assert_eq!(file_part("nul.txt"), "nul-.txt");
/// assert_eq!(file_part("lpt9"), "lpt9-");
/// assert_eq!(file_part("Console"), "Console");
/// ```
#[must_use]
pub fn file_part(text: &str) -> String {
    let forbidden = |c: char| {
        matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            || c.is_control()
            || is_unsafe(c)
    };
    let replaced: String = text
        .chars()
        .map(|c| if forbidden(c) { '-' } else { c })
        .collect();
    let spaced = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let whole = spaced.trim_matches(|c: char| c == '.' || c == '-' || c == ' ');
    let trimmed = whole
        .get(..whole.floor_char_boundary(FILE_PART_BYTES))
        .unwrap_or(whole);
    let calm = trimmed
        .split("--")
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let trimmed_end = calm.trim_end_matches(['.', ' ', '-']);
    let part = if trimmed_end.is_empty() {
        "holst".to_owned()
    } else {
        trimmed_end.to_owned()
    };
    let (stem, extension) = part.split_at(part.find('.').unwrap_or(part.len()));
    if is_device_name(stem) {
        format!("{stem}-{extension}")
    } else {
        part.clone()
    }
}

/// Returns whether Windows opens `stem` as a device, in any case.
fn is_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    let numbered = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
        .is_some_and(|number| matches!(number.as_bytes(), [b'1'..=b'9']));
    numbered || matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
}

pub const LINE_BREAK: &str = " / ";

/// Untrusted text on one line: [`printable`], line breaks as [`LINE_BREAK`].
#[must_use]
pub fn one_line(text: &str) -> Cow<'_, str> {
    if text.contains('\n') {
        let lines = text.lines().filter(|line| !line.trim().is_empty());
        Cow::Owned(joined(lines.map(printable), LINE_BREAK).to_string())
    } else {
        printable(text)
    }
}

/// Nothing to show: `null`, an empty string, list or object, or an object of
/// such.
///
/// ```
/// use holst_core::domain::text::is_blank;
/// use serde_json::json;
///
/// assert!(is_blank(&json!({ "channels": [] })));
/// assert!(!is_blank(&json!({ "count": 0 })));
/// ```
#[must_use]
pub fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.values().all(is_blank),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

/// A file size as a person reads it: bytes, then KiB, MiB, GiB with a tenth.
///
/// ```
/// use holst_core::domain::text::size_text;
/// assert_eq!(size_text(1536_u64), "1.5 KiB");
/// ```
#[must_use]
pub fn size_text<N: TryInto<u64>>(bytes: N) -> String {
    let bytes = bytes.try_into().unwrap_or(u64::MAX);
    let units = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
    units
        .into_iter()
        .find(|&(unit, _)| bytes >= unit)
        .map_or_else(
            || format!("{bytes} B"),
            |(unit, name)| {
                let tenths = bytes.saturating_mul(10) / unit;
                format!("{}.{} {name}", tenths / 10, tenths % 10)
            },
        )
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use rstest::rstest;
    use serde_json::{Value, json};

    use crate::domain::text::{file_part, is_blank, one_line, printable, size_text};

    #[test]
    fn a_file_part_is_cut_by_bytes_on_a_character_boundary() {
        assert!(file_part(&"🎉".repeat(80)).len() <= 200);
        assert!(file_part(&"я".repeat(300)).len() <= 200);
        assert_eq!(file_part(&"a".repeat(300)).len(), 200);
    }

    #[test]
    fn printable_text_has_no_escapes_or_bidi_overrides() {
        let hostile = "a\u{1b}[2Jb\u{9b}c\u{202e}d\u{2066}e\rf\u{7f}g\th\ni";
        assert_eq!(
            printable(hostile),
            "a\u{fffd}[2Jb\u{fffd}c\u{fffd}d\u{fffd}e\u{fffd}f\u{fffd}g\th\ni"
        );
        assert!(matches!(printable("plain текст"), Cow::Borrowed(_)));
    }

    #[rstest::rstest]
    #[case::plain("plain текст", "plain текст")]
    #[case::lines("Цель:\nfirewall", "Цель: / firewall")]
    #[case::windows_breaks("a\r\nb", "a / b")]
    #[case::blank_lines("a\n\n  \nb\n", "a / b")]
    #[case::lone_return("a\rb", "a\u{fffd}b")]
    #[case::unsafe_inside("a\u{1b}[2J\nb", "a\u{fffd}[2J / b")]
    fn one_line_text_is_printable_with_breaks_as_slashes(
        #[case] text: &str,
        #[case] shown: &str,
    ) {
        assert_eq!(one_line(text), shown);
    }

    #[test]
    fn one_line_borrows_text_without_breaks() {
        assert!(matches!(one_line("plain текст"), Cow::Borrowed(_)));
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Blankness {
        Blank,
        Shown,
    }

    #[rstest]
    #[case::null(Value::Null, Blankness::Blank)]
    #[case::empty_list(json!([]), Blankness::Blank)]
    #[case::wrapped_empty_list(json!({ "channels": [], "cursor": null }), Blankness::Blank)]
    #[case::nested(json!({ "a": { "b": "" } }), Blankness::Blank)]
    #[case::zero(json!({ "count": 0 }), Blankness::Shown)]
    #[case::false_flag(json!(false), Blankness::Shown)]
    #[case::one_item(json!({ "channels": [{}] }), Blankness::Shown)]
    fn blank_means_nothing_to_show(#[case] value: Value, #[case] blank: Blankness) {
        assert_eq!(
            is_blank(&value)
                .then_some(Blankness::Blank)
                .map_or(Blankness::Shown, |kind| kind),
            blank
        );
    }

    #[rstest]
    #[case(0, "0 B")]
    #[case(1023, "1023 B")]
    #[case(1024, "1.0 KiB")]
    #[case(1536, "1.5 KiB")]
    #[case(169_869_312, "162.0 MiB")]
    #[case(3 << 30, "3.0 GiB")]
    fn sizes_read_as_a_person_would(#[case] bytes: u64, #[case] expected: &str) {
        assert_eq!(size_text(bytes), expected);
    }
}
