//! Markdown as the blocks and marks of the web editor's rich text; no I/O.

use std::{collections::BTreeSet, ops::Range};

use pulldown_cmark::{
    CodeBlockKind,
    Event,
    HeadingLevel,
    Options,
    Parser,
    Tag,
    TagEnd,
};
use serde::Deserialize;

pub(crate) mod read;
pub mod template;

use crate::domain::board::ObjectKind;

/// How a text is written: lines as paragraphs, or markdown as blocks and marks.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TextFormat {
    #[default]
    Plain,
    Markdown,
}

/// A mark on a run of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Bold,
    Italic,
    Strikethrough,
    Code,
    /// Has no markdown form.
    Underline,
}

impl Mark {
    /// Every mark the editor knows.
    pub const ALL: [Mark; 5] = [
        Mark::Bold,
        Mark::Italic,
        Mark::Underline,
        Mark::Strikethrough,
        Mark::Code,
    ];

    /// The mark of an attribute name.
    #[must_use]
    pub fn of(name: &str) -> Option<Mark> {
        Mark::ALL.into_iter().find(|mark| mark.name() == name)
    }

    /// The name of the attribute.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Mark::Bold => "bold",
            Mark::Italic => "italic",
            Mark::Strikethrough => "strikethrough",
            Mark::Code => "code",
            Mark::Underline => "underline",
        }
    }
}

/// A stretch of text with one set of marks, inside one link or none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub marks: BTreeSet<Mark>,
    pub link: Option<String>,
}

impl Run {
    fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            marks: BTreeSet::new(),
            link: None,
        }
    }
}

/// What a block is in the web editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    HeadingOne,
    HeadingTwo,
    HeadingThree,
    Bullet,
    /// A numbered item; the number is set on the first of a list.
    Numbered(Option<u32>),
    Check(bool),
    Code {
        language: String,
    },
    Divider,
}

impl BlockKind {
    /// The `type` of the element.
    #[must_use]
    pub const fn element(&self) -> &'static str {
        match self {
            BlockKind::Paragraph => "paragraph",
            BlockKind::HeadingOne => "heading-one",
            BlockKind::HeadingTwo => "heading-two",
            BlockKind::HeadingThree => "heading-three",
            BlockKind::Bullet => "ul-list-item",
            BlockKind::Numbered(_) => "ol-list-item",
            BlockKind::Check(_) => "checkbox-list-item",
            BlockKind::Code { .. } => "code-block",
            BlockKind::Divider => "divider",
        }
    }
}

/// A block and the blocks nested under it (an indented list inside an item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub runs: Vec<Run>,
    pub nested: Vec<Block>,
}

impl Block {
    fn new(kind: BlockKind, runs: Vec<Run>) -> Self {
        Self {
            kind,
            runs,
            nested: Vec::new(),
        }
    }

    fn plain(text: &str) -> Self {
        Self::new(BlockKind::Paragraph, plain_runs(text))
    }
}

fn plain_runs(text: &str) -> Vec<Run> {
    Some(text)
        .filter(|text| !text.is_empty())
        .map(|text| vec![Run::plain(text)])
        .unwrap_or_default()
}

/// A kind of text the editor of an object may keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    Headings,
    Checkboxes,
    Nesting,
    CodeBlocks,
    Dividers,
    InlineCode,
}

/// What an editor of an object keeps; the rest it turns into paragraphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor(pub BTreeSet<Feature>);

impl Editor {
    fn keeps(&self, feature: Feature) -> bool {
        self.0.contains(&feature)
    }
}

impl Editor {
    /// The editor of a kind's text (web bundle: sticker lines 202246-202270,
    /// shape 184084-184110, text 198925-198958).
    #[must_use]
    pub fn of(kind: &ObjectKind) -> Self {
        let sticker = *kind == ObjectKind::Sticker;
        Self(
            [
                (Feature::Headings, !sticker),
                (Feature::Checkboxes, *kind != ObjectKind::Shape),
                (Feature::Nesting, !sticker),
            ]
            .into_iter()
            .filter_map(|(feature, kept)| kept.then_some(feature))
            .collect(),
        )
    }
}

