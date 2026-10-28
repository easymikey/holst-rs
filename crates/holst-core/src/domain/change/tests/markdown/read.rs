//! Text read back as markdown: what the writer wrote comes out the same.

use rstest::rstest;
use serde_json::json;
use yrs::{Doc, Map as _, Text as _, Transact as _, Xml as _, XmlTextPrelim};

use super::{added, text, written};
use crate::{
    domain::{
        board::{self, Object, ObjectKind, tests::plain_object},
        change::rich::fill_blocks,
        id_source::SequentialIds,
        rich_text::{Editor, Feature, parse},
    },
    error::Result,
};

fn read(document: &Doc) -> Option<String> {
    board::objects(document)
        .first()
        .and_then(|object| board::markdown_text(document, object))
}

#[rstest]
#[case::heading_one("# Plan")]
#[case::heading_two("## Plan")]
#[case::heading_three("### Plan")]
#[case::paragraphs("one\n\ntwo")]
#[case::bold("a **bold** b")]
#[case::italic("a *it* b")]
#[case::strikethrough("a ~~gone~~ b")]
#[case::bold_italic("***both***")]
#[case::marks_apart("**a** *b* ~~c~~")]
#[case::link("see [docs](https://a.b) now")]
#[case::link_with_a_mark("[**docs**](https://a.b)")]
#[case::link_with_parentheses("[x](<https://a.b/c(d)>)")]
#[case::autolink("<https://a.b>")]
#[case::bullets("- a\n- b")]
#[case::numbers_from_three("3. x\n4. y")]
#[case::checkboxes("- [x] done\n- [ ] open")]
#[case::nested_bullets("- a\n  - b\n    - c")]
#[case::bullets_in_numbers("1. a\n   - b\n2. c")]
#[case::list_after_text("# Plan\n\nfirst\n\n- a\n- b\n\nlast")]
#[case::non_ascii("**жирный** обычный 😀 [ссылка](https://a.b)")]
#[case::escaped_marks("a \\* b \\[c\\] \\`d\\`")]
#[case::escaped_line_starts("1\\. not a list\n\n\\# not a heading\n\n\\- not a bullet")]
#[case::hard_break("a\\\nb")]
fn what_the_writer_wrote_reads_back_the_same(#[case] markdown: &str) -> Result<()> {
    let (document, _) = written(&added(text(markdown)?))?;
    assert_eq!(read(&document).as_deref(), Some(markdown));
    Ok(())
}

/// A document written with an editor that keeps every block.
fn rich(markdown: &str) -> Doc {
    let editor = Editor(
        [
            Feature::Headings,
            Feature::Checkboxes,
            Feature::Nesting,
            Feature::CodeBlocks,
            Feature::Dividers,
            Feature::InlineCode,
        ]
        .into_iter()
        .collect(),
    );
    let document = Doc::new();
    let documents = document.get_or_insert_map("documents");
    let ids = SequentialIds::new("w");
    let mut transaction = document.transact_mut();
    let root = documents.insert(&mut transaction, "d1", XmlTextPrelim::new(""));
    fill_blocks(
        &mut transaction,
        &root,
        &parse(markdown, &editor).blocks,
        &ids,
    );
    drop(transaction);
    document
}

fn read_rich(document: &Doc) -> Option<String> {
    let object = Object {
        fields: json!({ "documentId": "d1" }),
        ..plain_object("o1", ObjectKind::Text, "")
    };
    board::markdown_text(document, &object)
}

#[rstest]
#[case::code_block("```rust\nlet a = 1;\n\nlet b = 2;\n```")]
#[case::code_block_without_a_language("```\nplain\n```")]
#[case::code_block_with_fences_inside("````\n```\nx\n```\n````")]
#[case::divider("one\n\n---\n\ntwo")]
#[case::inline_code("use `cargo test` now")]
#[case::inline_code_with_a_tick("a ``x`y`` b")]
#[case::code_in_bold("**`x`**")]
fn blocks_only_a_full_editor_keeps_read_back_the_same(#[case] markdown: &str) {
    assert_eq!(read_rich(&rich(markdown)).as_deref(), Some(markdown));
}

#[test]
fn a_mention_reads_as_at_and_the_user() {
    let document = Doc::new();
    let documents = document.get_or_insert_map("documents");
    let mut transaction = document.transact_mut();
    let root = documents.insert(&mut transaction, "d1", XmlTextPrelim::new(""));
    let wrapper = root.insert_embed(&mut transaction, 0, XmlTextPrelim::new(""));
    wrapper.insert_attribute(&mut transaction, "type", "wrapper");
    let paragraph =
        wrapper.insert_embed(&mut transaction, 0, XmlTextPrelim::new("hi "));
    paragraph.insert_attribute(&mut transaction, "type", "paragraph");
    let before = paragraph.len(&transaction);
    let mention =
        paragraph.insert_embed(&mut transaction, before, XmlTextPrelim::new(""));
    mention.insert_attribute(&mut transaction, "type", "mention");
    mention.insert_attribute(&mut transaction, "userId", "u1");
    let after = paragraph.len(&transaction);
    paragraph.insert(&mut transaction, after, " there");
    drop(transaction);
    assert_eq!(read_rich(&document).as_deref(), Some("hi @u1 there"));
}

#[test]
fn an_object_with_no_text_document_reads_empty_and_one_without_text_reads_none() {
    let document = Doc::new();
    let with_document = Object {
        fields: json!({ "documentId": "missing" }),
        ..plain_object("o1", ObjectKind::Text, "")
    };
    let without = plain_object("o2", ObjectKind::Shape, "");
    assert_eq!(
        (
            board::markdown_text(&document, &with_document),
            board::markdown_text(&document, &without)
        ),
        (Some(String::new()), None)
    );
}

#[test]
fn an_arrow_label_in_slate_json_reads_as_markdown() {
    let arrow = Object {
        fields: json!({ "texts": [
            { "editor": [{ "type": "wrapper", "children": [
                { "type": "paragraph", "children": [
                    { "text": "via " }, { "text": "api", "bold": true }
                ] }
            ] }] },
            { "text": "plain label" }
        ] }),
        ..plain_object("a1", ObjectKind::Arrow, "")
    };
    assert_eq!(
        board::markdown_text(&Doc::new(), &arrow).as_deref(),
        Some("via **api**\n\nplain label")
    );
}
