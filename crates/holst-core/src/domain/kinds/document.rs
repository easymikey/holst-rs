//! A document: a card of rich text on a light color, stored as a `card`.

use serde_json::json;

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
        palette::Color,
    },
    protocol::document::{COLOR_TOKEN, DOCUMENT_ID, FIXED_SIZE, HEIGHT, WIDTH},
};

const DEFAULT_COLOR: &str = "white1";

/// The colors the card's toolbar offers.
pub const COLORS: [&str; 8] = [
    DEFAULT_COLOR,
    "gray3",
    "red3",
    "orange3",
    "yellow3",
    "green3",
    "blue3",
    "violet3",
];

/// A new document of `text` on `color`, as the web client places it, growing
/// with its text; the corner is the caller's to set.
#[must_use]
pub fn made(text: &str, color: Option<&Color>) -> Made {
    Made {
        kind: ObjectKind::Document,
        at: Point::ORIGIN,
        size: Some(Size::CARD),
        parent: None,
        fields: vec![
            (WIDTH, json!(Size::CARD.width())),
            (HEIGHT, json!(Size::CARD.height())),
            (FIXED_SIZE, json!(false)),
            (
                COLOR_TOKEN,
                json!(color.map_or(DEFAULT_COLOR, Color::as_str)),
            ),
        ],
        texts: vec![(DOCUMENT_ID, text.to_owned())],
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// The color token of a document, when it is stored.
pub(super) fn detail(card: &Object) -> Detail {
    Detail {
        text: None,
        words: card
            .fields
            .get(COLOR_TOKEN)
            .map(|token| ("color", token.clone()))
            .into_iter()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use yrs::Doc;

    use super::{COLORS, made};
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            geometry::Size,
            kinds::detail,
            palette::{Color, hex},
        },
    };

    #[test]
    fn a_made_document_has_the_keys_size_and_token_of_the_web_client() -> Result<()> {
        let blue: Color = "blue3".parse()?;
        let plain = made("Spec", None);
        let filled = made("Spec", Some(&blue));
        assert_eq!(
            (plain.kind, plain.size, plain.fields, plain.texts),
            (
                ObjectKind::Document,
                Some(Size::new(320.0, 64.0)?),
                vec![
                    ("width", json!(320.0)),
                    ("height", json!(64.0)),
                    ("fixedSize", json!(false)),
                    ("colorToken", json!("white1")),
                ],
                vec![("documentId", "Spec".to_owned())]
            )
        );
        assert_eq!(filled.fields.last(), Some(&("colorToken", json!("blue3"))));
        Ok(())
    }

    #[test]
    fn every_offered_color_is_a_palette_token() {
        assert!(COLORS.iter().all(|token| hex(token).is_some()));
    }

    #[test]
    fn a_document_lists_its_color_token() {
        let card = object_of(json!({
            "id": "k1", "type": "card", "colorToken": "blue3",
            "position": {"x": 0, "y": 0}
        }));
        let words = card
            .iter()
            .map(|object| detail(&Doc::new(), object).words)
            .collect::<Vec<_>>();
        assert_eq!(words, vec![vec![("color", json!("blue3"))]]);
    }
}
