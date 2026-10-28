//! The outlines of shapes: polygons, curves and the shape drawn.

use itertools::Itertools as _;
use serde_json::Value;

use crate::{
    domain::{
        area,
        board::{Object, shape::ShapeKind},
        geometry::{Rectangle, Size, StoredPoint},
        palette,
        palette::TEXT_COLOR,
        picture::svg::{
            Paint,
            StrokeKind,
            StrokeStyle,
            TextAlign,
            TextBlock,
            dashes,
            fill_attribute,
            format_number,
            paint,
        },
        text_layout,
    },
    protocol::document,
};

/// The SVG attributes an outline is drawn with: a filled one and a line.
#[derive(Debug, Clone, Copy)]
pub(super) struct OutlineAttributes<'a> {
    pub filled: &'a str,
    pub unfilled: &'a str,
}

/// An outline's SVG and the box its text fits in.
pub(super) struct ShapeOutline {
    pub(super) svg: String,
    pub(super) text_rectangle: Rectangle,
}

impl ShapeOutline {
    const fn new(svg: String, text_rectangle: Rectangle) -> Self {
        Self {
            svg,
            text_rectangle,
        }
    }
}

/// A polygon through `points` (shares of the size) and the text box (shares).
fn polygon(
    points: &[StoredPoint],
    text: Rectangle,
    size: Size,
    paint: &OutlineAttributes<'_>,
) -> ShapeOutline {
    let (width, height) = (size.width(), size.height());
    let at = points
        .iter()
        .map(|spot| {
            format!(
                "{},{}",
                format_number(spot.x * width),
                format_number(spot.y * height)
            )
        })
        .join(" ");
    ShapeOutline {
        svg: format!("<polygon points=\"{at}\" {}/>", paint.filled),
        text_rectangle: Rectangle::new(
            text.x * width,
            text.y * height,
            text.width * width,
            text.height * height,
        ),
    }
}

/// A polygon shape: corners and text box as shares of the size.
struct PolygonShape {
    kind: ShapeKind,
    points: &'static [StoredPoint],
    text_rectangle: Rectangle,
}

const MIDDLE: Rectangle = Rectangle::new(0.25, 0.25, 0.5, 0.5);

const WIDE: Rectangle = Rectangle::new(0.18, 0.0, 0.64, 1.0);

