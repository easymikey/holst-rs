//! Arrows as SVG: the route, heads and labels.

use itertools::Itertools as _;
use serde_json::Value;

use crate::{
    domain::{
        area,
        area::route::{ArrowLine, ArrowRoute, ObjectSide},
        board::Object,
        geometry::{Direction, Rectangle, StoredPoint},
        palette,
        palette::TEXT_COLOR,
        picture::svg::{
            StrokeKind,
            StrokeStyle,
            TextAlign,
            TextBlock,
            dashes,
            escape,
            format_number,
            paint,
        },
        text_layout,
    },
    protocol::document,
};

/// How an arrow ends, as the document names the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrowhead {
    None,
    Triangle,
    Rhombus,
    Circle,
    Arc,
    Miro,
    Figma,
    /// Any other name: two open strokes.
    Open,
}

impl Arrowhead {
    fn of(name: &str) -> Self {
        // PROTOCOL: arrowhead names.
        match name {
            "none" => Arrowhead::None,
            "triangle" => Arrowhead::Triangle,
            "rhombus" => Arrowhead::Rhombus,
            "circle" => Arrowhead::Circle,
            "arc" => Arrowhead::Arc,
            "miro" => Arrowhead::Miro,
            "figma" => Arrowhead::Figma,
            _ => Arrowhead::Open,
        }
    }

    fn length(self, width: f64) -> f64 {
        let base = match self {
            Arrowhead::Triangle => 15.0,
            Arrowhead::Circle => 14.0,
            Arrowhead::Arc => 17.0,
            Arrowhead::Figma => 11.0,
            Arrowhead::None
            | Arrowhead::Rhombus
            | Arrowhead::Miro
            | Arrowhead::Open => 12.0,
        };
        base * (width / 2.0).max(1.0).sqrt()
    }

    fn inset(self, length: f64) -> f64 {
        match self {
            Arrowhead::Miro | Arrowhead::Figma => 0.6 * length,
            Arrowhead::Triangle | Arrowhead::Rhombus | Arrowhead::Circle => length,
            Arrowhead::None | Arrowhead::Arc | Arrowhead::Open => 0.0,
        }
    }
}

/// How an arrowhead looks: its kind, the stroke width and the color.
#[derive(Debug, Clone, Copy)]
struct HeadStyle<'a> {
    head: Arrowhead,
    width: f64,
    color: &'a str,
}

fn head(
    HeadStyle {
        head: kind,
        width,
        color,
    }: HeadStyle<'_>,
    end: StoredPoint,
    aim: Direction,
) -> String {
    let length = kind.length(width);
    let spread = |degrees: f64, reach: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        StoredPoint {
            x: end.x - reach * (aim.dx * cos - aim.dy * sin),
            y: end.y - reach * (aim.dx * sin + aim.dy * cos),
        }
    };
    let back = |share: f64| StoredPoint {
        x: end.x - aim.dx * length * share,
        y: end.y - aim.dy * length * share,
    };
    let points = |all: &[StoredPoint]| {
        all.iter()
            .map(|spot| format!("{},{}", format_number(spot.x), format_number(spot.y)))
            .join(" ")
    };
    match kind {
        Arrowhead::None => String::new(),
        Arrowhead::Triangle => format!(
            "<polygon points=\"{}\" fill=\"{color}\"/>",
            points(&[end, spread(30.0, length), spread(-30.0, length)])
        ),
        Arrowhead::Rhombus => format!(
            "<polygon points=\"{}\" fill=\"{color}\"/>",
            points(&[
                end,
                spread(30.0, length / 2.0),
                back(1.0),
                spread(-30.0, length / 2.0)
            ])
        ),
        Arrowhead::Circle => {
            let centre = back(0.5);
            format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{color}\"/>",
                format_number(centre.x),
                format_number(centre.y),
                format_number(length / 2.0)
            )
        }
        Arrowhead::Miro | Arrowhead::Figma => format!(
            "<polygon points=\"{}\" fill=\"{color}\" stroke=\"{color}\" stroke-width=\"{}\" \
             stroke-linejoin=\"round\"/>",
            points(&[end, spread(32.7, length), back(0.6), spread(-32.7, length)]),
            format_number(width / 2.0)
        ),
        Arrowhead::Arc | Arrowhead::Open => format!(
            "<polyline points=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" \
             stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
            points(&[spread(32.7, length), end, spread(-32.7, length)]),
            format_number(width)
        ),
    }
}

