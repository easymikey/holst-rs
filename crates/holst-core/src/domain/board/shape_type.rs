//! The kinds of shape the web client draws, and the outline a shape holds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::{board::arrow_look::Bend, geometry::Size},
    protocol::document::{
        ARROW_TYPE,
        ARROWHEAD_END,
        ARROWHEAD_START,
        BORDER_RADIUS,
        COLOR,
        CONTROL,
        ELBOW_MIDWAY_CONTROLS,
        END,
        FILL_COLOR,
        MIDWAY_CONTROLS,
        OBJECT_ID,
        OPACITY,
        SHAPE_TYPE,
        START,
        STROKE_COLOR,
        STROKE_STYLE,
        STROKE_WIDTH,
    },
};

/// Makes the enum of shape kinds from the stored name and default size of each.
macro_rules! shape_types {
    ($($name:ident = $stored:literal, ($width:literal, $height:literal);)+) => {
        /// A shape kind of the web client's picker: what `shapeType` can hold.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
        #[serde(rename_all = "snake_case")]
        #[strum(serialize_all = "snake_case")]
        pub enum ShapeType {
            $($name,)+
        }

        impl ShapeType {
            const ALL: &'static [ShapeType] = &[$(ShapeType::$name,)+];

            /// The name the document stores in `shapeType`.
            #[must_use]
            pub const fn stored_name(self) -> &'static str {
                match self {
                    $(ShapeType::$name => $stored,)+
                }
            }

            /// The width and height the web client gives a new shape of the kind.
            #[must_use]
            pub const fn default_sides(self) -> (f64, f64) {
                match self {
                    $(ShapeType::$name => ($width, $height),)+
                }
            }
        }
    };
}

// BUNDLE: main.a33651a4.js 196553-196620 (names), 182154-182477 (sizes).
shape_types! {
    Square = "square", (144.0, 144.0);
    RoundedRectangle = "roundedRectangle", (144.0, 144.0);
    Ellipse = "ellipse", (144.0, 144.0);
    Triangle = "triangle", (168.0, 144.0);
    Diamond = "diamond", (144.0, 144.0);
    BasicSpeechBubble = "basic-speech-bubble", (176.0, 144.0);
    RightParallelogram = "rightParallelogram", (176.0, 144.0);
    BasicStar = "basic-star", (156.0, 144.0);
    BasicArrowRight = "basic-arrow-right", (176.0, 144.0);
    BasicArrowLeft = "basic-arrow-left", (176.0, 144.0);
    BasicArrowLeftRight = "basic-arrow-left-right", (200.0, 144.0);
    BasicPentagon = "basic-pentagon", (150.0, 144.0);
    BasicHexagon = "basic-hexagon", (168.0, 144.0);
    BasicOctagon = "basic-octagon", (144.0, 144.0);
    BasicTrapezoid = "basic-trapezoid", (176.0, 144.0);
    LeftParallelogram = "leftParallelogram", (176.0, 144.0);
    InvertedTriangle = "invertedTriangle", (168.0, 144.0);
    BasicCross = "basic-cross", (144.0, 144.0);
    BasicCloud = "basic-cloud", (176.0, 144.0);
    FlowchartProcess = "flowchart-process", (176.0, 128.0);
    FlowchartDecision = "flowchart-decision", (200.0, 128.0);
    FlowchartTerminator = "flowchart-terminator", (176.0, 106.0);
    FlowchartPredefinedProcess = "flowchart-predefined-process", (176.0, 128.0);
    FlowchartDocument = "flowchart-document", (176.0, 128.0);
    FlowchartMultipleDocuments = "flowchart-multiple-documents", (188.0, 128.0);
    FlowchartManualInput = "flowchart-manual-input", (176.0, 128.0);
    FlowchartPreparation = "flowchart-preparation", (176.0, 128.0);
    FlowchartData = "flowchart-data", (176.0, 128.0);
    FlowchartDatabase = "flowchart-database", (144.0, 128.0);
    FlowchartDirectAccessStorage = "flowchart-direct-access-storage", (176.0, 128.0);
    FlowchartInternalStorage = "flowchart-internal-storage", (176.0, 128.0);
    FlowchartManualOperation = "flowchart-manual-operation", (176.0, 128.0);
    FlowchartDelay = "flowchart-delay", (176.0, 128.0);
    FlowchartStoredData = "flowchart-stored-data", (168.0, 128.0);
    FlowchartMerge = "flowchart-merge", (168.0, 128.0);
    FlowchartConnector = "flowchart-connector", (148.0, 148.0);
    FlowchartOr = "flowchart-or", (86.0, 86.0);
    FlowchartSummingJunction = "flowchart-summing-junction", (86.0, 86.0);
    FlowchartDisplay = "flowchart-display", (176.0, 128.0);
    FlowchartOffPageConnector = "flowchart-off-page-connector", (156.0, 144.0);
    FlowchartLeftCurlyBraceAnnotation = "flowchart-left-curly-brace-annotation", (176.0, 144.0);
    FlowchartRightCurlyBraceAnnotation = "flowchart-right-curly-brace-annotation", (176.0, 144.0);
    FlowchartComment = "flowchart-comment", (176.0, 144.0);
    FlowchartNote = "flowchart-note", (168.0, 128.0);
    BpmnTask = "bpmn-task", (168.0, 128.0);
    BpmnTransaction = "bpmn-transaction", (168.0, 128.0);
    BpmnEventSubprocess = "bpmn-event-subprocess", (168.0, 128.0);
    BpmnCallActivity = "bpmn-call-activity", (168.0, 128.0);
    BpmnEventStart = "bpmn-event-start", (40.0, 40.0);
    BpmnEventStartNonInterrupting = "bpmn-event-start-non-interrupting", (40.0, 40.0);
    BpmnEventIntermediate = "bpmn-event-intermediate", (40.0, 40.0);
    BpmnEventIntermediateNonInterrupting = "bpmn-event-intermediate-non-interrupting", (40.0, 40.0);
    BpmnEventEnd = "bpmn-event-end", (40.0, 40.0);
    BpmnConversation = "bpmn-conversation", (46.0, 40.0);
    BpmnCallConversation = "bpmn-call-conversation", (46.0, 40.0);
    BpmnGateway = "bpmn-gateway", (48.0, 48.0);
    BpmnDataObject = "bpmn-data-object", (60.0, 80.0);
    BpmnDataStore = "bpmn-data-store", (144.0, 118.0);
    BpmnEmptyPool = "bpmn-empty-pool", (200.0, 40.0);
    BpmnGroup = "bpmn-group", (200.0, 384.0);
    BpmnAnnotation = "bpmn-annotation", (144.0, 118.0);
    BpmnMessage = "bpmn-message", (44.0, 33.0);
}

