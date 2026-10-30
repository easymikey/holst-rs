//! The kinds of shape the web client draws, and the outline a shape holds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::{
        board::{
            arrow_look::{ArrowType, Bend, EndSide},
            frame_look::{FrameBackground, FrameRatio},
        },
        geometry::Size,
    },
    protocol::document::{
        ARROW_TYPE,
        ARROWHEAD_END,
        ARROWHEAD_START,
        BACKGROUND_STYLE,
        BORDER_RADIUS,
        COLOR,
        CONTENT_HIDDEN,
        CONTROL,
        END,
        FILL_COLOR,
        OBJECT_ID,
        OPACITY,
        POSITION,
        RATIO,
        SHAPE_TYPE,
        START,
        STROKE_COLOR,
        STROKE_STYLE,
        STROKE_WIDTH,
        TEXT,
        TEXTS,
    },
};

/// A shape kind of the web client's picker: what `shapeType` can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ShapeType {
    Square,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Diamond,
    BasicSpeechBubble,
    RightParallelogram,
    BasicStar,
    BasicArrowRight,
    BasicArrowLeft,
    BasicArrowLeftRight,
    BasicPentagon,
    BasicHexagon,
    BasicOctagon,
    BasicTrapezoid,
    LeftParallelogram,
    InvertedTriangle,
    BasicCross,
    BasicCloud,
    FlowchartProcess,
    FlowchartDecision,
    FlowchartTerminator,
    FlowchartPredefinedProcess,
    FlowchartDocument,
    FlowchartMultipleDocuments,
    FlowchartManualInput,
    FlowchartPreparation,
    FlowchartData,
    FlowchartDatabase,
    FlowchartDirectAccessStorage,
    FlowchartInternalStorage,
    FlowchartManualOperation,
    FlowchartDelay,
    FlowchartStoredData,
    FlowchartMerge,
    FlowchartConnector,
    FlowchartOr,
    FlowchartSummingJunction,
    FlowchartDisplay,
    FlowchartOffPageConnector,
    FlowchartLeftCurlyBraceAnnotation,
    FlowchartRightCurlyBraceAnnotation,
    FlowchartComment,
    FlowchartNote,
    BpmnTask,
    BpmnTransaction,
    BpmnEventSubprocess,
    BpmnCallActivity,
    BpmnEventStart,
    BpmnEventStartNonInterrupting,
    BpmnEventIntermediate,
    BpmnEventIntermediateNonInterrupting,
    BpmnEventEnd,
    BpmnConversation,
    BpmnCallConversation,
    BpmnGateway,
    BpmnDataObject,
    BpmnDataStore,
    BpmnEmptyPool,
    BpmnGroup,
    BpmnAnnotation,
    BpmnMessage,
}

