//! What only a mind-map node takes: hiding or showing its children.

use serde_json::json;

use super::{Written, only};
use crate::{
    domain::{
        board::ObjectKind,
        kinds::mind_map::{HIDE_CHILDREN, NODE},
    },
    error::Result,
};

pub(super) const CHILDREN: &str = "children";

pub(super) fn write(hidden: bool) -> Written {
    Written {
        fields: vec![(HIDE_CHILDREN, json!(hidden))],
        touched: Vec::new(),
    }
}

pub(super) fn check(kind: &ObjectKind) -> Result<()> {
    only(kind, &ObjectKind::of(NODE), CHILDREN)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use crate::{
        domain::{
            board,
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
            },
            geometry::Point,
            id_source::SequentialIds,
            kinds::mind_map,
            rich_text::TextFormat,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    fn board_with(new: NewObject) -> Result<Doc> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new,
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn edit(document: &Doc, hidden: bool) -> Result<()> {
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: "s-1".into(),
            patch: ObjectPatch {
                kind_edits: vec![KindEdit::Children(hidden)],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new("r"), &change).map(drop)
    }

    fn hidden(document: &Doc) -> Option<Value> {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, "s-1"))
            .and_then(|value| board::json(&value.to_json(&transaction)))
            .and_then(|fields| fields.get("hideChildren").cloned())
    }

    #[test]
    fn hiding_then_showing_writes_the_flag() -> Result<()> {
        let document =
            board_with(NewObject::Made(mind_map::made("Plan", None, None)?))?;
        edit(&document, true)?;
        let after_hide = hidden(&document);
        edit(&document, false)?;
        assert_eq!(
            (after_hide, hidden(&document)),
            (Some(json!(true)), Some(json!(false)))
        );
        Ok(())
    }

    #[test]
    fn a_sticker_is_refused() -> Result<()> {
        let document = board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })?;
        assert!(matches!(
            edit(&document, true),
            Err(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "children",
                ..
            }))
        ));
        Ok(())
    }
}
