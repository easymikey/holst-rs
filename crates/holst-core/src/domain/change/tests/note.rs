//! A board's note: a slate-yjs `Y.XmlText` with a wrapper per block.

use rstest::rstest;
use yrs::{
    Doc,
    Text as _,
    Transact as _,
    XmlTextRef,
    types::{text::YChange, xml::Xml as _},
};

use super::ME;
use crate::{
    domain::{
        board::{note_root, note_text},
        change::{
            build::build,
            value::{BoardChange, ChangeSources, ChangeUpdate, NoteContent},
        },
        id_source::SequentialIds,
        rich_text::{Kept, TextFormat, template::NoteTemplate},
        time::Timestamp,
    },
    error::Result,
};

fn write(document: &Doc, note: NoteContent) -> Result<ChangeUpdate> {
    let numbers = SequentialIds::new("n");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &numbers,
    };
    build(document, &BoardChange::Note(note), ME, &context)
}

fn text(text: &str, format: TextFormat) -> NoteContent {
    NoteContent::Text {
        text: text.to_owned(),
        format,
    }
}

#[test]
fn a_markdown_note_is_a_wrapper_per_block_in_an_xml_text_root() -> Result<()> {
    let document = Doc::new();
    write(
        &document,
        text("# Plan\n\n- [x] done\n", TextFormat::Markdown),
    )?;
    let root = note_root(&document);
    let txn = document.transact();
    let wrappers: Vec<String> = root
        .diff(&txn, YChange::identity)
        .into_iter()
        .filter_map(|chunk| chunk.insert.cast::<XmlTextRef>().ok())
        .map(|wrapper| {
            wrapper
                .get_attribute(&txn, "type")
                .map(|kind| kind.to_string(&txn))
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(wrappers, ["wrapper", "wrapper"]);
    drop(txn);
    assert_eq!(
        note_text(&document, TextFormat::Markdown),
        "# Plan\n\n- [x] done"
    );
    Ok(())
}

#[test]
fn a_new_note_replaces_the_old_one() -> Result<()> {
    let document = Doc::new();
    write(&document, text("old\nlines", TextFormat::Plain))?;
    assert_eq!(note_text(&document, TextFormat::Plain), "old\nlines");
    write(&document, text("new", TextFormat::Plain))?;
    assert_eq!(note_text(&document, TextFormat::Plain), "new");
    Ok(())
}

#[test]
fn an_empty_note_empties_the_note() -> Result<()> {
    let document = Doc::new();
    write(&document, text("old", TextFormat::Plain))?;
    write(&document, text("", TextFormat::Plain))?;
    assert_eq!(note_text(&document, TextFormat::Plain), "");
    Ok(())
}

#[test]
fn what_the_note_cannot_hold_is_reported_as_kept() -> Result<()> {
    let document = Doc::new();
    let built = write(&document, text("> quoted\n", TextFormat::Markdown))?;
    assert_eq!(built.kept, [Kept::Quote].into());
    assert!(built.objects.is_empty());
    Ok(())
}

#[rstest]
#[case::todo(
    NoteTemplate::Todo,
    "# To do list\n\n\\[Describe the project you are working on here\\]\n\n- [x] Add an action item here\n- [ ] "
)]
#[case::agenda(
    NoteTemplate::Agenda,
    "# Meeting agenda\n\nDate: \\[Meeting date\\]\n\n## Agenda details\n\n1. Add agenda item\n   - Add agenda details\n2. "
)]
#[case::annotation(
    NoteTemplate::Annotation,
    "# Board annotation\n\n\\[Add text here to tell your team about the purpose of the board and explain how they should interact with it\\]"
)]
fn a_template_reads_back_as_its_markdown(
    #[case] template: NoteTemplate,
    #[case] expected: &str,
) -> Result<()> {
    let document = Doc::new();
    write(&document, NoteContent::Template(template))?;
    assert_eq!(note_text(&document, TextFormat::Markdown), expected);
    Ok(())
}