// BUNDLE: main.a33651a4.js 196553-196620 (names), 182154-182477 (sizes).
/// Each kind with its stored name and default size, in declaration order.
const SHAPES: &[(ShapeType, &str, (f64, f64))] = &[
    (ShapeType::Square, "square", (144.0, 144.0)),
    (
        ShapeType::RoundedRectangle,
        "roundedRectangle",
        (144.0, 144.0),
    ),
    (ShapeType::Ellipse, "ellipse", (144.0, 144.0)),
    (ShapeType::Triangle, "triangle", (168.0, 144.0)),
    (ShapeType::Diamond, "diamond", (144.0, 144.0)),
    (
        ShapeType::BasicSpeechBubble,
        "basic-speech-bubble",
        (176.0, 144.0),
    ),
    (
        ShapeType::RightParallelogram,
        "rightParallelogram",
        (176.0, 144.0),
    ),
    (ShapeType::BasicStar, "basic-star", (156.0, 144.0)),
    (
        ShapeType::BasicArrowRight,
        "basic-arrow-right",
        (176.0, 144.0),
    ),
    (
        ShapeType::BasicArrowLeft,
        "basic-arrow-left",
        (176.0, 144.0),
    ),
    (
        ShapeType::BasicArrowLeftRight,
        "basic-arrow-left-right",
        (200.0, 144.0),
    ),
    (ShapeType::BasicPentagon, "basic-pentagon", (150.0, 144.0)),
    (ShapeType::BasicHexagon, "basic-hexagon", (168.0, 144.0)),
    (ShapeType::BasicOctagon, "basic-octagon", (144.0, 144.0)),
    (ShapeType::BasicTrapezoid, "basic-trapezoid", (176.0, 144.0)),
    (
        ShapeType::LeftParallelogram,
        "leftParallelogram",
        (176.0, 144.0),
    ),
    (
        ShapeType::InvertedTriangle,
        "invertedTriangle",
        (168.0, 144.0),
    ),
    (ShapeType::BasicCross, "basic-cross", (144.0, 144.0)),
    (ShapeType::BasicCloud, "basic-cloud", (176.0, 144.0)),
    (
        ShapeType::FlowchartProcess,
        "flowchart-process",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartDecision,
        "flowchart-decision",
        (200.0, 128.0),
    ),
    (
        ShapeType::FlowchartTerminator,
        "flowchart-terminator",
        (176.0, 106.0),
    ),
    (
        ShapeType::FlowchartPredefinedProcess,
        "flowchart-predefined-process",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartDocument,
        "flowchart-document",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartMultipleDocuments,
        "flowchart-multiple-documents",
        (188.0, 128.0),
    ),
    (
        ShapeType::FlowchartManualInput,
        "flowchart-manual-input",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartPreparation,
        "flowchart-preparation",
        (176.0, 128.0),
    ),
    (ShapeType::FlowchartData, "flowchart-data", (176.0, 128.0)),
    (
        ShapeType::FlowchartDatabase,
        "flowchart-database",
        (144.0, 128.0),
    ),
    (
        ShapeType::FlowchartDirectAccessStorage,
        "flowchart-direct-access-storage",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartInternalStorage,
        "flowchart-internal-storage",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartManualOperation,
        "flowchart-manual-operation",
        (176.0, 128.0),
    ),
    (ShapeType::FlowchartDelay, "flowchart-delay", (176.0, 128.0)),
    (
        ShapeType::FlowchartStoredData,
        "flowchart-stored-data",
        (168.0, 128.0),
    ),
    (ShapeType::FlowchartMerge, "flowchart-merge", (168.0, 128.0)),
    (
        ShapeType::FlowchartConnector,
        "flowchart-connector",
        (148.0, 148.0),
    ),
    (ShapeType::FlowchartOr, "flowchart-or", (86.0, 86.0)),
    (
        ShapeType::FlowchartSummingJunction,
        "flowchart-summing-junction",
        (86.0, 86.0),
    ),
    (
        ShapeType::FlowchartDisplay,
        "flowchart-display",
        (176.0, 128.0),
    ),
    (
        ShapeType::FlowchartOffPageConnector,
        "flowchart-off-page-connector",
        (156.0, 144.0),
    ),
    (
        ShapeType::FlowchartLeftCurlyBraceAnnotation,
        "flowchart-left-curly-brace-annotation",
        (176.0, 144.0),
    ),
    (
        ShapeType::FlowchartRightCurlyBraceAnnotation,
        "flowchart-right-curly-brace-annotation",
        (176.0, 144.0),
    ),
    (
        ShapeType::FlowchartComment,
        "flowchart-comment",
        (176.0, 144.0),
    ),
    (ShapeType::FlowchartNote, "flowchart-note", (168.0, 128.0)),
    (ShapeType::BpmnTask, "bpmn-task", (168.0, 128.0)),
    (
        ShapeType::BpmnTransaction,
        "bpmn-transaction",
        (168.0, 128.0),
    ),
    (
        ShapeType::BpmnEventSubprocess,
        "bpmn-event-subprocess",
        (168.0, 128.0),
    ),
    (
        ShapeType::BpmnCallActivity,
        "bpmn-call-activity",
        (168.0, 128.0),
    ),
    (ShapeType::BpmnEventStart, "bpmn-event-start", (40.0, 40.0)),
    (
        ShapeType::BpmnEventStartNonInterrupting,
        "bpmn-event-start-non-interrupting",
        (40.0, 40.0),
    ),
    (
        ShapeType::BpmnEventIntermediate,
        "bpmn-event-intermediate",
        (40.0, 40.0),
    ),
    (
        ShapeType::BpmnEventIntermediateNonInterrupting,
        "bpmn-event-intermediate-non-interrupting",
        (40.0, 40.0),
    ),
    (ShapeType::BpmnEventEnd, "bpmn-event-end", (40.0, 40.0)),
    (
        ShapeType::BpmnConversation,
        "bpmn-conversation",
        (46.0, 40.0),
    ),
    (
        ShapeType::BpmnCallConversation,
        "bpmn-call-conversation",
        (46.0, 40.0),
    ),
    (ShapeType::BpmnGateway, "bpmn-gateway", (48.0, 48.0)),
    (ShapeType::BpmnDataObject, "bpmn-data-object", (60.0, 80.0)),
    (ShapeType::BpmnDataStore, "bpmn-data-store", (144.0, 118.0)),
    (ShapeType::BpmnEmptyPool, "bpmn-empty-pool", (200.0, 40.0)),
    (ShapeType::BpmnGroup, "bpmn-group", (200.0, 384.0)),
    (ShapeType::BpmnAnnotation, "bpmn-annotation", (144.0, 118.0)),
    (ShapeType::BpmnMessage, "bpmn-message", (44.0, 33.0)),
];