impl ShapeType {
    /// The kind a stored `shapeType` names; none for a name the picker lacks.
    #[must_use]
    pub fn from_stored_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.stored_name() == name)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StrokeStyle {
    Solid,
    Dashed,
    Dotted,
}

impl StrokeStyle {
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
        };
        (look.shape.is_some() || look.stroke.is_some()).then_some(look)
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
    let name = if fields
        .get(ARROW_TYPE)
        .and_then(Value::as_str)
        .is_some_and(|line| line != "elbow")
    {
        MIDWAY_CONTROLS
    } else {
        ELBOW_MIDWAY_CONTROLS
    };
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

/// Where an arrow's ends stick, as `start,end`; none when no end holds a side.
fn sides_of(fields: &Value) -> Option<String> {
    let side = |name: &str| {
        let end = fields.get(name)?;
        let control = end.get(CONTROL).and_then(Value::as_u64)?;
        end.get(OBJECT_ID)?;
        ["top", "right", "bottom", "left"]
            .get(usize::try_from(control).ok()?.checked_sub(1)?)
            .copied()
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
    fn every_kind_is_known_by_its_stored_name() {
        assert_eq!(ShapeType::ALL.len(), ShapeType::VARIANTS.len());
        assert!(ShapeType::ALL.iter().all(|kind| {
            ShapeType::from_stored_name(kind.stored_name()) == Some(*kind)
        }));
    }

    #[test]
    fn the_bpmn_set_is_the_last_eighteen() {
        let bpmn = ShapeType::ALL.iter().filter(|kind| kind.is_bpmn()).count();
        assert_eq!(bpmn, 18);
        assert!(ShapeType::BpmnMessage.is_bpmn() && !ShapeType::Square.is_bpmn());
    }
}
