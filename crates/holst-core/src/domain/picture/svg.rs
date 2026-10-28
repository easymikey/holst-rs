//! Objects as SVG: numbers, text blocks, stickers, frames and the rest.

use std::borrow::Cow;

use serde_json::Value;

use crate::{
    domain::{
        area,
        geometry::{MAX_COORDINATE, Rectangle},
        palette,
        text_layout,
    },
    protocol::document,
};

pub(super) const FONTS: &str = "Inter, -apple-system, 'Helvetica Neue', 'Segoe UI', Roboto, Arial, \
                     'Noto Sans', sans-serif";

pub(crate) const MONO: &str =
    "'Source Code Pro', Menlo, Consolas, 'DejaVu Sans Mono', monospace";

pub(crate) const PLACEHOLDER: &str = "#e4e4e4";

pub(super) fn format_number(value: f64) -> String {
    let finite = if value.is_finite() { value } else { 0.0 }
        .clamp(-MAX_COORDINATE, MAX_COORDINATE);
    let rounded = (finite * 100.0).round() / 100.0 + 0.0;
    format!("{rounded}")
}

pub(crate) const fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\r' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..
    )
}

pub(super) fn escape(text: &str) -> Cow<'_, str> {
    text.find(|c| matches!(c, '&' | '<' | '>' | '"' | '\'') || !is_xml_char(c))
        .map_or(Cow::Borrowed(text), |_| {
            Cow::Owned(text.chars().filter(|c| is_xml_char(*c)).fold(
                String::new(),
                |mut out, c| {
                    match c {
                        '&' => out.push_str("&amp;"),
                        '<' => out.push_str("&lt;"),
                        '>' => out.push_str("&gt;"),
                        '"' => out.push_str("&quot;"),
                        '\'' => out.push_str("&apos;"),
                        c => out.push(c),
                    }
                    out
                },
            ))
        })
}

/// A stored color with its opacity.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Paint {
    pub color: String,
    pub opacity: f64,
}

pub(super) fn paint(fields: &Value, name: &str) -> Option<Paint> {
    let value = fields.get(name)?;
    let color = palette::of_value(value.get(document::COLOR).unwrap_or(value))?;
    let opacity = area::scene::number(value, document::OPACITY)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    Some(Paint { color, opacity })
}

pub(super) fn fill_attribute(paint: Option<&Paint>) -> String {
    paint.map_or_else(
        || "fill=\"none\"".to_owned(),
        |Paint { color, opacity }| {
            let alpha = if *opacity < 1.0 {
                format!(" fill-opacity=\"{}\"", format_number(*opacity))
            } else {
                String::new()
            };
            format!("fill=\"{color}\"{alpha}")
        },
    )
}

/// What a dashed stroke draws, which sets the gap of its dashes.
#[derive(Clone, Copy, Debug)]
pub(super) enum StrokeKind {
    Outline,
    Arrow,
}

/// How a stroke is broken into dashes, as the document names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl StrokeStyle {
    /// The style of a stored `strokeStyle`; anything else is solid.
    pub(super) fn of(name: Option<&str>) -> Self {
        // PROTOCOL: stroke style names, docs/board-protocol.md § Other types.
        match name {
            Some("dashed") => StrokeStyle::Dashed,
            Some("dotted") => StrokeStyle::Dotted,
            Some(_) | None => StrokeStyle::Solid,
        }
    }
}

pub(super) fn dashes(style: StrokeStyle, width: f64, kind: StrokeKind) -> String {
    let long = (2.0 * width).max(10.0);
    match (style, kind) {
        (StrokeStyle::Dashed, StrokeKind::Outline) => {
            format!(
                " stroke-dasharray=\"{} {}\"",
                format_number(long),
                format_number(long)
            )
        }
        (StrokeStyle::Dashed, StrokeKind::Arrow) => {
            format!(
                " stroke-dasharray=\"{} {}\"",
                format_number(long),
                format_number(long + width)
            )
        }
        (StrokeStyle::Dotted, StrokeKind::Outline) => {
            format!(
                " stroke-dasharray=\"{} {}\"",
                format_number(width),
                format_number(width)
            )
        }
        (StrokeStyle::Dotted, StrokeKind::Arrow) => {
            format!(" stroke-dasharray=\"0.01 {}\"", format_number(2.0 * width))
        }
        (StrokeStyle::Solid, StrokeKind::Outline | StrokeKind::Arrow) => String::new(),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TextAlign {
    pub(crate) horizontal: HorizontalAlign,
    pub(crate) vertical: VerticalAlign,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HorizontalAlign {
    Left,
    Centre,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum VerticalAlign {
    Top,
    Middle,
    Bottom,
}

impl TextAlign {
    pub(super) const CENTRE: TextAlign = TextAlign {
        horizontal: HorizontalAlign::Centre,
        vertical: VerticalAlign::Middle,
    };

    pub(crate) const TOP_LEFT: TextAlign = TextAlign {
        horizontal: HorizontalAlign::Left,
        vertical: VerticalAlign::Top,
    };

    pub(super) fn of(fields: &Value) -> TextAlign {
        let field = |name| fields.get(name).and_then(Value::as_str);
        TextAlign {
            // PROTOCOL: alignment names, docs/board-protocol.md § Other types.
            horizontal: match field(document::HORIZONTAL_ALIGN) {
                Some("left" | "justify") => HorizontalAlign::Left,
                Some("right") => HorizontalAlign::Right,
                _ => HorizontalAlign::Centre,
            },
            vertical: match field(document::VERTICAL_ALIGN) {
                Some("top") => VerticalAlign::Top,
                Some("bottom") => VerticalAlign::Bottom,
                _ => VerticalAlign::Middle,
            },
        }
    }
}

pub(super) struct TextBlock<'a> {
    pub(super) lines: &'a [String],
    pub(super) rectangle: Rectangle,
    pub(super) font_size: f64,
    pub(super) line_height: f64,
    pub(super) color: &'a str,
    pub(super) align: TextAlign,
    pub(super) weight: u16,
}

impl TextBlock<'_> {
    pub(super) fn svg(&self) -> String {
        let total = text_layout::count(self.lines.len()) * self.line_height;
        let top = match self.align.vertical {
            VerticalAlign::Top => self.rectangle.y,
            VerticalAlign::Middle => {
                self.rectangle.y + (self.rectangle.height - total) / 2.0
            }
            VerticalAlign::Bottom => self.rectangle.bottom() - total,
        };
        let (x, anchor) = match self.align.horizontal {
            HorizontalAlign::Left => (self.rectangle.x, "start"),
            HorizontalAlign::Centre => {
                (self.rectangle.x + self.rectangle.width / 2.0, "middle")
            }
            HorizontalAlign::Right => (self.rectangle.right(), "end"),
        };
        let spans = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(at, line)| {
                let baseline = top
                    + text_layout::count(at) * self.line_height
                    + self.line_height / 2.0
                    + 0.35 * self.font_size;
                format!(
                    "<tspan x=\"{}\" y=\"{}\">{}</tspan>",
                    format_number(x),
                    format_number(baseline),
                    escape(line)
                )
            })
            .collect::<Vec<_>>()
            .concat();
        let weight = if self.weight == 400 {
            String::new()
        } else {
            format!(" font-weight=\"{}\"", self.weight)
        };
        if spans.is_empty() {
            String::new()
        } else {
            format!(
                "<text font-size=\"{}\" fill=\"{}\" text-anchor=\"{anchor}\"{weight}>{spans}</text>",
                format_number(self.font_size),
                self.color
            )
        }
    }
}