impl ShapeType {
    /// The name the document stores in `shapeType`.
    #[expect(
        clippy::indexing_slicing,
        clippy::as_conversions,
        reason = "the table test pins order"
    )]
    #[must_use]
    pub const fn stored_name(self) -> &'static str {
        SHAPES[self as usize].1
    }

    /// The width and height the web client gives a new shape of the kind.
    #[expect(
        clippy::indexing_slicing,
        clippy::as_conversions,
        reason = "the table test pins order"
    )]
    #[must_use]
    pub const fn default_sides(self) -> (f64, f64) {
        SHAPES[self as usize].2
    }
}

impl ShapeType {
    /// The kind a stored `shapeType` names; none for a name the picker lacks.
    #[must_use]
    pub fn from_stored_name(name: &str) -> Option<Self> {
        SHAPES
            .iter()
            .find(|(_, stored, _)| *stored == name)
            .map(|(kind, ..)| *kind)
    }

    /// Whether the kind is one of the BPMN set, which keeps `bpmnResources`.
    #[must_use]
    pub fn is_bpmn(self) -> bool {
        self.stored_name().starts_with("bpmn-")
    }

    /// The size of a new shape of the kind.
    #[must_use]
    pub fn default_size(self) -> Option<Size> {
        let (width, height) = self.default_sides();
        Size::new(width, height).ok()
    }

    /// The kinds whose corners round a half side; the others a sixth.
    #[must_use]
    pub const fn rounds_by_half(self) -> bool {
        matches!(
            self,
            ShapeType::Square
                | ShapeType::RoundedRectangle
                | ShapeType::FlowchartProcess
                | ShapeType::FlowchartPredefinedProcess
                | ShapeType::FlowchartInternalStorage
                | ShapeType::BpmnTask
                | ShapeType::BpmnTransaction
                | ShapeType::BpmnEventSubprocess
                | ShapeType::BpmnCallActivity
                | ShapeType::BpmnEmptyPool
                | ShapeType::BpmnGroup
                | ShapeType::BpmnMessage
        )
    }
}

/// How the line of a shape is drawn.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl StrokeStyle {
    /// The style of a stored `strokeStyle`; anything else is solid.
    #[must_use]
    pub fn of_stored(name: Option<&str>) -> Self {
        match name {
            Some("dashed") => StrokeStyle::Dashed,
            Some("dotted") => StrokeStyle::Dotted,
            Some(_) | None => StrokeStyle::Solid,
        }
    }

    /// The name the document stores in `strokeStyle`.
    #[must_use]
    pub const fn stored_name(self) -> &'static str {
        match self {
            StrokeStyle::Solid => "solid",
            StrokeStyle::Dashed => "dashed",
            StrokeStyle::Dotted => "dotted",
        }
    }
}

