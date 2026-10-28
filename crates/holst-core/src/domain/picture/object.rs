use serde_json::Value;

use crate::{
    domain::{
        area,
        area::scene::Scene,
        board::{Object, ObjectKind},
        geometry::Rectangle,
        palette,
        palette::TEXT_COLOR,
        picture::{
            PictureImages,
            arrow::arrow,
            shapes::shape,
            svg::{
                HorizontalAlign,
                MONO,
                PLACEHOLDER,
                Paint,
                TextAlign,
                TextBlock,
                VerticalAlign,
                escape,
                fill_attribute,
                format_number,
                paint,
            },
        },
        text_layout,
    },
    protocol::document,
};

pub(super) fn object_svg(
    scene: &Scene<'_>,
    object: &Object,
    images: &PictureImages<'_>,
) -> String {
    let id = escape(&object.id);
    let inner = match object.kind {
        ObjectKind::Arrow => scene
            .route(object)
            .map(|route| arrow(object, &route))
            .unwrap_or_default(),
        ObjectKind::Sticker
        | ObjectKind::StickerStack
        | ObjectKind::Text
        | ObjectKind::Shape
        | ObjectKind::Frame
        | ObjectKind::Document
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Stamp
        | ObjectKind::Group
        | ObjectKind::Table
        | ObjectKind::TableCell
        | ObjectKind::Code
        | ObjectKind::Drawing
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => scene
            .rectangle_of(object)
            .map(|rectangle| placed(object, rectangle, images))
            .unwrap_or_default(),
    };
    format!("<g id=\"o-{id}\">{inner}</g>")
}

fn placed(object: &Object, rectangle: Rectangle, images: &PictureImages<'_>) -> String {
    let scale = match object.kind {
        ObjectKind::Sticker | ObjectKind::StickerStack | ObjectKind::Text => {
            area::scene::text_scale(object)
        }
        ObjectKind::Shape
        | ObjectKind::Frame
        | ObjectKind::Document
        | ObjectKind::Arrow
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Stamp
        | ObjectKind::Group
        | ObjectKind::Table
        | ObjectKind::TableCell
        | ObjectKind::Code
        | ObjectKind::Drawing
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => 1.0,
    };
    let local =
        Rectangle::new(0.0, 0.0, rectangle.width / scale, rectangle.height / scale);
    let rotation = area::scene::rotation(object);
    let turn = if rotation == 0.0 {
        String::new()
    } else {
        format!(" rotate({})", format_number(rotation))
    };
    let zoom = if (scale - 1.0).abs() > 1e-9 {
        format!(" scale({})", format_number(scale))
    } else {
        String::new()
    };
    let content = match &object.kind {
        ObjectKind::Sticker => sticker(object, local),
        ObjectKind::StickerStack => stack(object, local),
        ObjectKind::Text => free_text(object, local),
        ObjectKind::Shape => shape(object, local),
        ObjectKind::Frame => frame(object, local),
        ObjectKind::Document => card(object, local),
        ObjectKind::Arrow | ObjectKind::Group => String::new(),
        ObjectKind::Image => image(object, local, images),
        ObjectKind::Table => table_outline(local),
        ObjectKind::TableCell => table_cell(object, local),
        ObjectKind::Code => code_block(object, local),
        ObjectKind::Drawing => drawing(&object.fields),
        ObjectKind::Stamp => stamp(object, local, images),
        ObjectKind::File
        | ObjectKind::Dice
        | ObjectKind::Wheel
        | ObjectKind::Flipcard
        | ObjectKind::Task
        | ObjectKind::Link
        | ObjectKind::Other(_) => kind_card(object, local),
    };
    format!(
        "<g transform=\"translate({} {}){turn}{zoom}\">{content}</g>",
        format_number(rectangle.x),
        format_number(rectangle.y)
    )
}

