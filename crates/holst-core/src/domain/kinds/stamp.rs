//! A text stamp: an emoji or a few words placed as a stamp.

use serde_json::{Value, json};

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
    },
    protocol::document::{DATA, HEIGHT, WIDTH},
};

const TEXT: &str = "text";
const TEXT_STAMP: &str = "textStamp";

/// What the web client places a stamp as, in board units.
const SIDE: f64 = 60.0;

/// A new text stamp as the web places it; corner set by caller.
#[must_use]
pub fn made(text: &str) -> Made {
    Made {
        kind: ObjectKind::Stamp,
        at: Point::ORIGIN,
        size: Size::new(SIDE, SIDE).ok(),
        parent: None,
        fields: vec![
            (WIDTH, json!(SIDE)),
            (HEIGHT, json!(SIDE)),
            (DATA, json!({ "type": TEXT_STAMP, (TEXT): text })),
        ],
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// Whether the stamp stores a text, as a text stamp does.
pub(crate) fn is_text(stamp: &Value) -> bool {
    stamp.pointer("/data/type").and_then(Value::as_str) == Some(TEXT_STAMP)
}

/// The `data` of the stamp with `text`, its other keys kept.
pub(crate) fn retexted(stamp: &Value, text: &str) -> Value {
    let kept = stamp
        .get(DATA)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Value::Object(
        kept.into_iter()
            .chain([(TEXT.to_owned(), json!(text))])
            .collect(),
    )
}

/// The text of a text stamp; an image stamp has none.
pub(super) fn detail(stamp: &Object) -> Detail {
    Detail {
        text: stamp
            .fields
            .get(DATA)
            .and_then(|data| data.get(TEXT))
            .and_then(Value::as_str)
            .map(str::to_owned),
        words: Vec::new(),
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

    fn text_of_stamp(data: &Value) -> Vec<Option<String>> {
        object_of(json!({
            "id": "s1",
            "type": "stamp",
            "position": {"x": 0, "y": 0},
            "data": data,
        }))
        .iter()
        .map(|object| detail(&Doc::new(), object).text)
        .collect()
    }

    #[test]
    fn a_made_stamp_has_the_keys_and_size_of_the_web_client() {
        let stamp = made("\u{1F44D}");
        assert_eq!(
            (
                stamp.kind,
                stamp.fields,
                stamp.size.map(|size| (size.width(), size.height())),
            ),
            (
                ObjectKind::Stamp,
                vec![
                    ("width", json!(60.0)),
                    ("height", json!(60.0)),
                    ("data", json!({"type": "textStamp", "text": "\u{1F44D}"})),
                ],
                Some((60.0, 60.0)),
            )
        );
    }

    #[test]
    fn a_text_stamp_lists_its_text() {
        let data = json!({"type": "textStamp", "text": "\u{1F44D}"});
        assert_eq!(text_of_stamp(&data), [Some("\u{1F44D}".to_owned())]);
    }

    #[test]
    fn an_image_stamp_lists_no_text() {
        let data = json!({"type": "imageStamp", "url": "https://example.com/a.png"});
        assert_eq!(text_of_stamp(&data), [None]);
    }
}