/// What a listing shows of a shape or an arrow: its kind and look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeLook {
    pub shape: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f64>,
    pub stroke_style: Option<String>,
    pub radius: Option<f64>,
    pub opacity: Option<f64>,
    /// An arrow's line: `straight`, `elbow` or `curved`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// An arrow's heads as `start,end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heads: Option<String>,
    /// An arrow's bend, one entry per control: `x,y`, or `x,` and `,y`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bend: Vec<String>,
    /// An arrow's sides as `start,end`: top to left, `auto`, or `free`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<String>,
    /// An arrow's labels as `text@position`, the place from 0 to 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// A frame's ratio: `16:9`, `4:3`, `1:1`, `a4` or `a4_landscape`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<String>,
    /// A frame's background: `fill`, `border` or `outline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Whether a frame hides its content.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl ShapeLook {
    /// The look the stored fields of a shape hold; none when they hold none.
    #[must_use]
    pub fn of(fields: &Value) -> Option<Self> {
        let text =
            |name: &str| fields.get(name).and_then(Value::as_str).map(str::to_owned);
        let number = |name: &str| fields.get(name).and_then(Value::as_f64);
        let inside =
            |name: &str, key: &str| fields.get(name).and_then(|inner| inner.get(key));
        let look = Self {
            shape: text(SHAPE_TYPE),
            stroke: inside(STROKE_COLOR, COLOR)
                .and_then(Value::as_str)
                .map(str::to_owned),
            stroke_width: number(STROKE_WIDTH),
            stroke_style: text(STROKE_STYLE),
            radius: number(BORDER_RADIUS),
            opacity: inside(FILL_COLOR, OPACITY).and_then(Value::as_f64),
            line: text(ARROW_TYPE),
            heads: heads_of(fields),
            bend: bend_of(fields),
            sides: sides_of(fields),
            labels: labels_of(fields),
            ratio: fields
                .get(RATIO)
                .and_then(Value::as_u64)
                .and_then(FrameRatio::of_stored)
                .map(|ratio| <&str>::from(ratio).to_owned()),
            background: fields
                .get(BACKGROUND_STYLE)
                .and_then(Value::as_u64)
                .and_then(FrameBackground::of_stored)
                .map(|background| <&str>::from(background).to_owned()),
            hidden: fields
                .get(CONTENT_HIDDEN)
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        };
        (look.shape.is_some()
            || look.stroke.is_some()
            || look.ratio.is_some()
            || look.background.is_some()
            || look.hidden)
            .then_some(look)
    }

    /// The look in a few words for a line of a listing.
    #[must_use]
    pub fn words(&self) -> String {
        [
            self.shape.clone(),
            self.line.clone(),
            self.heads.as_ref().map(|heads| format!("heads {heads}")),
            (!self.bend.is_empty()).then(|| format!("bend {}", self.bend.join(" "))),
            self.sides.as_ref().map(|sides| format!("sides {sides}")),
            (!self.labels.is_empty())
                .then(|| format!("labels {}", self.labels.join(" "))),
            self.ratio.as_ref().map(|ratio| format!("ratio {ratio}")),
            self.background
                .as_ref()
                .map(|background| format!("background {background}")),
            self.hidden.then(|| "hidden".to_owned()),
            self.stroke
                .as_ref()
                .map(|stroke| format!("stroke {stroke}")),
            self.stroke_width.map(|width| format!("{width}px")),
            self.stroke_style
                .as_ref()
                .filter(|style| *style != "solid")
                .cloned(),
            self.radius
                .filter(|radius| *radius > 0.0)
                .map(|radius| format!("radius {radius}")),
            self.opacity
                .filter(|opacity| *opacity < 1.0)
                .map(|opacity| format!("opacity {opacity}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
    }
}

/// The controls an arrow bends at, those of its line kind.
fn bend_of(fields: &Value) -> Vec<String> {
    let line = ArrowType::of_stored(fields.get(ARROW_TYPE).and_then(Value::as_str));
    let name = line.controls_field();
    let axis = |control: &Value, axis: &str| control.get(axis).and_then(Value::as_f64);
    fields
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|control| {
            Bend {
                x: axis(control, "x"),
                y: axis(control, "y"),
            }
            .to_string()
        })
        .collect()
}

/// The labels of an arrow as `text@position`; a label with no place sits in
/// the middle.
fn labels_of(fields: &Value) -> Vec<String> {
    fields
        .get(TEXTS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|label| {
            let text = label.get(TEXT).and_then(Value::as_str)?;
            let place = label.get(POSITION).and_then(Value::as_f64).unwrap_or(0.5);
            Some(format!("{text}@{place}"))
        })
        .collect()
}

/// Where an arrow's ends stick, as `start,end`; none when no end holds a side.
fn sides_of(fields: &Value) -> Option<String> {
    let side = |name: &str| {
        let end = fields.get(name)?;
        let control = end.get(CONTROL).and_then(Value::as_u64)?;
        end.get(OBJECT_ID)?;
        EndSide::of_control(control).map(<&str>::from)
    };
    let rest = |name: &str| {
        let on_object = fields.get(name).and_then(|end| end.get(OBJECT_ID));
        on_object.map_or("free", |_| "auto")
    };
    let (start, end) = (side(START), side(END));
    (start.is_some() || end.is_some()).then(|| {
        format!(
            "{},{}",
            start.unwrap_or_else(|| rest(START)),
            end.unwrap_or_else(|| rest(END))
        )
    })
}

/// An arrow's heads as `start,end`, the web client's defaults where the
/// document is silent; none for a document that names no head.
fn heads_of(fields: &Value) -> Option<String> {
    let head = |name: &str, side: &str, default: &'static str| {
        fields
            .get(name)
            .and_then(Value::as_str)
            .or_else(|| {
                fields
                    .get(side)
                    .and_then(|end| end.get("arrowhead"))
                    .and_then(Value::as_str)
            })
            .unwrap_or(default)
    };
    let named =
        fields.get(ARROWHEAD_START).is_some() || fields.get(ARROWHEAD_END).is_some();
    named.then(|| {
        format!(
            "{},{}",
            head(ARROWHEAD_START, START, "none"),
            head(ARROWHEAD_END, END, "miro")
        )
    })
}

