//! Marks and colours on the whole text of an object, and their read back.

use serde_json::json;
use yrs::Doc;

use super::{added, document_shape, sticker, text, written};
use crate::{
    domain::{
        board::{self, ObjectKind},
        change::{
            build::build,
            tests::{ME, edit, fields_of},
            value::{
                Backdrop,
                BoardChange,
                ChangeSources,
                NewEntry,
                NewObject,
                ObjectPatch,
                TextLook,
            },
        },
        geometry::Point,
        id_source::SequentialIds,
        palette::Color,
        rich_text::{Mark, TextFormat},
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

fn look(
    marks: Option<Vec<Mark>>,
    color: Option<&str>,
    bg: Option<Backdrop>,
) -> Result<TextLook> {
    Ok(TextLook {
        marks,
        color: color.map(str::parse::<Color>).transpose()?,
        background: bg,
    })
}

fn tinted(color: &str) -> Result<Backdrop> {
    Ok(Backdrop::Color(color.parse()?))
}

fn apply(document: &Doc, patch: ObjectPatch) -> Result<()> {
    let ids = SequentialIds::new("e");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(2),
        ids: &ids,
    };
    build(document, &edit("w-1".into(), patch), ME, &context)?;
    Ok(())
}

fn styled(look: TextLook) -> ObjectPatch {
    ObjectPatch {
        look,
        ..ObjectPatch::default()
    }
}

#[test]
fn marks_and_colours_cover_every_piece_of_the_text() -> Result<()> {
    let (document, _) = written(&added(sticker("a **b**\n\nsee [docs](x.com)")?))?;
    apply(
        &document,
        styled(look(
            Some(vec![Mark::Italic, Mark::Underline]),
            Some("red6"),
            Some(tinted("yellow4")?),
        )?),
    )?;
    let all = "<backgroundColor,color,italic,underline>";
    assert_eq!(
        document_shape(&document, "w-1"),
        format!(
            r#"wrapper[paragraph["a b"{all}]]wrapper[paragraph["see "{all}link{{link=http://x.com}}["docs"{all}]]]"#
        )
    );
    let fields = fields_of(&document, "w-1");
    assert_eq!(
        (fields.get("textColor"), fields.get("textBackgroundColor")),
        (
            Some(&json!({"color": "red6", "opacity": 1})),
            Some(&json!({"color": "yellow4", "opacity": 1})),
        )
    );
    Ok(())
}

#[test]
fn none_takes_the_marks_and_the_background_away() -> Result<()> {
    let (document, _) = written(&added(sticker("a **b**")?))?;
    apply(
        &document,
        styled(look(
            Some(vec![Mark::Bold]),
            Some("red6"),
            Some(tinted("yellow4")?),
        )?),
    )?;
    apply(
        &document,
        styled(look(Some(Vec::new()), None, Some(Backdrop::Clear))?),
    )?;
    assert_eq!(
        document_shape(&document, "w-1"),
        r#"wrapper[paragraph["a b"<color>]]"#
    );
    assert_eq!(
        fields_of(&document, "w-1").get("textBackgroundColor"),
        Some(&json!({"color": null, "opacity": 0}))
    );
    Ok(())
}

#[test]
fn the_marks_all_of_the_text_carries_are_read_back() -> Result<()> {
    let (document, _) = written(&added(sticker("a **b**")?))?;
    let marks = |doc: &Doc| {
        board::objects(doc)
            .first()
            .map(|object| board::text_marks(doc, object))
    };
    let before = marks(&document);
    apply(
        &document,
        styled(look(Some(vec![Mark::Italic, Mark::Code]), None, None)?),
    )?;
    let after = marks(&document);
    assert_eq!(
        (before, after),
        (Some(vec![]), Some(vec!["italic", "code"]))
    );
    Ok(())
}

#[test]
fn a_new_text_and_its_marks_come_in_one_edit() -> Result<()> {
    let (document, _) = written(&added(text("old")?))?;
    apply(
        &document,
        ObjectPatch {
            text: Some("# New".into()),
            format: TextFormat::Markdown,
            look: look(Some(vec![Mark::Bold]), None, None)?,
            ..ObjectPatch::default()
        },
    )?;
    assert_eq!(
        document_shape(&document, "w-1"),
        r#"wrapper[heading-one["New"<bold>]]"#
    );
    Ok(())
}

#[test]
fn a_frame_has_no_text_marks() -> Result<()> {
    let document = Doc::new();
    let ids = SequentialIds::new("w");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(1),
        ids: &ids,
    };
    build(
        &document,
        &BoardChange::AddAll(vec![NewEntry {
            new: NewObject::Frame {
                title: "Q4".into(),
                at: Point::new(0.0, 0.0)?,
                size: None,
                color: None,
                look: crate::domain::change::value::FrameEdit::default(),
            },
            frame: None,
            format: TextFormat::Plain,
        }]),
        ME,
        &context,
    )?;
    let refused = apply(&document, styled(look(Some(vec![Mark::Bold]), None, None)?));
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "marks",
            ..
        }))
    ));
    assert_eq!(ObjectKind::Frame.as_str(), "frame");
    Ok(())
}