impl Editor {
    /// The editor of a board's note: every kind of block and inline code (web
    /// bundle: `availableBlocks: "all"` 234199, `withInlineCode` 241683).
    #[must_use]
    pub fn note() -> Self {
        Self(
            [
                Feature::Headings,
                Feature::Checkboxes,
                Feature::Nesting,
                Feature::CodeBlocks,
                Feature::Dividers,
                Feature::InlineCode,
            ]
            .into(),
        )
    }
}

/// What the markdown had that the editor cannot hold; it stays as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kept {
    Table,
    Image,
    Quote,
    Html,
    Heading,
    Checkbox,
    CodeBlock,
    Divider,
    InlineCode,
    Indentation,
}

impl Kept {
    const fn as_str(self) -> &'static str {
        match self {
            Kept::Table => "table",
            Kept::Image => "image",
            Kept::Quote => "quote",
            Kept::Html => "html",
            Kept::Heading => "heading",
            Kept::Checkbox => "checkbox",
            Kept::CodeBlock => "code block",
            Kept::Divider => "divider",
            Kept::InlineCode => "inline code",
            Kept::Indentation => "indentation",
        }
    }
}

/// The warning for what stayed text, or none.
#[must_use]
pub fn kept_note(kept: &BTreeSet<Kept>) -> Option<String> {
    let words: Vec<&str> = kept.iter().map(|kept| kept.as_str()).collect();
    (!words.is_empty()).then(|| format!("kept as text: {}", words.join(", ")))
}

/// Blocks read from markdown and what stayed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub blocks: Vec<Block>,
    pub kept: BTreeSet<Kept>,
}

/// The blocks of `source` as `editor` keeps them.
#[must_use]
pub fn parse(source: &str, editor: &Editor) -> Parsed {
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_TABLES;
    let events = Parser::new_ext(source, options)
        .into_offset_iter()
        .collect();
    let mut reader = Reader {
        source,
        events,
        at: 0,
        kept: BTreeSet::new(),
    };
    let blocks = reader.blocks();
    let mut kept = reader.kept;
    let blocks = Some(adapt(blocks, editor, &mut kept))
        .filter(|blocks| !blocks.is_empty())
        .unwrap_or_else(|| vec![Block::plain("")]);
    Parsed { blocks, kept }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Span {
    Bold,
    Italic,
    Strike,
    Link(String),
}

/// What a start tag opens.
enum Start {
    Span(Span),
    Image,
    Block(Open),
}

/// A block-level tag, by what the reader does with it.
enum Open {
    Text(BlockKind),
    Code(String),
    List(Option<u64>),
    Quote,
    Html,
    Table,
    Container,
}

fn start(tag: Tag<'_>) -> Start {
    match tag {
        Tag::Paragraph => Start::Block(Open::Text(BlockKind::Paragraph)),
        Tag::Heading { level, .. } => Start::Block(Open::Text(heading(level))),
        Tag::BlockQuote(_) => Start::Block(Open::Quote),
        Tag::CodeBlock(kind) => Start::Block(Open::Code(language(&kind))),
        Tag::HtmlBlock => Start::Block(Open::Html),
        Tag::List(first) => Start::Block(Open::List(first)),
        Tag::Table(_) => Start::Block(Open::Table),
        Tag::Emphasis => Start::Span(Span::Italic),
        Tag::Strong => Start::Span(Span::Bold),
        Tag::Strikethrough => Start::Span(Span::Strike),
        Tag::Link { dest_url, .. } => Start::Span(Span::Link(link_target(&dest_url))),
        Tag::Image { .. } => Start::Image,
        Tag::Item
        | Tag::FootnoteDefinition(_)
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::TableHead
        | Tag::TableRow
        | Tag::TableCell
        | Tag::Superscript
        | Tag::Subscript
        | Tag::MetadataBlock(_) => Start::Block(Open::Container),
    }
}

const fn heading(level: HeadingLevel) -> BlockKind {
    match level {
        HeadingLevel::H1 => BlockKind::HeadingOne,
        HeadingLevel::H2 => BlockKind::HeadingTwo,
        HeadingLevel::H3 | HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => {
            BlockKind::HeadingThree
        }
    }
}

/// The language of a fence; the editor shows `plaintext` for none it knows
/// (bundle line 241380-241383).
fn language(kind: &CodeBlockKind<'_>) -> String {
    match kind {
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .map_or_else(|| "plaintext".to_owned(), str::to_lowercase),
        CodeBlockKind::Indented => "plaintext".to_owned(),
    }
}

/// The address as the editor keeps it: `http://` is added to one with no
/// scheme it knows (bundle lines 240364-240366, 241291-241296).
fn link_target(url: &str) -> String {
    let known = ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| url.starts_with(scheme));
    if known || url.is_empty() {
        url.to_owned()
    } else {
        format!("http://{url}")
    }
}