fn sticker(object: &Object, local: Rectangle) -> String {
    let fill = paint(&object.fields, document::FILL_COLOR)
        .map_or_else(|| "#fcf3af".to_owned(), |paint| paint.color);
    let inner = Rectangle::new(
        16.0,
        16.0,
        local.width - 32.0,
        (local.height - 32.0).max(24.0),
    );
    let forced = area::scene::number(&object.fields, document::FONT_SIZE);
    let size =
        text_layout::sticker_font_size(&object.text, inner.width, inner.height, forced);
    let lines = text_layout::wrap(&object.text, inner.width, size).lines;
    let text = TextBlock {
        lines: &lines,
        rectangle: inner,
        font_size: size,
        line_height: size * text_layout::STICKER_LINE_HEIGHT,
        color: palette::text_color_on(&fill),
        align: TextAlign::of(&object.fields),
        weight: 400,
    };
    format!(
        "<rect width=\"{}\" height=\"{}\" fill=\"{fill}\" filter=\"url(#shadow)\"/>{}",
        format_number(local.width),
        format_number(local.height),
        text.svg()
    )
}

fn stack(object: &Object, local: Rectangle) -> String {
    let fill = paint(&object.fields, document::FILL_COLOR)
        .map_or_else(|| "#fcf3af".to_owned(), |paint| paint.color);
    let title = object
        .fields
        .get(document::TITLE)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let lines = [title];
    let heading = TextBlock {
        lines: &lines,
        rectangle: Rectangle::new(20.0, 20.0, local.width - 40.0, 26.0),
        font_size: 20.0,
        line_height: 26.0,
        color: "#1d2939",
        align: TextAlign::TOP_LEFT,
        weight: 600,
    };
    format!(
        "<rect width=\"{w}\" height=\"{h}\" rx=\"12\" fill=\"#ffffff\" stroke=\"#99999966\"/>\
         <rect x=\"20\" y=\"60\" width=\"{iw}\" height=\"{ih}\" fill=\"{fill}\"/>{}",
        heading.svg(),
        w = format_number(local.width),
        h = format_number(local.height),
        iw = format_number((local.width - 40.0).max(0.0)),
        ih = format_number((local.height - 80.0).max(0.0)),
    )
}

fn free_text(object: &Object, local: Rectangle) -> String {
    let lines = text_layout::wrap(
        &object.text,
        area::scene::free_text_wrap(object),
        area::scene::FREE_TEXT_FONT_SIZE,
    )
    .lines;
    let fill = paint(&object.fields, document::FILL_COLOR);
    let color = paint(&object.fields, document::TEXT_COLOR)
        .map_or_else(|| TEXT_COLOR.to_owned(), |paint| paint.color);
    let text = TextBlock {
        lines: &lines,
        rectangle: local,
        font_size: area::scene::FREE_TEXT_FONT_SIZE,
        line_height: area::scene::line_height(area::scene::FREE_TEXT_FONT_SIZE),
        color: &color,
        align: TextAlign {
            vertical: VerticalAlign::Top,
            ..TextAlign::of(&object.fields)
        },
        weight: 400,
    };
    let back = fill
        .map(|fill| {
            format!(
                "<rect width=\"{}\" height=\"{}\" {}/>",
                format_number(local.width),
                format_number(local.height),
                fill_attribute(Some(&fill))
            )
        })
        .unwrap_or_default();
    let left = TextAlign::of(&object.fields).horizontal;
    let text = match (left, object.fields.get(document::HORIZONTAL_ALIGN)) {
        (_, None) => TextBlock {
            align: TextAlign::TOP_LEFT,
            ..text
        },
        _ => text,
    };
    format!("{back}{}", text.svg())
}

/// The colors a frame is drawn with: its fill, border and label.
pub(super) struct FrameColors {
    pub fill: Option<Paint>,
    pub border: String,
    pub title: String,
    pub title_border: String,
    pub title_text: String,
}

