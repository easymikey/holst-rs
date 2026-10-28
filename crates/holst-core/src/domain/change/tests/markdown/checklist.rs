//! Ticking a checklist item of a text, counted in reading order.

use std::num::NonZeroU32;

use yrs::Doc;

use super::{added, document_shape, sticker, text, written};
use crate::{
    domain::{
        change::{
            build::build,
            tests::{ME, edit},
            value::{ChangeSources, ObjectPatch, Tick},
        },
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

const NESTED: &str = "- [ ] a\n  - [ ] b\n- [ ] c";

fn tick(document: &Doc, item: u32, done: bool) -> Result<()> {
    let ids = SequentialIds::new("e");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(2),
        ids: &ids,
    };
    let patch = ObjectPatch {
        tick: NonZeroU32::new(item).map(|item| Tick { item, done }),
        ..ObjectPatch::default()
    };
    build(document, &edit("w-1".into(), patch), ME, &context).map(drop)
}

#[test]
fn an_item_is_ticked_by_its_place_in_reading_order() -> Result<()> {
    let (document, _) = written(&added(text(NESTED)?))?;
    tick(&document, 2, true)?;
    assert_eq!(
        document_shape(&document, "w-1"),
        concat!(
            r#"wrapper[checkbox-list-item{checked=false}["a"]"#,
            r#"wrapper[checkbox-list-item{checked=true}["b"]]]"#,
            r#"wrapper[checkbox-list-item{checked=false}["c"]]"#,
        )
    );
    Ok(())
}

#[test]
fn a_checklist_with_blank_lines_keeps_its_checkboxes() -> Result<()> {
    let (document, _) = written(&added(text("- [x] a\n\n- [ ] b")?))?;
    assert_eq!(
        document_shape(&document, "w-1"),
        concat!(
            r#"wrapper[checkbox-list-item{checked=true}["a"]]"#,
            r#"wrapper[checkbox-list-item{checked=false}["b"]]"#,
        )
    );
    Ok(())
}

#[test]
fn a_ticked_item_is_unticked_and_a_second_tick_changes_nothing() -> Result<()> {
    let (document, _) = written(&added(sticker("- [x] a\n- [ ] b")?))?;
    tick(&document, 1, false)?;
    tick(&document, 2, true)?;
    tick(&document, 2, true)?;
    assert_eq!(
        document_shape(&document, "w-1"),
        concat!(
            r#"wrapper[checkbox-list-item{checked=false}["a"]]"#,
            r#"wrapper[checkbox-list-item{checked=true}["b"]]"#,
        )
    );
    Ok(())
}

#[test]
fn an_item_the_text_does_not_have_is_refused_and_changes_nothing() -> Result<()> {
    let (document, _) = written(&added(text("- [ ] a\n- [ ] b")?))?;
    let before = document_shape(&document, "w-1");
    let refused = tick(&document, 3, true);
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NoCheckItem {
            item: 3,
            count: 2
        }))
    ));
    assert_eq!(document_shape(&document, "w-1"), before);
    Ok(())
}

#[test]
fn a_text_with_no_checklist_has_no_item_to_tick() -> Result<()> {
    let (document, _) = written(&added(text("- a\n- b")?))?;
    let refused = tick(&document, 1, true);
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NoCheckItem {
            count: 0,
            ..
        }))
    ));
    Ok(())
}