const POLYGONS: [PolygonShape; 16] = [
    PolygonShape {
        kind: ShapeKind::Triangle,
        points: &[
            StoredPoint::new(0.0, 1.0),
            StoredPoint::new(0.5, 0.0),
            StoredPoint::new(1.0, 1.0),
        ],
        text_rectangle: Rectangle::new(0.25, 0.5, 0.5, 0.5),
    },
    PolygonShape {
        kind: ShapeKind::InvertedTriangle,
        points: &[
            StoredPoint::new(0.0, 0.0),
            StoredPoint::new(1.0, 0.0),
            StoredPoint::new(0.5, 1.0),
        ],
        text_rectangle: Rectangle::new(0.25, 0.0, 0.5, 0.5),
    },
    PolygonShape {
        kind: ShapeKind::Diamond,
        points: &[
            StoredPoint::new(0.0, 0.5),
            StoredPoint::new(0.5, 0.0),
            StoredPoint::new(1.0, 0.5),
            StoredPoint::new(0.5, 1.0),
        ],
        text_rectangle: MIDDLE,
    },
    PolygonShape {
        kind: ShapeKind::RightParallelogram,
        points: &[
            StoredPoint::new(0.18, 0.0),
            StoredPoint::new(1.0, 0.0),
            StoredPoint::new(0.82, 1.0),
            StoredPoint::new(0.0, 1.0),
        ],
        text_rectangle: WIDE,
    },
    PolygonShape {
        kind: ShapeKind::LeftParallelogram,
        points: &[
            StoredPoint::new(0.0, 0.0),
            StoredPoint::new(0.82, 0.0),
            StoredPoint::new(1.0, 1.0),
            StoredPoint::new(0.18, 1.0),
        ],
        text_rectangle: WIDE,
    },
    PolygonShape {
        kind: ShapeKind::Trapezoid,
        points: &[
            StoredPoint::new(0.0, 1.0),
            StoredPoint::new(0.18, 0.0),
            StoredPoint::new(0.82, 0.0),
            StoredPoint::new(1.0, 1.0),
        ],
        text_rectangle: WIDE,
    },
    PolygonShape {
        kind: ShapeKind::ManualInput,
        points: &[
            StoredPoint::new(0.0, 0.2),
            StoredPoint::new(1.0, 0.0),
            StoredPoint::new(1.0, 1.0),
            StoredPoint::new(0.0, 1.0),
        ],
        text_rectangle: Rectangle::new(0.0, 0.2, 1.0, 0.8),
    },
    PolygonShape {
        kind: ShapeKind::OffPageConnector,
        points: &[
            StoredPoint::new(0.0, 0.0),
            StoredPoint::new(1.0, 0.0),
            StoredPoint::new(1.0, 0.75),
            StoredPoint::new(0.5, 1.0),
            StoredPoint::new(0.0, 0.75),
        ],
        text_rectangle: Rectangle::new(0.0, 0.0, 1.0, 0.75),
    },
    PolygonShape {
        kind: ShapeKind::Pentagon,
        points: &[
            StoredPoint::new(0.5, 0.0),
            StoredPoint::new(1.0, 0.382),
            StoredPoint::new(0.809, 1.0),
            StoredPoint::new(0.191, 1.0),
            StoredPoint::new(0.0, 0.382),
        ],
        text_rectangle: Rectangle::new(0.25, 0.191, 0.5, 0.809),
    },
    PolygonShape {
        kind: ShapeKind::Hexagon,
        points: &[
            StoredPoint::new(0.25, 0.0),
            StoredPoint::new(0.75, 0.0),
            StoredPoint::new(1.0, 0.5),
            StoredPoint::new(0.75, 1.0),
            StoredPoint::new(0.25, 1.0),
            StoredPoint::new(0.0, 0.5),
        ],
        text_rectangle: Rectangle::new(0.25, 0.0, 0.5, 1.0),
    },
    PolygonShape {
        kind: ShapeKind::Octagon,
        points: &[
            StoredPoint::new(0.293, 0.0),
            StoredPoint::new(0.707, 0.0),
            StoredPoint::new(1.0, 0.293),
            StoredPoint::new(1.0, 0.707),
            StoredPoint::new(0.707, 1.0),
            StoredPoint::new(0.293, 1.0),
            StoredPoint::new(0.0, 0.707),
            StoredPoint::new(0.0, 0.293),
        ],
        text_rectangle: MIDDLE,
    },
    PolygonShape {
        kind: ShapeKind::Star,
        points: &[
            StoredPoint::new(0.5, 0.0),
            StoredPoint::new(0.35, 0.32),
            StoredPoint::new(0.0, 0.37),
            StoredPoint::new(0.26, 0.6),
            StoredPoint::new(0.21, 1.0),
            StoredPoint::new(0.5, 0.78),
            StoredPoint::new(0.79, 1.0),
            StoredPoint::new(0.74, 0.6),
            StoredPoint::new(1.0, 0.37),
            StoredPoint::new(0.65, 0.32),
        ],
        text_rectangle: Rectangle::new(0.26, 0.34, 0.48, 0.44),
    },
    PolygonShape {
        kind: ShapeKind::Cross,
        points: &[
            StoredPoint::new(0.0, 0.25),
            StoredPoint::new(0.0, 0.75),
            StoredPoint::new(0.25, 0.75),
            StoredPoint::new(0.25, 1.0),
            StoredPoint::new(0.75, 1.0),
            StoredPoint::new(0.75, 0.75),
            StoredPoint::new(1.0, 0.75),
            StoredPoint::new(1.0, 0.25),
            StoredPoint::new(0.75, 0.25),
            StoredPoint::new(0.75, 0.0),
            StoredPoint::new(0.25, 0.0),
            StoredPoint::new(0.25, 0.25),
        ],
        text_rectangle: MIDDLE,
    },
    PolygonShape {
        kind: ShapeKind::ArrowRight,
        points: &[
            StoredPoint::new(0.0, 0.22),
            StoredPoint::new(0.0, 0.78),
            StoredPoint::new(0.7, 0.78),
            StoredPoint::new(0.7, 1.0),
            StoredPoint::new(1.0, 0.5),
            StoredPoint::new(0.7, 0.0),
            StoredPoint::new(0.7, 0.22),
        ],
        text_rectangle: Rectangle::new(0.0, 0.22, 0.85, 0.56),
    },
    PolygonShape {
        kind: ShapeKind::ArrowLeft,
        points: &[
            StoredPoint::new(1.0, 0.22),
            StoredPoint::new(0.3, 0.22),
            StoredPoint::new(0.3, 0.0),
            StoredPoint::new(0.0, 0.5),
            StoredPoint::new(0.3, 1.0),
            StoredPoint::new(0.3, 0.78),
            StoredPoint::new(1.0, 0.78),
        ],
        text_rectangle: Rectangle::new(0.15, 0.22, 0.85, 0.56),
    },
    PolygonShape {
        kind: ShapeKind::ArrowLeftRight,
        points: &[
            StoredPoint::new(0.7, 0.22),
            StoredPoint::new(0.3, 0.22),
            StoredPoint::new(0.3, 0.0),
            StoredPoint::new(0.0, 0.5),
            StoredPoint::new(0.3, 1.0),
            StoredPoint::new(0.3, 0.78),
            StoredPoint::new(0.7, 0.78),
            StoredPoint::new(0.7, 1.0),
            StoredPoint::new(1.0, 0.5),
            StoredPoint::new(0.7, 0.0),
        ],
        text_rectangle: Rectangle::new(0.15, 0.22, 0.7, 0.56),
    },
];