pub(super) fn frame_colors(fields: &Value) -> FrameColors {
    let token = fields
        .get(document::FILL_COLOR)
        .and_then(|fill| fill.get(document::COLOR))
        .or_else(|| fields.get(document::COLOR_TOKEN))
        .and_then(Value::as_str);
    let fill = paint(fields, document::FILL_COLOR).or_else(|| {
        fields
            .get(document::COLOR_TOKEN)
            .and_then(palette::of_value)
            .map(|color| Paint {
                color,
                opacity: 1.0,
            })
    });
    let tone = |name: &str, shade: u8| {
        palette::hex(&format!("{name}{}", shade.min(12)))
            .unwrap_or(TEXT_COLOR)
            .to_owned()
    };
    let gray = |shade| tone("gray", shade);
    let colours = |border, label, label_border, label_text| FrameColors {
        fill: fill.clone(),
        border,
        title: label,
        title_border: label_border,
        title_text: label_text,
    };
    match (token.and_then(palette::split), &fill) {
        (_, None) => colours(gray(6), gray(3), gray(6), gray(11)),
        (Some(("white", _)), _) => colours(gray(7), gray(3), gray(7), gray(12)),
        (Some(("gray", 4)), _) => colours(gray(8), gray(5), gray(8), gray(12)),
        (Some((name, shade @ 2..=4)), _) => colours(
            tone(name, shade + 3),
            tone(name, shade + 1),
            tone(name, shade + 4),
            tone(name, 12),
        ),
        (Some((name, 5)), _) => colours(
            tone(name, 9),
            tone(name, 9),
            tone(name, 10),
            "#ffffff".to_owned(),
        ),
        (_, Some(Paint { color, .. })) => colours(
            palette::scaled(color, 0.8),
            palette::scaled(color, 0.9),
            palette::scaled(color, 0.8),
            palette::text_color_on(&palette::scaled(color, 0.9)).to_owned(),
        ),
    }
}

fn frame(object: &Object, local: Rectangle) -> String {
    let fields = &object.fields;
    let FrameColors {
        fill,
        border,
        title: label,
        title_border: label_border,
        title_text: label_text,
    } = frame_colors(fields);
    let style = fields
        .get(document::BACKGROUND_STYLE)
        .and_then(Value::as_u64);
    let fill = match style {
        Some(3) => None,
        _ => fill,
    };
    let border = match (style, &fill) {
        (Some(1), Some(Paint { color, .. })) => color.clone(),
        _ => border,
    };
    let radius = 6.0_f64.min(local.width / 8.0).min(local.height / 8.0);
    let title = truncated(&object.text, 13.0, (local.width - 12.0).max(13.0));
    let title_width = text_layout::width(&title, 13.0) + 12.0;
    let lines = [title];
    let caption = TextBlock {
        lines: &lines,
        rectangle: Rectangle::new(6.0, -30.0, title_width, 24.0),
        font_size: 13.0,
        line_height: 24.0,
        color: &label_text,
        align: TextAlign {
            horizontal: HorizontalAlign::Left,
            vertical: VerticalAlign::Middle,
        },
        weight: 500,
    };
    let pill = Some(&object.text)
        .filter(|text| !text.is_empty())
        .map(|_| {
            format!(
                "<rect y=\"-30\" width=\"{}\" height=\"24\" rx=\"5\" fill=\"{label}\" \
                 stroke=\"{label_border}\" stroke-width=\"1.1\"/>{}",
                format_number(title_width),
                caption.svg()
            )
        })
        .unwrap_or_default();
    format!(
        "<rect x=\"0.55\" y=\"0.55\" width=\"{}\" height=\"{}\" rx=\"{}\" {} stroke=\"{border}\" \
         stroke-width=\"1.1\"/>{pill}",
        format_number((local.width - 1.1).max(0.0)),
        format_number((local.height - 1.1).max(0.0)),
        format_number(radius),
        fill_attribute(fill.as_ref())
    )
}

fn truncated(text: &str, size: f64, max: f64) -> String {
    let line = text_layout::paragraphs(text).next().unwrap_or_default();
    if text_layout::width(line, size) <= max {
        line.to_owned()
    } else {
        let room = max - text_layout::width("…", size);
        let kept: String = line
            .chars()
            .scan(0.0, |used, c| {
                *used += text_layout::advance(c) * size;
                Some((*used, c))
            })
            .take_while(|(used, _)| *used <= room)
            .map(|(_, c)| c)
            .collect();
        format!("{kept}…")
    }
}

fn card(object: &Object, local: Rectangle) -> String {
    let fill = object
        .fields
        .get(document::COLOR_TOKEN)
        .and_then(palette::of_value)
        .unwrap_or_else(|| "#ffffff".to_owned());
    let style = CardStyle {
        fill: &fill,
        border: "#00000029",
        radius: 10.0,
        font_size: 16.0,
    };
    boxed(object, local, style)
}

/// How a box with text looks: fill, border, corner radius and font size.
#[derive(Debug, Clone, Copy)]
struct CardStyle<'a> {
    fill: &'a str,
    border: &'a str,
    radius: f64,
    font_size: f64,
}