fn unit(from: StoredPoint, to: StoredPoint) -> Direction {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = dx.hypot(dy);
    Some(length).filter(|length| *length > 1e-9).map_or(
        Direction { dx: 1.0, dy: 0.0 },
        |length| Direction {
            dx: dx / length,
            dy: dy / length,
        },
    )
}

fn moved(point: StoredPoint, direction: Direction, by: f64) -> StoredPoint {
    StoredPoint {
        x: point.x + direction.dx * by,
        y: point.y + direction.dy * by,
    }
}

fn route_path(route: &ArrowRoute, points: &[StoredPoint]) -> String {
    let first = points.first().copied().unwrap_or_default();
    let last = points.last().copied().unwrap_or_default();
    match route.line {
        ArrowLine::Curved => {
            let reach = 0.4 * first.distance_to(last);
            let handle =
                |end: &area::route::ArrowEnd, at: StoredPoint, other: StoredPoint| {
                    let direction =
                        end.side.map_or_else(|| unit(at, other), ObjectSide::normal);
                    moved(at, direction, reach)
                };
            let c1 = handle(&route.start, first, last);
            let c2 = handle(&route.end, last, first);
            format!(
                "M{} {} C{} {} {} {} {} {}",
                format_number(first.x),
                format_number(first.y),
                format_number(c1.x),
                format_number(c1.y),
                format_number(c2.x),
                format_number(c2.y),
                format_number(last.x),
                format_number(last.y)
            )
        }
        ArrowLine::Straight | ArrowLine::Elbow => {
            let corners = rounded_corners(points);
            format!(
                "M{} {}{corners} L{} {}",
                format_number(first.x),
                format_number(first.y),
                format_number(last.x),
                format_number(last.y)
            )
        }
    }
}

fn rounded_corners(points: &[StoredPoint]) -> String {
    points
        .windows(3)
        .filter_map(|w| match w {
            [a, b, c] => Some((*a, *b, *c)),
            _ => None,
        })
        .map(|(a, b, c)| {
            let radius = (a.distance_to(b) / 2.0)
                .min(b.distance_to(c) / 2.0)
                .floor()
                .min(20.0);
            let enter = moved(b, unit(b, a), radius);
            let leave = moved(b, unit(b, c), radius);
            format!(
                " L{} {} Q{} {} {} {}",
                format_number(enter.x),
                format_number(enter.y),
                format_number(b.x),
                format_number(b.y),
                format_number(leave.x),
                format_number(leave.y)
            )
        })
        .collect::<Vec<_>>()
        .concat()
}

fn halfway(points: &[StoredPoint]) -> StoredPoint {
    let lengths: Vec<f64> = points
        .windows(2)
        .map(|w| match w {
            [a, b] => a.distance_to(*b),
            _ => 0.0,
        })
        .collect();
    let half = lengths.iter().sum::<f64>() / 2.0;
    let found = points
        .windows(2)
        .zip(&lengths)
        .scan(0.0, |walked, (pair, length)| {
            let before = *walked;
            *walked += length;
            Some((before, pair, *length))
        })
        .find(|(before, _, length)| before + length >= half);
    found
        .and_then(|(before, pair, length)| match pair {
            [a, b] => Some(moved(*a, unit(*a, *b), (half - before).min(length))),
            _ => None,
        })
        .or_else(|| points.first().copied())
        .unwrap_or_default()
}

/// Which way each end of an arrow points.
struct EndDirections {
    start: Direction,
    end: Direction,
}

fn end_directions(route: &ArrowRoute) -> EndDirections {
    let points = &route.points;
    let first = points.first().copied().unwrap_or_default();
    let last = points.last().copied().unwrap_or_default();
    let curved = matches!(route.line, ArrowLine::Curved);
    let before_last = if curved {
        first
    } else {
        points.iter().rev().nth(1).copied().unwrap_or(first)
    };
    let after_first = if curved {
        last
    } else {
        points.get(1).copied().unwrap_or(last)
    };
    let inwards = |side: ObjectSide| side.normal().reversed();
    let start = match (curved, route.start.side) {
        (true, Some(side)) => inwards(side),
        _ => unit(after_first, first),
    };
    let end = match (curved, route.end.side) {
        (true, Some(side)) => inwards(side),
        _ => unit(before_last, last),
    };
    EndDirections { start, end }
}