struct Reader<'a> {
    source: &'a str,
    events: Vec<(Event<'a>, Range<usize>)>,
    at: usize,
    kept: BTreeSet<Kept>,
}

impl Reader<'_> {
    /// The blocks up to the end of the enclosing tag, which stays unread.
    fn blocks(&mut self) -> Vec<Block> {
        let mut out = Vec::new();
        // imperative: the cursor walks the events block by block
        while let Some((event, range)) = self.events.get(self.at).cloned() {
            match event {
                Event::End(_) => break,
                Event::Start(tag) => match start(tag) {
                    Start::Span(_) | Start::Image => out.push(self.paragraph()),
                    Start::Block(open) => {
                        self.at += 1;
                        out.extend(self.open(open, &range));
                    }
                },
                Event::Rule => {
                    self.at += 1;
                    out.push(Block::new(BlockKind::Divider, Vec::new()));
                }
                Event::Text(_)
                | Event::Code(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::TaskListMarker(_) => out.push(self.paragraph()),
            }
        }
        out
    }

    fn paragraph(&mut self) -> Block {
        Block::new(BlockKind::Paragraph, self.inline())
    }

    /// Moves past the end of the tag being read.
    fn end(&mut self) {
        let at_end = matches!(self.events.get(self.at), Some((Event::End(_), _)));
        self.at += usize::from(at_end);
    }

    /// The blocks of the tag just opened, and its end.
    fn container(&mut self) -> Vec<Block> {
        let inner = self.blocks();
        self.end();
        inner
    }

    fn open(&mut self, open: Open, range: &Range<usize>) -> Vec<Block> {
        match open {
            Open::Text(kind) => {
                let runs = self.inline();
                self.end();
                vec![Block::new(kind, runs)]
            }
            Open::Code(language) => {
                let text = self.raw();
                let runs = plain_runs(text.trim_end_matches('\n'));
                vec![Block::new(BlockKind::Code { language }, runs)]
            }
            Open::List(first) => self.list(first),
            Open::Quote => {
                self.kept.insert(Kept::Quote);
                self.container()
            }
            Open::Container => self.container(),
            Open::Html => {
                self.kept.insert(Kept::Html);
                lines(&self.raw())
            }
            Open::Table => {
                self.kept.insert(Kept::Table);
                self.skip_to_end(1);
                lines(self.source.get(range.clone()).unwrap_or_default())
            }
        }
    }

    /// The items of a list: the first of a numbered one carries its number.
    fn list(&mut self, first: Option<u64>) -> Vec<Block> {
        let mut number = first.map(|first| u32::try_from(first).unwrap_or(u32::MAX));
        let mut items = Vec::new();
        // imperative: the cursor walks the items of the list
        while matches!(self.events.get(self.at), Some((Event::Start(Tag::Item), _))) {
            self.at += 1;
            let kind = match first {
                Some(_) => BlockKind::Numbered(number.take()),
                None => BlockKind::Bullet,
            };
            items.push(self.item(kind));
        }
        self.end();
        items
    }

    /// An item: its first paragraph is its text, the other blocks nest under
    /// it.
    fn item(&mut self, kind: BlockKind) -> Block {
        let checked = match self.events.get(self.at) {
            Some((Event::TaskListMarker(done), _)) => Some(*done),
            _ => None,
        };
        self.at += usize::from(checked.is_some());
        let mut content = self.container();
        let head = content
            .first()
            .is_some_and(|block| block.kind == BlockKind::Paragraph)
            .then(|| content.remove(0));
        Block {
            kind: checked.map_or(kind, BlockKind::Check),
            runs: head.map(|head| head.runs).unwrap_or_default(),
            nested: content,
        }
    }

    /// The text of the events up to the end of the tag just opened, and its
    /// end.
    fn raw(&mut self) -> String {
        let rest = self.events.get(self.at..).unwrap_or_default();
        let used = rest
            .iter()
            .take_while(|(event, _)| !matches!(event, Event::End(_)))
            .count();
        let text = rest
            .iter()
            .take(used)
            .filter_map(|(event, _)| match event {
                Event::Text(text) | Event::Html(text) => Some(text.as_ref()),
                Event::Start(_)
                | Event::End(_)
                | Event::Code(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Rule
                | Event::TaskListMarker(_) => None,
            })
            .collect();
        self.at += used + 1;
        text
    }

    /// Moves to the end of the tag opened `depth` levels up, past it.
    fn skip_to_end(&mut self, depth: i32) {
        let rest = self.events.get(self.at..).unwrap_or_default();
        let before = rest
            .iter()
            .scan(depth, |depth, (event, _)| {
                *depth += i32::from(matches!(event, Event::Start(_)))
                    - i32::from(matches!(event, Event::End(_)));
                (*depth > 0).then_some(())
            })
            .count();
        self.at += before + 1;
    }

    /// The runs of the inline events up to a block-level one.
    fn inline(&mut self) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        let mut open: Vec<Span> = Vec::new();
        // imperative: the cursor walks the events of one block
        while let Some((event, range)) = self.events.get(self.at).cloned() {
            match event {
                Event::Text(text)
                | Event::InlineMath(text)
                | Event::DisplayMath(text) => {
                    push(&mut runs, &open, &text, false);
                }
                Event::Code(text) => push(&mut runs, &open, &text, true),
                Event::Html(text) | Event::InlineHtml(text) => {
                    self.kept.insert(Kept::Html);
                    push(&mut runs, &open, &text, false);
                }
                Event::FootnoteReference(label) => {
                    push(&mut runs, &open, &format!("[^{label}]"), false);
                }
                Event::TaskListMarker(done) => {
                    push(&mut runs, &open, if done { "[x] " } else { "[ ] " }, false);
                }
                Event::SoftBreak => push(&mut runs, &open, " ", false),
                Event::HardBreak => push(&mut runs, &open, "\n", false),
                Event::Start(tag) => match start(tag) {
                    Start::Span(span) => open.push(span),
                    Start::Image => {
                        self.kept.insert(Kept::Image);
                        self.at += 1;
                        self.skip_to_end(1);
                        self.at -= 1;
                        let source = self.source.get(range).unwrap_or_default();
                        push(&mut runs, &open, source, false);
                    }
                    Start::Block(_) => break,
                },
                Event::End(end) => {
                    let closes = matches!(
                        end,
                        TagEnd::Emphasis
                            | TagEnd::Strong
                            | TagEnd::Strikethrough
                            | TagEnd::Link
                    );
                    if closes {
                        open.pop();
                    } else {
                        break;
                    }
                }
                Event::Rule => break,
            }
            self.at += 1;
        }
        runs
    }
}