fn boxed(
    object: &Object,
    local: Rectangle,
    CardStyle {
        fill,
        border,
        radius,
        font_size: size,
    }: CardStyle<'_>,
) -> String {
    let inner = Rectangle::new(
        12.0,
        8.0,
        (local.width - 24.0).max(1.0),
        (local.height - 16.0).max(1.0),
    );
    let lines = text_layout::wrap(&object.text, inner.width, size).lines;
    let text = TextBlock {
        lines: &lines,
        rectangle: inner,
        font_size: size,
        line_height: (1.5 * size).round(),
        color: palette::text_color_on(fill),
        align: TextAlign::TOP_LEFT,
        weight: 400,
    };
    format!(
        "<rect width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{fill}\" stroke=\"{border}\" \
         stroke-width=\"1.5\"/>{}",
        format_number(local.width),
        format_number(local.height),
        format_number(radius),
        text.svg()
    )
}

fn kind_card(object: &Object, local: Rectangle) -> String {
    let label = Some(object.text.as_str())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| object.kind.as_str());
    let shown = Object {
        text: label.to_owned(),
        ..object.clone()
    };
    let style = CardStyle {
        fill: "#ffffff",
        border: "#00000029",
        radius: 10.0,
        font_size: 14.0,
    };
    boxed(&shown, local, style)
}

fn table_outline(local: Rectangle) -> String {
    format!(
        "<rect width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/>",
        format_number(local.width),
        format_number(local.height),
        palette::hex("gray7").unwrap_or(TEXT_COLOR)
    )
}

fn table_cell(object: &Object, local: Rectangle) -> String {
    let fill = paint(&object.fields, document::FILL_COLOR)
        .map_or_else(|| "#ffffff".to_owned(), |paint| paint.color);
    let border = palette::hex("gray7").unwrap_or(TEXT_COLOR);
    let inner = Rectangle::new(
        10.0,
        8.0,
        (local.width - 20.0).max(1.0),
        (local.height - 16.0).max(1.0),
    );
    let lines = text_layout::wrap(&object.text, inner.width, 14.0).lines;
    let text = TextBlock {
        lines: &lines,
        rectangle: inner,
        font_size: 14.0,
        line_height: 21.0,
        color: palette::text_color_on(&fill),
        align: TextAlign {
            vertical: VerticalAlign::Middle,
            ..TextAlign::TOP_LEFT
        },
        weight: 400,
    };
    format!(
        "<rect width=\"{}\" height=\"{}\" fill=\"{fill}\" stroke=\"{border}\" \
         stroke-width=\"1\"/>{}",
        format_number(local.width),
        format_number(local.height),
        text.svg()
    )
}

fn code_block(object: &Object, local: Rectangle) -> String {
    let lines: Vec<String> = text_layout::paragraphs(&object.text)
        .map(|line| truncated(line, 14.0 * 0.6 / 0.56, local.width - 24.0))
        .collect();
    let text = TextBlock {
        lines: &lines,
        rectangle: Rectangle::new(12.0, 12.0, local.width - 24.0, local.height - 24.0),
        font_size: 14.0,
        line_height: 21.0,
        color: TEXT_COLOR,
        align: TextAlign::TOP_LEFT,
        weight: 400,
    };
    format!(
        "<rect width=\"{}\" height=\"{}\" rx=\"6\" fill=\"#f7f6f3\"/>\
         <g font-family=\"{MONO}\" xml:space=\"preserve\">{}</g>",
        format_number(local.width),
        format_number(local.height),
        text.svg()
    )
}

fn image(object: &Object, local: Rectangle, images: &PictureImages<'_>) -> String {
    let fields = &object.fields;
    let id = escape(&object.id);
    let crop: Vec<f64> = fields
        .get(document::CROP_TRANSFORM)
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let (cx, cy, cw, ch) = match crop.as_slice() {
        [x, y, w, h, ..] if *w > 0.0 && *h > 0.0 => (*x, *y, *w, *h),
        _ => (0.0, 0.0, 1.0, 1.0),
    };
    let (w, h) = (local.width, local.height);
    let full = (w / cw, h / ch);
    let radius = match crop.get(4).copied() {
        Some(mask) if (mask - 1.0).abs() < f64::EPSILON => w.min(h) / 2.0,
        _ => area::scene::number(fields, document::BORDER_RADIUS).unwrap_or_default(),
    };
    let picture = image_address(object, images.storage)
        .map(|address| {
            format!(
                "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" \
                 xlink:href=\"{}\" clip-path=\"url(#c-{id})\"/>",
                format_number(-cx * full.0),
                format_number(-cy * full.1),
                format_number(full.0),
                format_number(full.1),
                escape(images.address(&address))
            )
        })
        .unwrap_or_default();
    format!(
        "<clipPath id=\"c-{id}\"><rect width=\"{w}\" height=\"{h}\" rx=\"{r}\"/></clipPath>\
         <rect width=\"{w}\" height=\"{h}\" rx=\"{r}\" fill=\"{PLACEHOLDER}\"/>{picture}",
        w = format_number(w),
        h = format_number(h),
        r = format_number(radius)
    )
}

