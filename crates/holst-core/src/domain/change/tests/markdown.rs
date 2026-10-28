//! Text written as markdown: the blocks, marks and links the editor reads.

use yrs::{
    Doc,
    Map as _,
    Out,
    ReadTxn,
    Text as _,
    Transact as _,
    XmlTextRef,
    types::{text::YChange, xml::Xml as _},
};

use super::{ME, edit, fields_of};
use crate::{
    domain::{
        board::{self, ObjectKind},
        change::{
            build::build,
            value::{BoardChange, ChangeSources, NewEntry, NewObject, ObjectPatch},
        },
        geometry::Point,
        id_source::SequentialIds,
        rich_text::{Kept, TextFormat},
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

/// An element as `type{attributes}[children]`; text in quotes, its marks in
/// `<>`.
fn shape<T: ReadTxn>(txn: &T, element: &XmlTextRef) -> String {
    let mut attributes: Vec<String> = element
        .attributes(txn)
        .filter(|(name, _)| !matches!(*name, "type" | "key"))
        .map(|(name, value)| format!("{name}={}", value.to_string(txn)))
        .collect();
    attributes.sort();
    let kind = element
        .get_attribute(txn, "type")
        .map(|kind| kind.to_string(txn))
        .unwrap_or_default();
    let children: String = element
        .diff(txn, YChange::identity)
        .into_iter()
        .map(|chunk| match chunk.insert {
            Out::YXmlText(child) => shape(txn, &child),
            other @ (Out::Any(_)
            | Out::YText(_)
            | Out::YArray(_)
            | Out::YMap(_)
            | Out::YXmlElement(_)
            | Out::YXmlFragment(_)
            | Out::YDoc(_)
            | Out::UndefinedRef(_)) => {
                let mut marks: Vec<String> = chunk
                    .attributes
                    .iter()
                    .flat_map(|marks| marks.keys().map(ToString::to_string))
                    .collect();
                marks.sort();
                let marks = marks.join(",");
                format!(
                    "{:?}{}",
                    other.to_string(txn),
                    if marks.is_empty() {
                        String::new()
                    } else {
                        format!("<{marks}>")
                    }
                )
            }
        })
        .collect();
    let attributes = if attributes.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", attributes.join(","))
    };
    format!("{kind}{attributes}[{children}]")
}

/// The wrappers of the text of an object, one after the other.
fn document_shape(document: &Doc, id: &str) -> String {
    let transaction = document.transact();
    let root = fields_of(document, id)
        .get("documentId")
        .and_then(|name| name.as_str().map(str::to_owned))
        .and_then(|name| transaction.get_map("documents")?.get(&transaction, &name))
        .and_then(|out| out.cast::<XmlTextRef>().ok());
    root.map(|root| {
        root.diff(&transaction, YChange::identity)
            .into_iter()
            .filter_map(|chunk| chunk.insert.cast::<XmlTextRef>().ok())
            .map(|wrapper| shape(&transaction, &wrapper))
            .collect()
    })
    .unwrap_or_default()
}

fn added(new: NewObject) -> BoardChange {
    BoardChange::AddAll(vec![NewEntry {
        new,
        frame: None,
        format: TextFormat::Markdown,
    }])
}

fn text(markdown: &str) -> Result<NewObject> {
    Ok(NewObject::Text {
        text: markdown.into(),
        at: Point::new(1.0, 1.0)?,
    })
}

fn sticker(markdown: &str) -> Result<NewObject> {
    Ok(NewObject::Sticker {
        text: markdown.into(),
        at: Point::new(1.0, 1.0)?,
        size: None,
        color: None,
    })
}

fn written(change: &BoardChange) -> Result<(Doc, std::collections::BTreeSet<Kept>)> {
    let document = Doc::new();
    let ids = SequentialIds::new("w");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let built = build(&document, change, ME, &context)?;
    Ok((document, built.kept))
}

#[test]
fn headings_nested_lists_and_links_are_wrappers_and_elements() -> Result<()> {
    let (document, kept) = written(&added(text(
        "# Plan\n\n- a\n  - b\n\nsee [docs](example.com) now",
    )?))?;
    assert_eq!(
        document_shape(&document, "w-1"),
        concat!(
            r#"wrapper[heading-one["Plan"]]"#,
            r#"wrapper[ul-list-item["a"]wrapper[ul-list-item["b"]]]"#,
            r#"wrapper[paragraph["see "link{link=http://example.com}["docs"<color>]" now"]]"#,
        )
    );
    assert!(kept.is_empty());
    Ok(())
}

#[test]
fn marks_and_checkboxes_and_numbers_keep_their_attributes() -> Result<()> {
    let (document, _) = written(&added(text(
        "**bold** *it* ~~gone~~\n\n3. x\n4. y\n\n- [x] done\n- [ ] open",
    )?))?;
    assert_eq!(
        document_shape(&document, "w-1"),
        concat!(
            r#"wrapper[paragraph["bold"<bold>" ""it"<italic>" ""gone"<strikethrough>]]"#,
            r#"wrapper[ol-list-item{counter=3}["x"]]"#,
            r#"wrapper[ol-list-item["y"]]"#,
            r#"wrapper[checkbox-list-item{checked=true}["done"]]"#,
            r#"wrapper[checkbox-list-item{checked=false}["open"]]"#,
        )
    );
    Ok(())
}

#[test]
fn text_after_a_mark_and_non_ascii_land_in_place() -> Result<()> {
    let (document, _) = written(&added(text(
        "**жирный** обычный 😀 [ссылка](https://a.b) конец",
    )?))?;
    assert_eq!(
        document_shape(&document, "w-1"),
        r#"wrapper[paragraph["жирный"<bold>" обычный 😀 "link{link=https://a.b}["ссылка"<color>]" конец"]]"#
    );
    let listed = board::objects(&document);
    assert_eq!(
        listed.first().map(|object| object.text.as_str()),
        Some("жирный обычный 😀 ссылка конец")
    );
    Ok(())
}

#[test]
fn a_sticker_keeps_lists_and_lays_the_rest_flat() -> Result<()> {
    let (document, kept) = written(&added(sticker(
        "# Plan\n\n- [x] a\n  - `b`\n\n| t |\n|---|\n| 1 |",
    )?))?;
    assert_eq!(
        document_shape(&document, "w-1"),
        concat!(
            r#"wrapper[paragraph["Plan"]]"#,
            r#"wrapper[checkbox-list-item{checked=true}["a"]]"#,
            r#"wrapper[ul-list-item["b"]]"#,
            r#"wrapper[paragraph["| t |"]]wrapper[paragraph["|---|"]]wrapper[paragraph["| 1 |"]]"#,
        )
    );
    assert_eq!(
        kept.into_iter().collect::<Vec<_>>(),
        [
            Kept::Table,
            Kept::Heading,
            Kept::InlineCode,
            Kept::Indentation
        ]
    );
    Ok(())
}

#[test]
fn an_edit_in_markdown_replaces_the_text_with_blocks() -> Result<()> {
    let (document, _) = written(&added(sticker("old")?))?;
    let ids = SequentialIds::new("e");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(2),
        ids: &ids,
    };
    let built = build(
        &document,
        &edit(
            "w-1".into(),
            ObjectPatch {
                text: Some("- one\n- **two**".into()),
                format: TextFormat::Markdown,
                ..ObjectPatch::default()
            },
        ),
        ME,
        &context,
    )?;
    assert_eq!(
        document_shape(&document, "w-1"),
        r#"wrapper[ul-list-item["one"]]wrapper[ul-list-item["two"<bold>]]"#
    );
    assert!(built.kept.is_empty());
    Ok(())
}

#[test]
fn a_frame_title_takes_no_markdown() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("w");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    let frame = NewObject::Frame {
        title: "Q4".into(),
        at: Point::new(0.0, 0.0)?,
        size: None,
        color: None,
    };
    build(
        &document,
        &BoardChange::AddAll(vec![NewEntry {
            new: frame,
            frame: None,
            format: TextFormat::Plain,
        }]),
        ME,
        &context,
    )?;
    let refused = build(
        &document,
        &edit(
            "w-1".into(),
            ObjectPatch {
                text: Some("# Q5".into()),
                format: TextFormat::Markdown,
                ..ObjectPatch::default()
            },
        ),
        ME,
        &context,
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "text format",
            ..
        }))
    ));
    assert_eq!(ObjectKind::Frame.as_str(), "frame");
    Ok(())
}