/// `text` as a run with the marks `open` asks for, joined to the last run
/// when the marks and the link are the same.
fn push(runs: &mut Vec<Run>, open: &[Span], text: &str, code: bool) {
    let marks: BTreeSet<Mark> = [
        (open.contains(&Span::Bold), Mark::Bold),
        (open.contains(&Span::Italic), Mark::Italic),
        (open.contains(&Span::Strike), Mark::Strikethrough),
        (code, Mark::Code),
    ]
    .into_iter()
    .filter_map(|(on, mark)| on.then_some(mark))
    .collect();
    let link = open
        .iter()
        .rev()
        .find_map(|span| match span {
            Span::Link(url) => Some(url.clone()),
            Span::Bold | Span::Italic | Span::Strike => None,
        })
        .filter(|url| !url.is_empty());
    match runs.last_mut() {
        Some(last) if last.marks == marks && last.link == link => {
            last.text.push_str(text);
        }
        Some(_) | None => runs.push(Run {
            text: text.to_owned(),
            marks,
            link,
        }),
    }
}

/// A paragraph for each line of `text`.
pub(crate) fn lines(text: &str) -> Vec<Block> {
    text.lines()
        .map(|line| Block::plain(line.trim_end()))
        .collect()
}

/// `blocks` as `editor` keeps them; what it cannot keep is noted in `kept`.
fn adapt(blocks: Vec<Block>, editor: &Editor, kept: &mut BTreeSet<Kept>) -> Vec<Block> {
    blocks
        .into_iter()
        .flat_map(|block| {
            let Block { kind, runs, nested } = block;
            let nested = adapt(nested, editor, kept);
            let own = adapted(kind, runs, editor, kept);
            if editor.keeps(Feature::Nesting) {
                attached(own, nested)
            } else {
                if !nested.is_empty() {
                    kept.insert(Kept::Indentation);
                }
                own.into_iter().chain(nested).collect()
            }
        })
        .collect()
}