/// The outline of `kind` in a box of `size`, and the box its text fits in.
pub(super) fn outline(
    kind: ShapeKind,
    size: Size,
    radius: f64,
    paint: &OutlineAttributes<'_>,
) -> ShapeOutline {
    POLYGONS
        .iter()
        .find(|shape| shape.kind == kind)
        .map_or_else(
            || curved_outline(kind, size, radius, paint),
            |shape| polygon(shape.points, shape.text_rectangle, size, paint),
        )
}

fn speech_bubble(width: f64, height: f64, style: &str) -> String {
    let body = 0.75 * height;
    let corner = width.min(height) / 8.0;
    let d = [
        format!(
            "M{} {} L{} {} L{} {}",
            format_number(0.25 * width),
            format_number(body),
            format_number(0.25 * width),
            format_number(height),
            format_number(0.41 * width),
            format_number(body)
        ),
        format!(
            "L{} {} Q{} {} {} {}",
            format_number(width - corner),
            format_number(body),
            format_number(width),
            format_number(body),
            format_number(width),
            format_number(body - corner)
        ),
        format!(
            "L{} {} Q{} 0 {} 0 L{} 0",
            format_number(width),
            format_number(corner),
            format_number(width),
            format_number(width - corner),
            format_number(corner)
        ),
        format!(
            "Q0 0 0 {} L0 {} Q0 {} {} {} Z",
            format_number(corner),
            format_number(body - corner),
            format_number(body),
            format_number(corner),
            format_number(body)
        ),
    ]
    .join(" ");
    format!("<path d=\"{d}\" {style}/>")
}

fn curved_outline(
    kind: ShapeKind,
    size: Size,
    radius: f64,
    paint: &OutlineAttributes<'_>,
) -> ShapeOutline {
    let (width, height) = (size.width(), size.height());
    let whole = Rectangle::new(0.0, 0.0, width, height);
    let rectangle =
        |corner: f64| rounded_rectangle(width, height, corner, paint.filled);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let inscribed = Rectangle::new(
        width * (1.0 - half) / 2.0,
        height * (1.0 - half) / 2.0,
        width * half,
        height * half,
    );
    match kind {
        ShapeKind::Square => ShapeOutline::new(rectangle(radius), whole),
        ShapeKind::RoundedRectangle => {
            ShapeOutline::new(rectangle(width.min(height) / 6.0), whole)
        }
        ShapeKind::Ellipse => ShapeOutline::new(
            format!(
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" {}/>",
                format_number(width / 2.0),
                format_number(height / 2.0),
                format_number(width / 2.0),
                format_number(height / 2.0),
                paint.filled
            ),
            inscribed,
        ),
        ShapeKind::SpeechBubble => ShapeOutline::new(
            speech_bubble(width, height, paint.filled),
            Rectangle::new(0.0, 0.0, width, 0.75 * height),
        ),
        ShapeKind::Terminator => {
            ShapeOutline::new(rectangle(width.min(height) / 2.0), whole)
        }
        ShapeKind::Process => ShapeOutline::new(
            rectangle((width.min(height) / 4.0).floor().min(8.0)),
            whole,
        ),
        ShapeKind::BpmnTask => ShapeOutline::new(rectangle(10.0), whole),
        ShapeKind::PredefinedProcess => ShapeOutline::new(
            format!(
                "{}<path d=\"M20 0V{h}M{x} 0V{h}\" {}/>",
                rounded_rectangle(width, height, 8.0, paint.filled),
                paint.unfilled,
                h = format_number(height),
                x = format_number(width - 20.0)
            ),
            Rectangle::new(20.0, 0.0, (width - 40.0).max(0.0), height),
        ),
        ShapeKind::Document => ShapeOutline::new(
            format!(
                "<path d=\"M0 {b} L0 0 L{w} 0 L{w} {h} C{c1x} {c1y} {c2x} {c2y} 0 {b} Z\" {}/>",
                paint.filled,
                b = format_number(0.8 * height),
                w = format_number(width),
                h = format_number(height),
                c1x = format_number(0.6 * width),
                c1y = format_number(0.5 * height),
                c2x = format_number(0.4 * width),
                c2y = format_number(1.3 * height)
            ),
            Rectangle::new(0.0, 0.0, width, 0.8 * height),
        ),
        ShapeKind::Database => ShapeOutline::new(
            format!(
                "<path d=\"M{w} 12 L{w} {b} A{rx} 12 0 0 1 0 {b} L0 12 Z\" {style}/>\
                 <ellipse cx=\"{rx}\" cy=\"12\" rx=\"{rx}\" ry=\"12\" {style}/>",
                style = paint.filled,
                w = format_number(width),
                b = format_number(height - 12.0),
                rx = format_number(width / 2.0)
            ),
            Rectangle::new(0.0, 24.0, width, (height - 36.0).max(0.0)),
        ),
        // Polygons are drawn before this point; the rest is a plain box.
        ShapeKind::Unlisted
        | ShapeKind::Triangle
        | ShapeKind::Diamond
        | ShapeKind::InvertedTriangle
        | ShapeKind::RightParallelogram
        | ShapeKind::LeftParallelogram
        | ShapeKind::Trapezoid
        | ShapeKind::ManualInput
        | ShapeKind::OffPageConnector
        | ShapeKind::Pentagon
        | ShapeKind::Hexagon
        | ShapeKind::Octagon
        | ShapeKind::Star
        | ShapeKind::Cross
        | ShapeKind::ArrowRight
        | ShapeKind::ArrowLeft
        | ShapeKind::ArrowLeftRight => ShapeOutline::new(
            rectangle(radius.max(6.0).min(width.min(height) / 4.0)),
            whole,
        ),
    }
}

