//! A die: how many faces it has and the value it shows.

use serde_json::json;

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
        palette::Color,
    },
    protocol::document::{COLOR, FILL_COLOR, OPACITY, ROTATION, SCALE},
};

/// The face counts the web client offers, but for fudge and percentile.
pub const FACES: [u32; 7] = [2, 4, 6, 8, 10, 12, 20];

/// The faces of a die the web client places.
pub const DEFAULT_FACES: u32 = 6;

const FACES_FIELD: &str = "faces";
const VALUE_FIELD: &str = "value";
const SPIN_TIME: &str = "spinTime";

/// What the web client draws a die as at scale 1, in board units.
const DRAWN: f64 = 240.0;
const DEFAULT_SCALE: f64 = 0.5;

/// A new die showing 1, as the web client places it, filled with `color`; the
/// corner is the caller's to set.
#[must_use]
pub fn made(faces: u32, color: Option<&Color>) -> Made {
    Made {
        kind: ObjectKind::Dice,
        at: Point::ORIGIN,
        size: Size::new(DRAWN * DEFAULT_SCALE, DRAWN * DEFAULT_SCALE).ok(),
        parent: None,
        fields: [
            (ROTATION, json!(0)),
            (SCALE, json!(DEFAULT_SCALE)),
            (FACES_FIELD, json!(faces)),
            (VALUE_FIELD, json!(1)),
            (SPIN_TIME, json!(0)),
        ]
        .into_iter()
        .chain(color.map(|color| {
            (FILL_COLOR, json!({ (COLOR): color.as_str(), (OPACITY): 1 }))
        }))
        .collect(),
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// The `faces` and the `value` of a die, each when it is stored.
pub(super) fn detail(die: &Object) -> Detail {
    Detail {
        text: None,
        words: [FACES_FIELD, VALUE_FIELD]
            .into_iter()
            .filter_map(|word| {
                die.fields.get(word).map(|stored| (word, stored.clone()))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::Doc;

    use super::{DEFAULT_FACES, FACES, made};
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            kinds::detail,
            palette::Color,
        },
    };

    fn die_with(fields: &[(&'static str, Value)]) -> Vec<String> {
        let stored = fields.iter().cloned().chain([
            ("id", json!("d1")),
            ("type", json!("dice")),
            ("position", json!({"x": 0, "y": 0})),
        ]);
        object_of(Value::Object(
            stored
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        ))
        .iter()
        .flat_map(|object| detail(&Doc::new(), object).words)
        .map(|(word, value)| format!("{word}={value}"))
        .collect()
    }

    #[test]
    fn a_made_die_has_the_keys_and_defaults_of_the_web_client() -> Result<()> {
        let red: Color = "red4".parse()?;
        let plain = made(DEFAULT_FACES, None);
        let filled = made(20, Some(&red));
        assert_eq!(
            (
                plain.kind,
                plain.fields,
                filled
                    .fields
                    .iter()
                    .map(|(name, value)| (*name, value.clone()))
                    .next_back(),
                plain.size.map(|size| (size.width(), size.height())),
            ),
            (
                ObjectKind::Dice,
                vec![
                    ("rotation", json!(0)),
                    ("scale", json!(0.5)),
                    ("faces", json!(6)),
                    ("value", json!(1)),
                    ("spinTime", json!(0)),
                ],
                Some(("fillColor", json!({"color": "red4", "opacity": 1}))),
                Some((120.0, 120.0)),
            )
        );
        assert_eq!(FACES.first().zip(FACES.last()), Some((&2, &20)));
        Ok(())
    }

    #[test]
    fn a_die_lists_its_faces_and_its_value() {
        let words = die_with(&[("faces", json!(20)), ("value", json!(7))]);
        assert_eq!(words, ["faces=20", "value=7"]);
    }

    #[test]
    fn a_die_without_the_fields_lists_no_words() {
        assert_eq!(die_with(&[]), Vec::<String>::new());
    }
}