/// `nested` under the last of `own`.
fn attached(mut own: Vec<Block>, nested: Vec<Block>) -> Vec<Block> {
    if let Some(last) = own.last_mut() {
        last.nested = nested;
    }
    own
}

/// One block as `editor` keeps it: itself, or paragraphs of its text.
fn adapted(
    kind: BlockKind,
    runs: Vec<Run>,
    editor: &Editor,
    kept: &mut BTreeSet<Kept>,
) -> Vec<Block> {
    let runs = if editor.keeps(Feature::InlineCode) {
        runs
    } else {
        without_code(runs, kept)
    };
    match kind {
        BlockKind::HeadingOne | BlockKind::HeadingTwo | BlockKind::HeadingThree
            if !editor.keeps(Feature::Headings) =>
        {
            kept.insert(Kept::Heading);
            vec![Block::new(BlockKind::Paragraph, runs)]
        }
        BlockKind::Check(done) if !editor.keeps(Feature::Checkboxes) => {
            kept.insert(Kept::Checkbox);
            let mark = Run::plain(if done { "[x] " } else { "[ ] " });
            vec![Block::new(
                BlockKind::Paragraph,
                merged(std::iter::once(mark).chain(runs).collect()),
            )]
        }
        BlockKind::Code { .. } if !editor.keeps(Feature::CodeBlocks) => {
            kept.insert(Kept::CodeBlock);
            let text: String = runs.iter().map(|run| run.text.as_str()).collect();
            lines(&text)
        }
        BlockKind::Divider if !editor.keeps(Feature::Dividers) => {
            kept.insert(Kept::Divider);
            vec![Block::plain("---")]
        }
        same @ (BlockKind::Paragraph
        | BlockKind::HeadingOne
        | BlockKind::HeadingTwo
        | BlockKind::HeadingThree
        | BlockKind::Bullet
        | BlockKind::Numbered(_)
        | BlockKind::Check(_)
        | BlockKind::Code { .. }
        | BlockKind::Divider) => vec![Block::new(same, runs)],
    }
}

/// `runs` with the code mark off, joined where they now agree.
fn without_code(runs: Vec<Run>, kept: &mut BTreeSet<Kept>) -> Vec<Run> {
    if runs.iter().any(|run| run.marks.contains(&Mark::Code)) {
        kept.insert(Kept::InlineCode);
    }
    merged(
        runs.into_iter()
            .map(|mut run| {
                run.marks.remove(&Mark::Code);
                run
            })
            .collect(),
    )
}