fn rounded_rectangle(width: f64, height: f64, corner: f64, style: &str) -> String {
    format!(
        "<rect width=\"{}\" height=\"{}\" rx=\"{}\" {style}/>",
        format_number(width),
        format_number(height),
        format_number(corner)
    )
}

fn stroke_attributes(object: &Object, stroke_width: f64) -> String {
    let fields = &object.fields;
    paint(fields, document::STROKE_COLOR)
        .filter(|_| stroke_width > 0.0)
        .map(|Paint { color, opacity }| {
            let alpha = if opacity < 1.0 {
                format!(" stroke-opacity=\"{}\"", format_number(opacity))
            } else {
                String::new()
            };
            format!(
                " stroke=\"{color}\" stroke-width=\"{}\"{alpha}{} stroke-linejoin=\"round\"",
                format_number(stroke_width),
                dashes(
                    StrokeStyle::of(
                        fields.get(document::STROKE_STYLE).and_then(Value::as_str)
                    ),
                    stroke_width,
                    StrokeKind::Outline
                )
            )
        })
        .unwrap_or_default()
}

pub(super) fn shape(object: &Object, local: Rectangle) -> String {
    Size::new(local.width, local.height)
        .map_or_else(|_| String::new(), |size| drawn_shape(object, size))
}

