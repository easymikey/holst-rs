//! A spinner wheel: the choices it spins among and the one it stopped at.

use serde_json::{Value, json};

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
    },
    protocol::document::SCALE,
};

/// The fewest choices the web client lets a wheel spin among.
pub const MIN_CHOICES: usize = 2;

/// The most choices the web client lets a wheel hold.
pub const MAX_CHOICES: usize = 20;

/// The longest label of a choice, in characters.
pub const MAX_LABEL: usize = 40;

const ITEMS: &str = "items";
const SELECTED: &str = "selectedItemId";

/// What the web client draws a wheel as at scale 1, in board units.
const DRAWN: f64 = 360.0;
const DEFAULT_SCALE: f64 = 1.5;

/// Wheel of `choices`, none selected, as the web places it; corner set by
/// caller.
#[must_use]
pub fn made(choices: &[String], mut new_id: impl FnMut() -> String) -> Made {
    let items: Vec<Value> = choices
        .iter()
        .map(|label| json!({ "id": new_id(), "label": label }))
        .collect();
    Made {
        kind: ObjectKind::Wheel,
        at: Point::ORIGIN,
        size: Size::new(DRAWN * DEFAULT_SCALE, DRAWN * DEFAULT_SCALE).ok(),
        parent: None,
        fields: vec![
            (SCALE, json!(DEFAULT_SCALE)),
            ("mode", json!("custom")),
            (ITEMS, Value::Array(items)),
            ("userIds", json!([])),
            ("removeItemOnceChosen", json!(false)),
            ("excludedItemIds", json!([])),
            (SELECTED, Value::Null),
            ("wheelRotation", json!(0)),
            ("spinTime", json!(0)),
        ],
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// The `choices` as labels, and the `selected` label, `none` before a spin.
pub(super) fn detail(wheel: &Object) -> Detail {
    let items = wheel
        .fields
        .get(ITEMS)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let chosen = wheel.fields.get(SELECTED).and_then(Value::as_str);
    let label = |item: &Value| item.get("label").cloned();
    Detail {
        text: None,
        words: vec![
            (
                "choices",
                Value::Array(items.iter().filter_map(label).collect()),
            ),
            (
                "selected",
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == chosen)
                    .filter(|_| chosen.is_some())
                    .and_then(label)
                    .unwrap_or_else(|| json!("none")),
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use yrs::Doc;

    use super::made;
    use crate::domain::{
        board::{ObjectKind, object_of},
        kinds::detail,
    };

    fn wheel_with(fields: &[(&'static str, Value)]) -> Vec<String> {
        let stored = fields.iter().cloned().chain([
            ("id", json!("w1")),
            ("type", json!("spinner-wheel")),
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
    fn a_made_wheel_has_the_keys_and_defaults_of_the_web_client() {
        let choices = ["Pizza".to_owned(), "Sushi".to_owned()];
        let drawn = std::cell::Cell::new(0_u32);
        let wheel = made(&choices, || {
            drawn.set(drawn.get() + 1);
            format!("c{}", drawn.get())
        });
        assert_eq!(
            (
                wheel.kind,
                wheel.fields,
                wheel.size.map(|size| (size.width(), size.height())),
            ),
            (
                ObjectKind::Wheel,
                vec![
                    ("scale", json!(1.5)),
                    ("mode", json!("custom")),
                    (
                        "items",
                        json!([
                            {"id": "c1", "label": "Pizza"},
                            {"id": "c2", "label": "Sushi"},
                        ])
                    ),
                    ("userIds", json!([])),
                    ("removeItemOnceChosen", json!(false)),
                    ("excludedItemIds", json!([])),
                    ("selectedItemId", Value::Null),
                    ("wheelRotation", json!(0)),
                    ("spinTime", json!(0)),
                ],
                Some((540.0, 540.0)),
            )
        );
    }

    #[test]
    fn a_wheel_lists_its_labels_and_none_selected() {
        let items =
            json!([{"id": "a", "label": "Pizza"}, {"id": "b", "label": "Sushi"}]);
        let words = wheel_with(&[("items", items), ("selectedItemId", Value::Null)]);
        assert_eq!(
            words,
            [r#"choices=["Pizza","Sushi"]"#, r#"selected="none""#]
        );
    }

    #[test]
    fn a_wheel_lists_the_label_it_stopped_at() {
        let items =
            json!([{"id": "a", "label": "Pizza"}, {"id": "b", "label": "Sushi"}]);
        let words = wheel_with(&[("items", items), ("selectedItemId", json!("b"))]);
        assert_eq!(
            words,
            [r#"choices=["Pizza","Sushi"]"#, r#"selected="Sushi""#]
        );
    }
}