#[cfg(test)]
mod tests {
    use strum::VariantNames as _;

    use super::*;

    #[test]
    fn a_stored_stroke_style_reads_back_and_anything_else_is_solid() {
        let styles = [StrokeStyle::Solid, StrokeStyle::Dashed, StrokeStyle::Dotted];
        assert!(styles.iter().all(|style| {
            StrokeStyle::of_stored(Some(style.stored_name())) == *style
        }));
        assert_eq!(StrokeStyle::of_stored(Some("zigzag")), StrokeStyle::Solid);
        assert_eq!(StrokeStyle::of_stored(None), StrokeStyle::Solid);
    }

    #[test]
    fn every_kind_is_known_by_its_stored_name() {
        assert_eq!(SHAPES.len(), ShapeType::VARIANTS.len());
        assert!(SHAPES.iter().all(|(kind, name, _)| {
            ShapeType::from_stored_name(name) == Some(*kind)
        }));
    }

    #[test]
    fn the_table_lists_each_kind_once_in_declaration_order() {
        assert!(SHAPES.iter().all(|(kind, name, sides)| {
            kind.stored_name() == *name && kind.default_sides() == *sides
        }));
    }

    #[test]
    fn the_bpmn_set_is_the_last_eighteen() {
        let bpmn = SHAPES.iter().filter(|(kind, ..)| kind.is_bpmn()).count();
        assert_eq!(bpmn, 18);
        assert!(ShapeType::BpmnMessage.is_bpmn() && !ShapeType::Square.is_bpmn());
    }

    #[test]
    fn an_unknown_line_name_bends_at_the_elbow_controls() {
        let fields = serde_json::json!({
            "arrowType": "spline",
            "strokeColor": { "color": "#111111" },
            "elbowMidwayControls": [{ "x": 3.0, "y": 4.0 }],
            "midwayControls": [{ "x": 7.0, "y": 8.0 }],
        });
        let bend = ShapeLook::of(&fields).map(|look| look.bend);
        assert_eq!(bend, Some(vec!["3,4".to_owned()]));
    }
}