fn merged(runs: Vec<Run>) -> Vec<Run> {
    runs.into_iter()
        .fold(Vec::new(), |mut joined: Vec<Run>, run| {
            match joined.last_mut() {
                Some(last) if last.marks == run.marks && last.link == run.link => {
                    last.text.push_str(&run.text);
                }
                Some(_) | None => joined.push(run),
            }
            joined
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Editor {
        Editor(
            [
                Feature::Headings,
                Feature::Checkboxes,
                Feature::Nesting,
                Feature::CodeBlocks,
                Feature::Dividers,
                Feature::InlineCode,
            ]
            .into(),
        )
    }

    fn kinds(blocks: &[Block]) -> Vec<&'static str> {
        blocks.iter().map(|block| block.kind.element()).collect()
    }

    fn text(block: &Block) -> String {
        block.runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn blocks_of_a_note() {
        let parsed = parse(
            "# Plan\n\ntext\n\n- a\n- b\n\n1. x\n2. y\n\n- [ ] open\n- [x] done\n\n```rust\nlet a = 1;\n```\n\n---\n",
            &all(),
        );
        assert_eq!(
            kinds(&parsed.blocks),
            [
                "heading-one",
                "paragraph",
                "ul-list-item",
                "ul-list-item",
                "ol-list-item",
                "ol-list-item",
                "checkbox-list-item",
                "checkbox-list-item",
                "code-block",
                "divider",
            ]
        );
        assert_eq!(
            parsed.blocks.get(4).map(|b| b.kind.clone()),
            Some(BlockKind::Numbered(Some(1)))
        );
        assert_eq!(
            parsed.blocks.get(5).map(|b| b.kind.clone()),
            Some(BlockKind::Numbered(None))
        );
        assert_eq!(
            parsed.blocks.get(7).map(|b| b.kind.clone()),
            Some(BlockKind::Check(true))
        );
        assert_eq!(
            parsed.blocks.get(8).map(|b| (b.kind.clone(), text(b))),
            Some((
                BlockKind::Code {
                    language: "rust".into()
                },
                "let a = 1;".into()
            ))
        );
        assert!(parsed.kept.is_empty());
    }

    #[test]
    fn marks_and_links_make_runs() {
        let parsed = parse("a **b** *c* ~~d~~ `e` [f](example.com)", &all());
        let runs = parsed
            .blocks
            .first()
            .map(|b| b.runs.clone())
            .unwrap_or_default();
        let shown: Vec<(&str, Vec<&str>, Option<&str>)> = runs
            .iter()
            .map(|r| {
                (
                    r.text.as_str(),
                    r.marks.iter().map(|m| m.name()).collect(),
                    r.link.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("a ", vec![], None),
                ("b", vec!["bold"], None),
                (" ", vec![], None),
                ("c", vec!["italic"], None),
                (" ", vec![], None),
                ("d", vec!["strikethrough"], None),
                (" ", vec![], None),
                ("e", vec!["code"], None),
                (" ", vec![], None),
                ("f", vec![], Some("http://example.com")),
            ]
        );
    }

    #[test]
    fn a_nested_list_nests() {
        let parsed = parse("- a\n  - b\n  - c\n- d\n", &all());
        assert_eq!(parsed.blocks.len(), 2);
        let first = parsed.blocks.first().map(|b| (text(b), b.nested.len()));
        assert_eq!(first, Some(("a".into(), 2)));
    }

    #[test]
    fn tables_and_images_stay_text() {
        let parsed = parse(
            "| a | b |\n|---|---|\n| 1 | 2 |\n\nsee ![alt](pic.png) here\n",
            &all(),
        );
        assert_eq!(
            parsed.blocks.iter().map(text).collect::<Vec<_>>(),
            [
                "| a | b |",
                "|---|---|",
                "| 1 | 2 |",
                "see ![alt](pic.png) here"
            ]
        );
        assert_eq!(
            kept_note(&parsed.kept).as_deref(),
            Some("kept as text: table, image")
        );
    }

    #[test]
    fn a_sticker_keeps_no_heading_nesting_or_code() {
        let editor = Editor::of(&ObjectKind::Sticker);
        let parsed = parse("# Plan\n\n- a\n  - `b`\n\n---\n", &editor);
        assert_eq!(
            kinds(&parsed.blocks),
            ["paragraph", "ul-list-item", "ul-list-item", "paragraph"]
        );
        assert_eq!(
            kept_note(&parsed.kept).as_deref(),
            Some("kept as text: heading, divider, inline code, indentation")
        );
    }

    #[test]
    fn nothing_is_one_empty_paragraph() {
        let parsed = parse("", &all());
        assert_eq!(parsed.blocks, [Block::plain("")]);
    }
}
