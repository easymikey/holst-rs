//! A flip card: a text on each side, one of them up.

use serde_json::{Value, json};
use yrs::{Doc, Transact as _};

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind, text_of},
        change::value::Made,
        geometry::{Point, Size},
        palette::Color,
    },
    protocol::document::{COLOR, DOCUMENTS, FILL_COLOR, HEIGHT, OPACITY, SCALE, WIDTH},
};

pub(crate) const FRONT_DOCUMENT: &str = "frontDocumentId";
pub(crate) const BACK_DOCUMENT: &str = "backDocumentId";
pub(crate) const SIDE: &str = "side";
const BACK: &str = "back";

/// What the web client places a card as, at scale 1, in board units.
pub(crate) const WIDE: f64 = 220.0;
pub(crate) const HIGH: f64 = 320.0;

/// A card showing its front as the web places it; corner set by caller.
#[must_use]
pub fn made(front: &str, back: &str, color: Option<&Color>) -> Made {
    Made {
        kind: ObjectKind::Flipcard,
        at: Point::ORIGIN,
        size: Size::new(WIDE, HIGH).ok(),
        parent: None,
        fields: [
            (WIDTH, json!(WIDE)),
            (HEIGHT, json!(HIGH)),
            (SCALE, json!(1)),
            (SIDE, json!("front")),
        ]
        .into_iter()
        .chain(color.map(|color| {
            (FILL_COLOR, json!({ (COLOR): color.as_str(), (OPACITY): 1 }))
        }))
        .collect(),
        texts: vec![
            (FRONT_DOCUMENT, front.to_owned()),
            (BACK_DOCUMENT, back.to_owned()),
        ],
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// The front as the text, the side that is up and the back as words.
pub(super) fn detail(document: &Doc, card: &Object) -> Detail {
    let documents = document.get_or_insert_map(DOCUMENTS);
    let transaction = document.transact();
    let text = |field: &str| {
        card.fields
            .get(field)
            .and_then(Value::as_str)
            .map(|id| text_of(&transaction, &documents, id))
    };
    Detail {
        text: text(FRONT_DOCUMENT),
        words: [
            (SIDE, card.fields.get(SIDE).cloned()),
            (BACK, text(BACK_DOCUMENT).map(Value::String)),
        ]
        .into_iter()
        .filter_map(|(word, value)| value.map(|value| (word, value)))
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use yrs::{Doc, Map as _, Transact as _, XmlTextPrelim};

    use super::made;
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            geometry::Size,
            kinds::detail,
            palette::Color,
        },
        protocol::document::DOCUMENTS,
    };

    #[test]
    fn a_made_card_has_the_keys_of_the_web_client() -> Result<()> {
        let red: Color = "red4".parse()?;
        let plain = made("Q", "A", None);
        let filled = made("Q", "A", Some(&red));
        assert_eq!(
            (plain.kind, plain.size, plain.fields),
            (
                ObjectKind::Flipcard,
                Some(Size::new(220.0, 320.0)?),
                vec![
                    ("width", json!(220.0)),
                    ("height", json!(320.0)),
                    ("scale", json!(1)),
                    ("side", json!("front")),
                ]
            )
        );
        assert_eq!(
            filled.fields.last(),
            Some(&("fillColor", json!({"color": "red4", "opacity": 1})))
        );
        Ok(())
    }

    #[test]
    fn a_card_has_a_document_for_each_side() {
        let card = made("Q", "A", None);
        assert_eq!(
            card.texts,
            vec![
                ("frontDocumentId", "Q".to_owned()),
                ("backDocumentId", "A".to_owned())
            ]
        );
    }

    #[test]
    fn a_card_lists_its_front_as_text_and_its_side_and_back_as_words() {
        let document = Doc::new();
        let documents = document.get_or_insert_map(DOCUMENTS);
        {
            let mut transaction = document.transact_mut();
            documents.insert(&mut transaction, "f1", XmlTextPrelim::new("Q"));
            documents.insert(&mut transaction, "b1", XmlTextPrelim::new("A"));
        }
        let card = object_of(json!({
            "id": "k1", "type": "flip-card", "side": "front",
            "frontDocumentId": "f1", "backDocumentId": "b1",
            "position": {"x": 0, "y": 0}
        }));
        let shown = card.iter().map(|object| detail(&document, object));
        assert_eq!(
            shown
                .map(|found| (found.text, found.words))
                .collect::<Vec<_>>(),
            vec![(
                Some("Q".to_owned()),
                vec![("side", json!("front")), ("back", json!("A"))]
            )]
        );
    }
}
