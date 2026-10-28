//! The Slate JSON the editor keeps, read back as markdown; inverse of `parse`.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{BlockKind, Mark};
use crate::domain::{board::slate_text, geometry::whole_u32};

/// A stretch of a line: text with marks, a link around pieces, a mention.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String, BTreeSet<Mark>),
    Link(String, Vec<Piece>),
    Mention(String),
}

/// A block with the blocks nested under it.
struct Node {
    kind: BlockKind,
    pieces: Vec<Piece>,
    nested: Vec<Node>,
}

/// The Slate `nodes` (wrappers, or elements) as markdown. A mention is
/// `@` and its user id: the name is not in the board.
#[must_use]
pub(crate) fn markdown(nodes: &Value) -> String {
    lines(&blocks_of(nodes)).join("\n")
}

fn type_of(node: &Value) -> Option<&str> {
    node.get("type").and_then(Value::as_str)
}

fn children_of(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn blocks_of(nodes: &Value) -> Vec<Node> {
    match nodes {
        Value::Array(items) => items.iter().flat_map(block_nodes).collect(),
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Object(_) => block_nodes(nodes),
    }
}

fn block_nodes(node: &Value) -> Vec<Node> {
    match (type_of(node), node.get("text").and_then(Value::as_str)) {
        (Some("wrapper"), _) => wrapped(children_of(node)),
        (None, Some(_)) => vec![Node {
            kind: BlockKind::Paragraph,
            pieces: pieces(std::slice::from_ref(node)),
            nested: Vec::new(),
        }],
        (Some(_), _) | (None, None) => vec![element(node)],
    }
}

/// A wrapper holds a block, then the wrappers nested under it.
fn wrapped(children: &[Value]) -> Vec<Node> {
    match children.split_first() {
        Some((head, rest)) if type_of(head) != Some("wrapper") => {
            vec![Node {
                nested: rest.iter().flat_map(block_nodes).collect(),
                ..element(head)
            }]
        }
        Some(_) | None => children.iter().flat_map(block_nodes).collect(),
    }
}

fn element(node: &Value) -> Node {
    let kind = kind_of(node);
    let pieces = if matches!(kind, BlockKind::Code { .. }) {
        vec![Piece::Text(slate_text(node), BTreeSet::new())]
    } else {
        pieces(children_of(node))
    };
    Node {
        kind,
        pieces,
        nested: Vec::new(),
    }
}

fn kind_of(node: &Value) -> BlockKind {
    match type_of(node) {
        Some("heading-one") => BlockKind::HeadingOne,
        Some("heading-two") => BlockKind::HeadingTwo,
        Some("heading-three") => BlockKind::HeadingThree,
        Some("ul-list-item") => BlockKind::Bullet,
        // the number is stored as a float
        Some("ol-list-item") => BlockKind::Numbered(
            node.get("counter")
                .and_then(Value::as_f64)
                .and_then(whole_u32),
        ),
        Some("checkbox-list-item") => {
            BlockKind::Check(node.get("checked").and_then(Value::as_bool) == Some(true))
        }
        Some("code-block") => BlockKind::Code {
            language: node
                .get("language")
                .and_then(Value::as_str)
                .filter(|language| *language != "plaintext")
                .unwrap_or_default()
                .to_owned(),
        },
        Some("divider") => BlockKind::Divider,
        Some(_) | None => BlockKind::Paragraph,
    }
}

/// The inline nodes as pieces; neighbouring text of one set of marks is one
/// piece.
fn pieces(children: &[Value]) -> Vec<Piece> {
    let found = children.iter().flat_map(|child| match type_of(child) {
        Some("link") => vec![Piece::Link(
            child
                .get("link")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            pieces(children_of(child)),
        )],
        Some("mention") => vec![Piece::Mention(
            child
                .get("userId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        )],
        Some(_) => pieces(children_of(child)),
        None => child
            .get("text")
            .and_then(Value::as_str)
            .map(|text| Piece::Text(text.to_owned(), marks_of(child)))
            .into_iter()
            .collect(),
    });
    found.fold(Vec::new(), |mut all, piece| {
        match (all.last_mut(), piece) {
            (Some(Piece::Text(text, marks)), Piece::Text(more, same))
                if *marks == same =>
            {
                text.push_str(&more);
            }
            (_, piece) => all.push(piece),
        }
        all
    })
}

fn marks_of(leaf: &Value) -> BTreeSet<Mark> {
    [Mark::Bold, Mark::Italic, Mark::Strikethrough, Mark::Code]
        .into_iter()
        .filter(|mark| leaf.get(mark.name()).and_then(Value::as_bool) == Some(true))
        .collect()
}

const fn is_item(kind: &BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Bullet | BlockKind::Numbered(_) | BlockKind::Check(_)
    )
}

fn blank(node: &Node) -> bool {
    node.kind == BlockKind::Paragraph && node.pieces.iter().all(is_empty)
}

fn is_empty(piece: &Piece) -> bool {
    matches!(piece, Piece::Text(text, _) if text.is_empty())
}

/// The lines of sibling blocks: a blank line between them, except between
/// the items of one list. A numbered item without a number follows the last.
fn lines(nodes: &[Node]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut before: Option<(&Node, u32)> = None;
    // imperative: a block's number and its separator depend on the one before
    for node in nodes.iter().filter(|node| !blank(node)) {
        let number = match (&node.kind, before) {
            (BlockKind::Numbered(Some(number)), _) => *number,
            (BlockKind::Numbered(None), Some((last, number)))
                if matches!(last.kind, BlockKind::Numbered(_)) =>
            {
                number.saturating_add(1)
            }
            (BlockKind::Numbered(None), _) => 1,
            _ => 0,
        };
        let joined = before.is_some_and(|(last, last_number)| {
            is_item(&last.kind)
                && is_item(&node.kind)
                && matches!(last.kind, BlockKind::Numbered(_))
                    == matches!(node.kind, BlockKind::Numbered(_))
                && (!matches!(node.kind, BlockKind::Numbered(_))
                    || number == last_number.saturating_add(1))
        });
        if before.is_some() && !joined {
            out.push(String::new());
        }
        out.extend(block(node, number));
        before = Some((node, number));
    }
    out
}

fn block(node: &Node, number: u32) -> Vec<String> {
    let text = inline(&node.pieces);
    let marker = match &node.kind {
        BlockKind::Bullet => Some("- ".to_owned()),
        BlockKind::Numbered(_) => Some(format!("{number}. ")),
        BlockKind::Check(done) => {
            Some(format!("- [{}] ", if *done { 'x' } else { ' ' }))
        }
        BlockKind::Paragraph
        | BlockKind::HeadingOne
        | BlockKind::HeadingTwo
        | BlockKind::HeadingThree
        | BlockKind::Code { .. }
        | BlockKind::Divider => None,
    };
    let own = match &node.kind {
        BlockKind::HeadingOne => vec![format!("# {}", flat(&text))],
        BlockKind::HeadingTwo => vec![format!("## {}", flat(&text))],
        BlockKind::HeadingThree => vec![format!("### {}", flat(&text))],
        BlockKind::Code { language } => fenced(language, &raw(&node.pieces)),
        BlockKind::Divider => vec!["---".to_owned()],
        BlockKind::Paragraph
        | BlockKind::Bullet
        | BlockKind::Numbered(_)
        | BlockKind::Check(_) => text.split('\n').map(str::to_owned).collect(),
    };
    let below = lines(&node.nested);
    match marker {
        Some(marker) => {
            let pad = " ".repeat(marker.len());
            let apart = !node
                .nested
                .iter()
                .find(|under| !blank(under))
                .is_some_and(|under| is_item(&under.kind));
            own.into_iter()
                .enumerate()
                .map(|(at, line)| {
                    if at == 0 {
                        format!("{marker}{line}")
                    } else {
                        indented(&pad, &line)
                    }
                })
                .chain((apart && !below.is_empty()).then(String::new))
                .chain(below.iter().map(|line| indented(&pad, line)))
                .collect()
        }
        None => own
            .into_iter()
            .chain((!below.is_empty()).then(String::new))
            .chain(below)
            .collect(),
    }
}

fn indented(pad: &str, line: &str) -> String {
    if line.is_empty() {
        String::new()
    } else {
        format!("{pad}{line}")
    }
}

/// A heading is one line.
fn flat(text: &str) -> String {
    text.replace("\\\n", " ").replace('\n', " ")
}

fn raw(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Text(text, _) => Some(text.as_str()),
            Piece::Link(..) | Piece::Mention(_) => None,
        })
        .collect()
}

/// The longest run of backticks in `text`.
fn ticks_in(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

fn fenced(language: &str, code: &str) -> Vec<String> {
    let fence = "`".repeat(ticks_in(code).saturating_add(1).max(3));
    std::iter::once(format!("{fence}{language}"))
        .chain(code.split('\n').map(str::to_owned))
        .chain(std::iter::once(fence))
        .collect()
}

fn inline(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .enumerate()
        .map(|(at, piece)| match piece {
            Piece::Text(text, marks) => spanned(text, marks, at == 0),
            Piece::Link(url, inner) => linked(url, inner),
            Piece::Mention(user) => format!("@{user}"),
        })
        .collect()
}

/// `text` with its marks; the spaces at its edges stay outside the marks,
/// which markdown needs.
fn spanned(text: &str, marks: &BTreeSet<Mark>, start: bool) -> String {
    let blank_edge = |c: char| c != '\n' && c.is_whitespace();
    let core = text.trim_matches(blank_edge);
    let lead = text.len() - text.trim_start_matches(blank_edge).len();
    let trail = text.len() - text.trim_end_matches(blank_edge).len();
    if core.is_empty() {
        escaped(text, start)
    } else {
        spanned_core(text, core, (lead, trail), marks, start)
    }
}

fn spanned_core(
    text: &str,
    core: &str,
    (lead, trail): (usize, usize),
    marks: &BTreeSet<Mark>,
    start: bool,
) -> String {
    let body = if marks.contains(&Mark::Code) {
        code_span(core)
    } else {
        escaped(core, start && lead == 0)
    };
    let marked = [
        (Mark::Strikethrough, "~~"),
        (Mark::Italic, "*"),
        (Mark::Bold, "**"),
    ]
    .into_iter()
    .filter(|(mark, _)| marks.contains(mark))
    .fold(body, |inner, (_, delimiter)| {
        format!("{delimiter}{inner}{delimiter}")
    });
    format!(
        "{}{marked}{}",
        text.get(..lead).unwrap_or_default(),
        text.get(text.len() - trail..).unwrap_or_default()
    )
}

fn code_span(text: &str) -> String {
    let text = text.replace('\n', " ");
    let ticks = "`".repeat(ticks_in(&text) + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{text}{pad}{ticks}")
}

/// A link is `<url>` when its text is its address, else `[text](url)`.
fn linked(url: &str, inner: &[Piece]) -> String {
    let bare = matches!(inner, [Piece::Text(text, marks)] if marks.is_empty() && text == url)
        && ["http://", "https://", "mailto:"]
            .iter()
            .any(|scheme| url.starts_with(scheme))
        && !url.contains([' ', '<', '>']);
    if bare {
        format!("<{url}>")
    } else {
        let target = url.replace('<', "%3C").replace('>', "%3E");
        let target = if target.contains([' ', '(', ')']) {
            format!("<{target}>")
        } else {
            target
        };
        let inside = inline(inner);
        format!("[{inside}]({target})")
    }
}

/// `text` with the characters that would start markdown escaped; `start`
/// says it begins a line.
fn escaped(text: &str, start: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let line_start = |at: usize| {
        at.checked_sub(1)
            .map_or(start, |before| chars.get(before) == Some(&'\n'))
    };
    // The digits of a line before `at`, as in `12.`
    let numbered = |at: usize| {
        let from = chars
            .get(..at)
            .unwrap_or_default()
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |i| i + 1);
        let digits = chars.get(from..at).unwrap_or_default();
        line_start(from)
            && !digits.is_empty()
            && digits.len() < 10
            && digits.iter().all(char::is_ascii_digit)
    };
    chars
        .iter()
        .enumerate()
        .map(|(at, &c)| {
            let next = chars.get(at + 1).copied();
            let before = at.checked_sub(1).and_then(|i| chars.get(i)).copied();
            let space_after = next.is_none_or(char::is_whitespace);
            let escape = match c {
                '\n' => None,
                '\\' | '*' | '`' | '[' | ']' | '<' | '~' => Some(true),
                '_' => Some(
                    !(before.is_some_and(char::is_alphanumeric)
                        && next.is_some_and(char::is_alphanumeric)),
                ),
                '>' => Some(line_start(at)),
                '#' | '+' => Some(line_start(at) && space_after),
                '-' => Some(line_start(at) && (space_after || next == Some('-'))),
                '.' | ')' => Some(space_after && numbered(at)),
                _ => Some(false),
            };
            match escape {
                None => "\\\n".to_owned(),
                Some(true) => format!("\\{c}"),
                Some(false) => c.to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::markdown;

    #[rstest]
    #[case::unknown_blocks_are_paragraphs(
        json!([{ "type": "wrapper", "children": [
            { "type": "quote", "children": [{ "text": "q" }] }] }]),
        "q"
    )]
    #[case::empty_paragraphs_are_dropped(
        json!([
            { "type": "wrapper", "children": [{ "type": "paragraph", "children": [{ "text": "a" }] }] },
            { "type": "wrapper", "children": [{ "type": "paragraph", "children": [{ "text": "" }] }] },
            { "type": "wrapper", "children": [{ "type": "paragraph", "children": [{ "text": "b" }] }] },
        ]),
        "a\n\nb"
    )]
    #[case::numbers_go_on_from_the_counter(
        json!([
            { "type": "wrapper", "children": [{ "type": "ol-list-item", "counter": 3.0, "children": [{ "text": "x" }] }] },
            { "type": "wrapper", "children": [{ "type": "ol-list-item", "children": [{ "text": "y" }] }] },
        ]),
        "3. x\n4. y"
    )]
    #[case::a_new_counter_starts_a_new_list(
        json!([
            { "type": "wrapper", "children": [{ "type": "ol-list-item", "counter": 1.0, "children": [{ "text": "x" }] }] },
            { "type": "wrapper", "children": [{ "type": "ol-list-item", "counter": 1.0, "children": [{ "text": "y" }] }] },
        ]),
        "1. x\n\n1. y"
    )]
    #[case::text_at_the_top_is_a_paragraph(json!([{ "text": "bare" }]), "bare")]
    #[case::spaces_stay_outside_marks(
        json!([{ "type": "wrapper", "children": [{ "type": "paragraph", "children": [
            { "text": "a " }, { "text": " b ", "bold": true }, { "text": "c" }] }] }]),
        "a  **b** c"
    )]
    #[case::a_link_of_the_editor(
        json!([{ "type": "wrapper", "children": [{ "type": "paragraph", "children": [
            { "type": "link", "link": "https://a.b", "children": [{ "text": "x", "color": 1 }] }] }] }]),
        "[x](https://a.b)"
    )]
    #[case::a_paragraph_under_an_item_follows_after_a_blank_line(
        json!([{ "type": "wrapper", "children": [
            { "type": "ul-list-item", "children": [{ "text": "a" }] },
            { "type": "wrapper", "children": [{ "type": "paragraph", "children": [{ "text": "p" }] }] },
        ] }]),
        "- a\n\n  p"
    )]
    fn slate_nodes_read_as_markdown(#[case] nodes: Value, #[case] expected: &str) {
        assert_eq!(markdown(&nodes), expected);
    }
}