/// Path data through the `x, y` pairs of `numbers`; empty when there is no
/// pair.
fn polyline(numbers: &[f64]) -> String {
    let points: Vec<String> = numbers
        .chunks_exact(2)
        .filter_map(|pair| match pair {
            [x, y] => Some(format!("{} {}", format_number(*x), format_number(*y))),
            _ => None,
        })
        .collect();
    Some(points.join(" L"))
        .filter(|path| !path.is_empty())
        .map(|path| format!("M{path}"))
        .unwrap_or_default()
}

fn drawing(fields: &Value) -> String {
    let color = fields
        .get(document::COLOR)
        .and_then(palette::of_value)
        .unwrap_or_else(|| TEXT_COLOR.to_owned());
    let alpha = area::scene::number(fields, document::ALPHA).unwrap_or(1.0);
    let width = area::scene::stroke_width(fields);
    let scale = area::scene::number(fields, document::SCALE).unwrap_or(1.0);
    // PROTOCOL: the drawing algorithm name, docs/board-protocol.md § Other types.
    let filled = matches!(
        fields.get(document::ALGORITHM).and_then(Value::as_str),
        Some("svgFill")
    );
    let d = match fields.get(document::PATH) {
        Some(Value::String(path)) => area::scene::number_list(path)
            .map_or_else(|| escape(path).into_owned(), |numbers| polyline(&numbers)),
        Some(Value::Array(values)) => {
            polyline(&values.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
        }
        _ => String::new(),
    };
    let paint = Some(()).filter(|()| filled).map_or_else(
        || {
            format!(
                "fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"round\" \
                 stroke-linejoin=\"round\"",
                format_number(width)
            )
        },
        |()| format!("fill=\"{color}\""),
    );
    format!(
        "<path transform=\"scale({})\" d=\"{d}\" {paint} opacity=\"{}\"/>",
        format_number(scale),
        format_number(alpha)
    )
}

pub(super) fn image_address(object: &Object, storage: &str) -> Option<String> {
    let url = object
        .fields
        .get(document::DATA)
        .and_then(|data| data.get(document::URL))
        .and_then(Value::as_str)
        .filter(|_| object.kind == ObjectKind::Stamp);
    let stamp = url.map(|url| {
        if url.starts_with("http") || url.starts_with("data:") {
            url.to_owned()
        } else {
            format!("{storage}stickers/{url}")
        }
    });
    let image = object
        .file
        .as_ref()
        .filter(|_| object.kind == ObjectKind::Image)
        .map(|file| format!("{storage}{}", file.name));
    image.or(stamp)
}

fn stamp(object: &Object, local: Rectangle, images: &PictureImages<'_>) -> String {
    let data = object.fields.get(document::DATA);
    let kind = data
        .and_then(|d| d.get(document::TYPE))
        .and_then(Value::as_str);
    let text = data
        .and_then(|d| d.get(document::TEXT))
        .and_then(Value::as_str);
    // PROTOCOL: the stamp type name, docs/board-protocol.md § Other types.
    match (kind, text, image_address(object, images.storage)) {
        (Some("textStamp"), Some(text), _) => format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"{}\" text-anchor=\"middle\">{}</text>",
            format_number(local.width / 2.0),
            format_number(local.height * 0.75),
            format_number(local.height * 0.7),
            escape(text)
        ),
        (_, _, Some(address)) => format!(
            "<image width=\"{}\" height=\"{}\" xlink:href=\"{}\"/>",
            format_number(local.width),
            format_number(local.height),
            escape(images.address(&address))
        ),
        _ => format!(
            "<circle cx=\"{r}\" cy=\"{r}\" r=\"{r}\" fill=\"{PLACEHOLDER}\"/>",
            r = format_number(local.width.min(local.height) / 2.0)
        ),
    }
}

