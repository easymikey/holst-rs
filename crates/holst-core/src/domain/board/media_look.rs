//! What a listing shows of an image, a file or a drawing, from stored fields.

use serde::Serialize;
use serde_json::Value;

use crate::{
    domain::{
        board::ObjectKind,
        change::media::{CropMask, FileDisplay, FlipState},
    },
    protocol::document::{
        ALPHA,
        BORDER_RADIUS,
        COLOR,
        CROP_TRANSFORM,
        STROKE_COLOR,
        STROKE_STYLE,
        STROKE_WIDTH,
    },
};

/// The looks wave 4 writes to media, in the words the flags take.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MediaLook {
    /// An image's mask, or `custom` for a stored crop without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<&'static str>,
    /// The axes an image is mirrored on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// How a file is shown: `stamp` or `content`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_page: Option<u64>,
    /// The pages the page service counted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub details_hidden: bool,
    /// Whether the file is the copy of one page.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub extracted: bool,
    /// A drawing's stroke colour as `#rrggbb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Whether a drawing is see-through.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub highlighter: bool,
}

type Reader = fn(&Value) -> MediaLook;

impl MediaLook {
    /// The look the stored fields hold; none when they hold none.
    #[must_use]
    pub fn of(kind: &ObjectKind, fields: &Value) -> Option<Self> {
        let readers: [(ObjectKind, Reader); 3] = [
            (ObjectKind::Image, Self::image),
            (ObjectKind::File, Self::file),
            (ObjectKind::Drawing, Self::drawing),
        ];
        readers
            .into_iter()
            .find(|(own, _)| own == kind)
            .map(|(_, read)| read(fields))
            .filter(|look| *look != Self::default())
    }

    fn image(fields: &Value) -> Self {
        let text = |name: &str| fields.get(name).and_then(Value::as_str);
        Self {
            crop: crop(fields),
            flip: Some(FlipState::of_stored(fields))
                .filter(|state| *state != FlipState::None)
                .map(<&str>::from),
            stroke_color: fields
                .get(STROKE_COLOR)
                .and_then(|stroke| stroke.get(COLOR))
                .and_then(Value::as_str)
                .map(str::to_owned),
            stroke_width: fields.get(STROKE_WIDTH).and_then(Value::as_f64),
            stroke_style: text(STROKE_STYLE).map(str::to_owned),
            radius: fields.get(BORDER_RADIUS).and_then(Value::as_f64),
            ..Self::default()
        }
    }

    fn file(fields: &Value) -> Self {
        let flag = |name: &str| fields.get(name).and_then(Value::as_bool) == Some(true);
        Self {
            display: fields
                .get("displayType")
                .and_then(Value::as_str)
                .and_then(FileDisplay::of_stored)
                .map(<&str>::from),
            start_page: fields.get("pinnedPage").and_then(Value::as_u64),
            pages: fields
                .get("pagesInfo")
                .and_then(|info| info.get("count"))
                .and_then(Value::as_u64),
            details_hidden: flag("hideDetails"),
            extracted: flag("extracted"),
            ..Self::default()
        }
    }

    fn drawing(fields: &Value) -> Self {
        Self {
            color: fields
                .get(COLOR)
                .and_then(Value::as_u64)
                .map(|rgb| format!("#{rgb:06x}")),
            stroke_width: fields.get(STROKE_WIDTH).and_then(Value::as_f64),
            highlighter: fields
                .get(ALPHA)
                .and_then(Value::as_f64)
                .is_some_and(|alpha| alpha < 1.0),
            ..Self::default()
        }
    }
}

/// The mask of `cropTransform`; `custom` for a cut that has none, nothing for
/// the whole picture.
fn crop(fields: &Value) -> Option<&'static str> {
    let stored = fields.get(CROP_TRANSFORM)?.as_array()?;
    let whole = stored
        .iter()
        .take(4)
        .map(Value::as_f64)
        .eq([0.0, 0.0, 1.0, 1.0].map(Some));
    match stored.get(4).and_then(Value::as_u64) {
        Some(number) => {
            Some(CropMask::of_stored(number).map_or("custom", <&str>::from))
        }
        None if whole => None,
        None => Some("custom"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn look(kind: &ObjectKind, fields: &Value) -> Value {
        json!(MediaLook::of(kind, fields))
    }

    #[test]
    fn an_image_reads_its_mask_flip_and_radius() {
        let fields = json!({
            "cropTransform": [0, 0, 1, 1, 1],
            "flip": {"x": -1, "y": 1},
            "borderRadius": 20,
        });
        assert_eq!(
            look(&ObjectKind::Image, &fields),
            json!({"crop": "circle", "flip": "horizontal", "radius": 20.0}),
        );
    }

    #[test]
    fn an_image_reads_its_outline() {
        let fields = json!({
            "strokeColor": {"color": "#ff0000"},
            "strokeWidth": 4,
            "strokeStyle": "dashed",
        });
        assert_eq!(
            look(&ObjectKind::Image, &fields),
            json!({
                "stroke_color": "#ff0000",
                "stroke_width": 4.0,
                "stroke_style": "dashed",
            }),
        );
    }

    #[test]
    fn a_crop_without_a_mask_is_custom_unless_it_is_the_whole_picture() {
        let cut = json!({"cropTransform": [0.1, 0, 0.5, 1, null]});
        let whole =
            json!({"cropTransform": [0, 0, 1, 1, null], "flip": {"x": 1, "y": 1}});
        assert_eq!(look(&ObjectKind::Image, &cut), json!({"crop": "custom"}));
        assert_eq!(look(&ObjectKind::Image, &whole), Value::Null);
    }

    #[test]
    fn every_flip_state_has_its_word() {
        let flip =
            |x: i8, y: i8| look(&ObjectKind::Image, &json!({"flip": {"x": x, "y": y}}));
        assert_eq!(flip(1, -1), json!({"flip": "vertical"}));
        assert_eq!(flip(-1, -1), json!({"flip": "both"}));
    }

    #[test]
    fn a_file_reads_its_display_page_and_pages() {
        let fields = json!({
            "displayType": "content",
            "pinnedPage": 2,
            "pagesInfo": {"count": 5},
        });
        assert_eq!(
            look(&ObjectKind::File, &fields),
            json!({"display": "content", "start_page": 2, "pages": 5}),
        );
    }

    #[test]
    fn a_file_reads_hidden_details_and_the_extracted_mark() {
        let fields = json!({"hideDetails": true, "extracted": true});
        assert_eq!(
            look(&ObjectKind::File, &fields),
            json!({"details_hidden": true, "extracted": true}),
        );
    }

    #[test]
    fn a_drawing_reads_its_colour_width_and_transparency() {
        let fields = json!({"color": 16_511_040, "alpha": 0.5, "strokeWidth": 22});
        assert_eq!(
            look(&ObjectKind::Drawing, &fields),
            json!({"color": "#fbf040", "stroke_width": 22.0, "highlighter": true}),
        );
    }

    #[test]
    fn a_pen_stroke_is_not_a_highlighter() {
        let fields = json!({"color": 1_973_790, "alpha": 1});
        assert_eq!(
            look(&ObjectKind::Drawing, &fields),
            json!({"color": "#1e1e1e"}),
        );
    }

    #[test]
    fn other_kinds_and_bare_media_have_none() {
        let fields =
            json!({"color": 1, "strokeWidth": 3, "cropTransform": [0, 0, 1, 1, 1]});
        assert_eq!(look(&ObjectKind::Sticker, &fields), Value::Null);
        assert_eq!(look(&ObjectKind::File, &json!({})), Value::Null);
    }
}
