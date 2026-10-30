//! A flip card: turned over, and its back rewritten.

use serde_json::{Value, json};
use yrs::TransactionMut;

use super::Written;
use crate::{
    domain::{
        board::ObjectKind,
        change::write::{WriteTransaction, write_text},
        kinds::flipcard::{BACK_DOCUMENT, HIGH, SIDE, WIDE},
        rich_text::TextFormat,
    },
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{HEIGHT, POSITION, ROTATION, SCALE, WIDTH, X, Y},
};

pub(super) const FLIP: &str = "flip";
pub(super) const BACK: &str = "back";

pub(super) fn check(kind: &ObjectKind, field: &'static str) -> Result<()> {
    (*kind == ObjectKind::Flipcard)
        .then_some(())
        .ok_or_else(|| {
            Error::BoardChange(BoardChangeError::NotChangeable {
                kind: kind.as_str().to_owned(),
                field,
            })
        })
}

/// The other side up, the width and height swapped, the centre kept (the web
/// client turns the card about its centre, `R2#68`).
pub(super) fn flip(fields: &Value) -> Written {
    let number = |name: &str, fallback: f64| {
        fields.get(name).and_then(Value::as_f64).unwrap_or(fallback)
    };
    let corner = |axis: &str| {
        fields
            .get(POSITION)
            .and_then(|position| position.get(axis))
            .and_then(Value::as_f64)
            .unwrap_or_default()
    };
    let (width, height) = (number(WIDTH, WIDE), number(HEIGHT, HIGH));
    let scale = number(SCALE, 1.0);
    let (sin, cos) = number(ROTATION, 0.0).to_radians().sin_cos();
    let (along, across) = (
        (width - height) * scale / 2.0,
        (height - width) * scale / 2.0,
    );
    let side = match fields.get(SIDE).and_then(Value::as_str) {
        Some("back") => "front",
        _ => "back",
    };
    Written {
        fields: vec![
            (SIDE, json!(side)),
            (WIDTH, json!(height)),
            (HEIGHT, json!(width)),
            (
                POSITION,
                json!({
                    (X): corner(X) + along * cos - across * sin,
                    (Y): corner(Y) + along * sin + across * cos,
                }),
            ),
        ],
        touched: Vec::new(),
    }
}

/// The back document rewritten as `text`, in the format given.
pub(super) fn back(
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    fields: &Value,
    (text, format): (&str, TextFormat),
) -> Written {
    Written {
        fields: write_text(
            transaction,
            scope,
            fields,
            BACK_DOCUMENT,
            (text, format, &ObjectKind::Flipcard),
        ),
        touched: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use crate::{
        domain::{
            board::{self, text_of},
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
            },
            geometry::Point,
            id_source::SequentialIds,
            kinds::flipcard,
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

    fn a_card() -> Result<Doc> {
        let mut made = flipcard::made("Q", "A", None);
        made.at = Point::new(100.0, 50.0)?;
        board_with(NewObject::Made(made))
    }

    fn card_id(document: &Doc) -> String {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.keys(&transaction).next().map(str::to_owned))
            .unwrap_or_default()
    }

    fn edit(document: &Doc, kind_edit: KindEdit) -> Result<()> {
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: card_id(document),
            patch: ObjectPatch {
                kind_edits: vec![kind_edit],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new("r"), &change).map(drop)
    }

    fn stored(document: &Doc) -> Value {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, &card_id(document)))
            .and_then(|value| board::json(&value.to_json(&transaction)))
            .unwrap_or_default()
    }

    fn sides(document: &Doc) -> Vec<String> {
        let fields = stored(document);
        let transaction = document.transact();
        let documents = transaction.get_map("documents");
        ["frontDocumentId", "backDocumentId"]
            .into_iter()
            .filter_map(|name| fields.get(name).and_then(Value::as_str))
            .filter_map(|id| {
                documents.as_ref().map(|map| text_of(&transaction, map, id))
            })
            .collect()
    }

    fn shape(fields: &Value) -> Vec<Value> {
        ["side", "width", "height", "position"]
            .into_iter()
            .map(|name| fields.get(name).cloned().unwrap_or_default())
            .collect()
    }

    #[test]
    fn a_card_turned_twice_is_the_same_card() -> Result<()> {
        let document = a_card()?;
        let before = shape(&stored(&document));
        edit(&document, KindEdit::Flip)?;
        edit(&document, KindEdit::Flip)?;
        assert_eq!(shape(&stored(&document)), before);
        Ok(())
    }

    #[test]
    fn a_portrait_card_turned_swaps_its_sides_about_its_centre() -> Result<()> {
        let document = a_card()?;
        edit(&document, KindEdit::Flip)?;
        assert_eq!(
            shape(&stored(&document)),
            vec![
                json!("back"),
                json!(320),
                json!(220),
                json!({"x": 50, "y": 100})
            ]
        );
        Ok(())
    }

    #[test]
    fn the_back_is_rewritten_and_the_front_stays() -> Result<()> {
        let document = a_card()?;
        edit(&document, KindEdit::Back("B".into(), TextFormat::Plain))?;
        assert_eq!(sides(&document), vec!["Q".to_owned(), "B".to_owned()]);
        Ok(())
    }

    #[test]
    fn a_sticker_takes_neither() -> Result<()> {
        let document = board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })?;
        let refused = [
            KindEdit::Flip,
            KindEdit::Back("B".into(), TextFormat::Plain),
        ]
        .map(|kind_edit| edit(&document, kind_edit).err());
        assert!(matches!(
            refused,
            [
                Some(Error::BoardChange(BoardChangeError::NotChangeable {
                    field: "flip",
                    ..
                })),
                Some(Error::BoardChange(BoardChangeError::NotChangeable {
                    field: "back",
                    ..
                })),
            ]
        ));
        Ok(())
    }
}
