//! What only a sticker takes: its text size and author.

use std::ops::RangeInclusive;

use serde_json::{Value, json};
use yrs::{Map as _, TransactionMut};

use super::{Written, only};
use crate::{
    domain::{
        board::ObjectKind,
        change::write::{WriteTransaction, object_map},
    },
    error::Result,
    protocol::document::{FONT_SIZE, SHOW_AUTHOR},
};

/// The size of a sticker's text: fitted to the note, or fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSize {
    /// The stored size is taken away and the text fits the note.
    Auto,
    Points(u8),
}

impl TextSize {
    /// BUNDLE: the sizes the sticker's size control offers (matrix 4.3).
    pub const RANGE: RangeInclusive<u8> = 4..=64;
}

/// A sticker's own options; a field left out stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickerEdit {
    pub text_size: Option<TextSize>,
    pub author: Option<bool>,
}

impl StickerEdit {
    pub(super) fn field(&self) -> &'static str {
        match (self.text_size, self.author) {
            (Some(_), None) => "text size",
            (None, Some(_)) => "author",
            _ => "text size and author",
        }
    }

    pub(super) fn write(
        &self,
        transaction: &mut TransactionMut<'_>,
        scope: &WriteTransaction<'_>,
        id: &str,
    ) -> Result<Written> {
        let map =
            object_map(transaction, &scope.roots.objects, id, &ObjectKind::Sticker)?;
        // BUNDLE: auto takes the stored size away (matrix 4.3).
        if self.text_size == Some(TextSize::Auto) {
            map.remove(transaction, FONT_SIZE);
        }
        let size = match self.text_size {
            Some(TextSize::Points(points)) => Some((FONT_SIZE, json!(points))),
            Some(TextSize::Auto) | None => None,
        };
        let author = self.author.map(|shown| (SHOW_AUTHOR, Value::Bool(shown)));
        Ok(Written {
            fields: size.into_iter().chain(author).collect(),
            touched: Vec::new(),
        })
    }

    pub(super) fn check(&self, kind: &ObjectKind) -> Result<()> {
        only(kind, &ObjectKind::Sticker, self.field())
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::Value;
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use super::{StickerEdit, TextSize};
    use crate::{
        domain::{
            board::{self, shape_type::ShapeType},
            change::{
                kind_edit::KindEdit,
                tests::{built, built_with_ids},
                value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
            },
            geometry::Point,
            id_source::SequentialIds,
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

    fn a_sticker() -> Result<Doc> {
        board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })
    }

    fn apply(
        document: &Doc,
        text_size: Option<TextSize>,
        author: Option<bool>,
    ) -> Result<()> {
        let patch = ObjectPatch {
            kind_edits: vec![KindEdit::Sticker(StickerEdit { text_size, author })],
            ..ObjectPatch::default()
        };
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: "s-1".into(),
            patch,
            stacking: None,
            tags: None,
        }]);
        built(document, &change).map(drop)
    }

    fn stored(document: &Doc, name: &str) -> Option<Value> {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, "s-1"))
            .and_then(|value| board::json(&value.to_json(&transaction)))
            .and_then(|fields| fields.get(name).cloned())
    }

    #[test]
    fn a_text_size_is_stored_and_auto_takes_it_away() -> Result<()> {
        let document = a_sticker()?;
        apply(&document, Some(TextSize::Points(24)), None)?;
        let set = stored(&document, "fontSize");
        apply(&document, Some(TextSize::Auto), None)?;
        assert_eq!(
            (set, stored(&document, "fontSize")),
            (Some(Value::from(24)), None)
        );
        Ok(())
    }

    #[test]
    fn the_author_is_shown_and_hidden() -> Result<()> {
        let document = a_sticker()?;
        apply(&document, None, Some(true))?;
        let shown = stored(&document, "showAuthor");
        apply(&document, None, Some(false))?;
        assert_eq!(
            (shown, stored(&document, "showAuthor")),
            (Some(Value::Bool(true)), Some(Value::Bool(false)))
        );
        Ok(())
    }

    #[test]
    fn a_field_left_out_stays() -> Result<()> {
        let document = a_sticker()?;
        apply(&document, Some(TextSize::Points(12)), None)?;
        apply(&document, None, Some(true))?;
        assert_eq!(stored(&document, "fontSize"), Some(Value::from(12)));
        Ok(())
    }

    #[test]
    fn a_shape_takes_neither() -> Result<()> {
        let document = board_with(NewObject::Shape {
            text: "box".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
            kind: ShapeType::Square,
        })?;
        let refused = apply(&document, Some(TextSize::Points(24)), None);
        assert!(matches!(
            refused,
            Err(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "text size",
                ..
            }))
        ));
        Ok(())
    }
}