/// Whether `svg` is a well-formed XML document, as resvg reads it.
#[cfg(all(test, feature = "png"))]
pub(crate) fn is_xml(svg: &str) -> bool {
    resvg::usvg::roxmltree::Document::parse(svg).is_ok()
}

/// Without resvg: at least no character XML forbids.
#[cfg(all(test, not(feature = "png")))]
pub(crate) fn is_xml(svg: &str) -> bool {
    svg.chars().all(crate::domain::picture::svg::is_xml_char)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::{Map, Value, json};
    use strum::IntoEnumIterator as _;

    use crate::domain::{
        area::{Area, scene::Scene},
        board::{ObjectKind, object_of},
        geometry::Rectangle,
        picture::{PictureImages, PixelsPerUnit, draw_rectangle, object::is_xml},
    };

    /// The stored fields of one object of `kind` that the drawing needs.
    fn stored(index: usize, kind: &ObjectKind) -> Value {
        let at = index * 300;
        let name = if matches!(kind, ObjectKind::Other(_)) {
            "mystery"
        } else {
            kind.as_str()
        };
        let extra = match kind {
            ObjectKind::Arrow => vec![
                ("start", json!({"point": {"x": at, "y": 0}})),
                ("end", json!({"point": {"x": at + 100, "y": 100}})),
            ],
            ObjectKind::Drawing => vec![("path", json!("0,0,50,50"))],
            ObjectKind::Table => vec![
                ("id", json!("table")),
                ("column-c", json!({"index": 0, "width": 100})),
                ("row-r", json!({"index": 0, "height": 50})),
            ],
            ObjectKind::TableCell => vec![
                ("parentId", json!("table")),
                ("columnId", json!("c")),
                ("rowId", json!("r")),
            ],
            ObjectKind::Sticker
            | ObjectKind::StickerStack
            | ObjectKind::Text
            | ObjectKind::Shape
            | ObjectKind::Document
            | ObjectKind::Frame
            | ObjectKind::Image
            | ObjectKind::File
            | ObjectKind::Stamp
            | ObjectKind::Group
            | ObjectKind::Code
            | ObjectKind::Dice
            | ObjectKind::Wheel
            | ObjectKind::Flipcard
            | ObjectKind::Task
            | ObjectKind::Link
            | ObjectKind::Other(_) => Vec::new(),
        };
        let fields: Map<String, Value> = [
            ("id", json!(format!("k{index}"))),
            ("type", json!(name)),
            ("position", json!({"x": at, "y": 0})),
            ("width", json!(200)),
            ("height", json!(150)),
        ]
        .into_iter()
        .chain(extra)
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        Value::Object(fields)
    }

    #[test]
    fn a_board_of_every_kind_is_drawn_one_group_per_object() {
        let objects: Vec<_> = ObjectKind::iter()
            .enumerate()
            .filter_map(|(index, kind)| object_of(stored(index, &kind)))
            .collect();
        assert_eq!(
            objects.len(),
            ObjectKind::iter().count(),
            "an object is lost"
        );
        let inlined = HashMap::new();
        let images = PictureImages {
            storage: "/",
            replacements: &inlined,
        };
        let scene = Scene::new(&objects);
        let area = scene
            .area_rectangle(&Area::Board, 40.0)
            .unwrap_or(Rectangle::new(0.0, 0.0, 1.0, 1.0));
        let picture = draw_rectangle(&scene, area, PixelsPerUnit::ONE, &images);
        assert!(is_xml(&picture.svg), "{}", picture.svg);
        // A group has no box of its own: its members are drawn, it is not.
        let drawn: Vec<_> = objects
            .iter()
            .filter(|object| object.kind != ObjectKind::Group)
            .collect();
        assert_eq!(picture.object_count, drawn.len());
        assert_eq!(picture.svg.matches("<g id=\"o-").count(), drawn.len());
        let missing: Vec<_> = drawn
            .iter()
            .filter(|object| {
                !picture.svg.contains(&format!("<g id=\"o-{}\">", object.id))
            })
            .map(|object| object.kind.as_str())
            .collect();
        assert!(missing.is_empty(), "not drawn: {missing:?}");
    }
}