fn drawn_shape(object: &Object, size: Size) -> String {
    let fields = &object.fields;
    let kind = ShapeKind::from_stored_name(
        fields
            .get(document::SHAPE_TYPE)
            .and_then(Value::as_str)
            .unwrap_or("square"),
    );
    let radius =
        area::scene::number(fields, document::BORDER_RADIUS).unwrap_or_default();
    let fill = paint(fields, document::FILL_COLOR);
    let stroke_width = area::scene::stroke_width(&object.fields)
        .min(size.width() / 2.0)
        .min(size.height() / 2.0);
    let stroke = stroke_attributes(object, stroke_width);
    let style = format!("{}{stroke}", fill_attribute(fill.as_ref()));
    let line = format!("fill=\"none\"{stroke}");
    let ShapeOutline {
        svg: drawn,
        text_rectangle: text_box,
    } = outline(
        kind,
        size,
        radius,
        &OutlineAttributes {
            filled: &style,
            unfilled: &line,
        },
    );
    let font = area::scene::number(fields, document::FONT_SIZE).unwrap_or(16.0);
    let pad = |side: f64| (side - 24.0).max(0.0).min(side) / side.max(1.0) * 12.0;
    let padded = || {
        Rectangle::new(
            text_box.x + pad(text_box.width),
            text_box.y + pad(text_box.height).min(12.0),
            (text_box.width - 2.0 * pad(text_box.width)).max(1.0),
            (text_box.height - 2.0 * pad(text_box.height).min(12.0)).max(1.0),
        )
    };
    // Every ellipse-like kind already has the inscribed box: no margin on top.
    let text_rect = if kind == ShapeKind::Ellipse {
        text_box
    } else {
        padded()
    };
    let lines = text_layout::wrap(&object.text, text_rect.width, font).lines;
    let color = paint(fields, document::TEXT_COLOR).map_or_else(
        || {
            fill.as_ref()
                .filter(|paint| paint.opacity > 0.0)
                .map_or(TEXT_COLOR, |paint| palette::text_color_on(&paint.color))
                .to_owned()
        },
        |paint| paint.color,
    );
    let text = TextBlock {
        lines: &lines,
        rectangle: text_rect,
        font_size: font,
        line_height: (1.5 * font).round(),
        color: &color,
        align: TextAlign::of(fields),
        weight: 400,
    };
    format!("{drawn}{}", text.svg())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use crate::domain::{
        board::{Object, object_of, shape::ShapeKind},
        geometry::Rectangle,
        picture::shapes::shape,
    };

    const NAMES: [&str; 41] = [
        "square",
        "roundedRectangle",
        "ellipse",
        "flowchart-connector",
        "bpmn-event-start",
        "bpmn-event-intermediate",
        "bpmn-event-end",
        "flowchart-or",
        "flowchart-summing-junction",
        "triangle",
        "invertedTriangle",
        "flowchart-merge",
        "diamond",
        "flowchart-decision",
        "bpmn-gateway",
        "rightParallelogram",
        "flowchart-data",
        "leftParallelogram",
        "basic-trapezoid",
        "flowchart-manual-operation",
        "flowchart-manual-input",
        "flowchart-off-page-connector",
        "basic-pentagon",
        "basic-hexagon",
        "flowchart-preparation",
        "bpmn-conversation",
        "basic-octagon",
        "basic-star",
        "basic-cross",
        "basic-arrow-right",
        "basic-arrow-left",
        "basic-arrow-left-right",
        "basic-speech-bubble",
        "flowchart-terminator",
        "flowchart-process",
        "bpmn-task",
        "bpmn-data-store",
        "flowchart-predefined-process",
        "flowchart-document",
        "flowchart-database",
        "mystery",
    ];

    fn drawn(name: &str, width: f64, height: f64) -> String {
        object_of(json!({
            "id": "s", "type": "shape", "shapeType": name, "documentId": "d",
            "position": {"x": 0, "y": 0}, "width": width, "height": height,
            "fillColor": {"color": "red5", "opacity": 1}, "strokeColor": "gray12",
            "strokeStyle": "dashed", "borderRadius": 12, "strokeWidth": 3,
        }))
        .map(|object| Object {
            text: "A fairly long label to wrap".into(),
            ..object
        })
        .map(|object| shape(&object, Rectangle::new(0.0, 0.0, width, height)))
        .unwrap_or_default()
    }

    /// Every stored name is drawn as before, except that the six ellipse-like
    /// flowchart and BPMN names now take the ellipse's text box (no 12-unit
    /// margin on top of the inscribed rectangle).
    #[test]
    fn outlines_of_every_shape() {
        let all: String = NAMES
            .iter()
            .flat_map(|name| {
                [(100.0, 100.0), (200.0, 60.0)].map(|(width, height)| {
                    format!("{name} {width}x{height}\n{}\n", drawn(name, width, height))
                })
            })
            .collect();
        insta::assert_snapshot!(all);
    }

    #[test]
    fn a_shape_without_area_draws_nothing() {
        assert_eq!(drawn("ellipse", 0.0, 50.0), "");
    }

    #[rstest]
    #[case::ellipse("ellipse", ShapeKind::Ellipse)]
    #[case::connector("flowchart-connector", ShapeKind::Ellipse)]
    #[case::event("bpmn-event-end", ShapeKind::Ellipse)]
    #[case::decision("flowchart-decision", ShapeKind::Diamond)]
    #[case::gateway("bpmn-gateway", ShapeKind::Diamond)]
    #[case::conversation("bpmn-conversation", ShapeKind::Hexagon)]
    #[case::store("bpmn-data-store", ShapeKind::Database)]
    #[case::other_bpmn("bpmn-subprocess", ShapeKind::BpmnTask)]
    #[case::unknown("mystery", ShapeKind::Unlisted)]
    #[case::empty("", ShapeKind::Unlisted)]
    fn stored_names_map_to_one_kind(#[case] name: &str, #[case] kind: ShapeKind) {
        assert_eq!(ShapeKind::from_stored_name(name), kind);
    }
}