fn filled_rectangle(rectangle: &Rectangle, fill: &str) -> String {
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"/>",
        format_number(rectangle.x),
        format_number(rectangle.y),
        format_number(rectangle.width),
        format_number(rectangle.height)
    )
}

/// An arrow's label: the mask cutting the line, its attribute, the text.
#[derive(Debug, Default)]
struct ArrowText {
    mask: String,
    masked: String,
    text: String,
}

fn arrow_text_svg(object: &Object, points: &[StoredPoint], width: f64) -> ArrowText {
    let fields = &object.fields;
    let id = escape(&object.id);
    let size = area::scene::number(fields, document::FONT_SIZE).unwrap_or(14.0);
    let lines = text_layout::wrap(&object.text, 240.0, size).lines;
    let label_width = lines
        .iter()
        .map(|line| text_layout::width(line, size))
        .fold(0.0, f64::max);
    let line = (1.5 * size).round();
    let label_height = text_layout::count(lines.len()) * line;
    let centre = halfway(points);
    let cut = Rectangle::new(
        centre.x - label_width / 2.0 - 5.0,
        centre.y - label_height / 2.0 - 5.0,
        label_width + 10.0,
        label_height + 10.0,
    );
    let reach = Rectangle::around(points.iter().copied())
        .unwrap_or(cut)
        .union(cut)
        .padded(width + 40.0);
    let color = paint(fields, document::TEXT_COLOR)
        .map_or_else(|| TEXT_COLOR.to_owned(), |paint| paint.color);
    let text = TextBlock {
        lines: &lines,
        rectangle: Rectangle::new(cut.x + 5.0, cut.y + 5.0, label_width, label_height),
        font_size: size,
        line_height: line,
        color: &color,
        align: TextAlign::CENTRE,
        weight: 400,
    };
    if object.text.trim().is_empty() {
        ArrowText::default()
    } else {
        ArrowText {
            mask: format!(
                "<mask id=\"m-{id}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" \
                 width=\"{}\" height=\"{}\">{}{}</mask>",
                format_number(reach.x),
                format_number(reach.y),
                format_number(reach.width),
                format_number(reach.height),
                filled_rectangle(&reach, "#fff"),
                filled_rectangle(&cut, "#000")
            ),
            masked: format!(" mask=\"url(#m-{id})\""),
            text: text.svg(),
        }
    }
}

pub(super) fn arrow(object: &Object, route: &ArrowRoute) -> String {
    let fields = &object.fields;
    let width = area::scene::stroke_width(fields);
    let color = paint(fields, document::STROKE_COLOR).map_or_else(
        || palette::hex("gray8").unwrap_or(TEXT_COLOR).to_owned(),
        |paint| paint.color,
    );
    let field = |name| fields.get(name).and_then(Value::as_str);
    let start_kind =
        field(document::ARROWHEAD_START).map_or(Arrowhead::None, Arrowhead::of);
    let end_kind =
        field(document::ARROWHEAD_END).map_or(Arrowhead::Miro, Arrowhead::of);
    let points = &route.points;
    let first = points.first().copied().unwrap_or_default();
    let last = points.last().copied().unwrap_or_default();
    let EndDirections {
        start: start_direction,
        end: end_direction,
    } = end_directions(route);
    let start_inset = start_kind.inset(start_kind.length(width));
    let end_inset = end_kind.inset(end_kind.length(width));
    let last_index = points.len().saturating_sub(1);
    let trimmed: Vec<StoredPoint> = points
        .iter()
        .enumerate()
        .map(|(index, &point)| match index {
            _ if index == last_index => moved(last, end_direction, -end_inset),
            0 => moved(first, start_direction, -start_inset),
            _ => point,
        })
        .collect();
    let d = route_path(route, &trimmed);
    let ArrowText { mask, masked, text } = arrow_text_svg(object, points, width);
    format!(
        "{mask}<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" \
         stroke-linejoin=\"round\" stroke-linecap=\"round\"{}{masked}/>{}{}{text}",
        format_number(width),
        dashes(
            StrokeStyle::of(field(document::STROKE_STYLE)),
            width,
            StrokeKind::Arrow
        ),
        head(
            HeadStyle {
                head: start_kind,
                width,
                color: &color
            },
            first,
            start_direction
        ),
        head(
            HeadStyle {
                head: end_kind,
                width,
                color: &color
            },
            last,
            end_direction
        ),
    )
}
