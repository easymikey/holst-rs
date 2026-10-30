//! A sticker stack: a pile of stickers under a title.

use serde_json::json;

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
        palette::Color,
    },
    protocol::document::{
        COLOR,
        FILL_COLOR,
        HEIGHT,
        OPACITY,
        TEXT_SCALE,
        TITLE,
        WIDTH,
    },
};

/// The widths the web client offers a stack.
pub const WIDTHS: [f64; 2] = [192.0, 384.0];

/// The height of a stack at either width.
pub const HEIGHT_OF: f64 = 192.0;

/// A new stack of `size` titled `title`, as the web client places it, filled
/// with `color`; the corner is the caller's to set.
#[must_use]
pub fn made(title: Option<&str>, size: Size, color: Option<&Color>) -> Made {
    Made {
        kind: ObjectKind::StickerStack,
        at: Point::ORIGIN,
        size: Some(size),
        parent: None,
        fields: [
            (WIDTH, json!(size.width())),
            (HEIGHT, json!(size.height())),
            (TEXT_SCALE, json!(1)),
        ]
        .into_iter()
        .chain(title.map(|title| (TITLE, json!(title))))
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

/// The title of a stack, shown as its text.
pub(super) fn detail(stack: &Object) -> Detail {
    Detail {
        text: stack
            .fields
            .get(TITLE)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        words: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use yrs::Doc;

    use super::made;
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            geometry::Size,
            kinds::detail,
            palette::Color,
        },
    };

    #[test]
    fn a_made_stack_has_the_keys_of_the_web_client() -> Result<()> {
        let red: Color = "red4".parse()?;
        let plain = made(Some("Ideas"), Size::new(192.0, 192.0)?, None);
        let wide = made(None, Size::new(384.0, 192.0)?, Some(&red));
        assert_eq!(
            (plain.kind, plain.fields),
            (
                ObjectKind::StickerStack,
                vec![
                    ("width", json!(192.0)),
                    ("height", json!(192.0)),
                    ("textScale", json!(1)),
                    ("title", json!("Ideas")),
                ]
            )
        );
        assert_eq!(
            wide.fields,
            vec![
                ("width", json!(384.0)),
                ("height", json!(192.0)),
                ("textScale", json!(1)),
                ("fillColor", json!({"color": "red4", "opacity": 1})),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_stack_lists_its_title_as_its_text() {
        let stack = object_of(json!({
            "id": "k1", "type": "sticker-stack", "title": "Ideas",
            "position": {"x": 0, "y": 0}
        }));
        let text = stack.iter().map(|object| detail(&Doc::new(), object).text);
        assert_eq!(
            text.collect::<Vec<Option<String>>>(),
            vec![Some("Ideas".to_owned())]
        );
    }
}
