//! Shape kinds: the outlines a shape on the board can have.

use std::str::FromStr;

use crate::error::{Error, Result, board_change::BoardChangeError};

const SHAPE_KINDS: &str = "square, rounded-rectangle, ellipse, triangle, diamond";

/// The outline of a shape: what a person creates and what the document holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeKind {
    #[default]
    Square,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Diamond,
    InvertedTriangle,
    RightParallelogram,
    LeftParallelogram,
    Trapezoid,
    ManualInput,
    OffPageConnector,
    Pentagon,
    Hexagon,
    Octagon,
    Star,
    Cross,
    ArrowRight,
    ArrowLeft,
    ArrowLeftRight,
    SpeechBubble,
    Terminator,
    Process,
    PredefinedProcess,
    Document,
    Database,
    BpmnTask,
    /// A stored kind this tool does not draw by name; drawn as a plain box.
    Unlisted,
}

impl ShapeKind {
    /// The name the document stores in `shapeType`; `Unlisted` is never made
    /// by a person, so it stores the default.
    #[must_use]
    pub const fn stored_name(self) -> &'static str {
        match self {
            ShapeKind::Square | ShapeKind::Unlisted => "square",
            ShapeKind::RoundedRectangle => "roundedRectangle",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Triangle => "triangle",
            ShapeKind::Diamond => "diamond",
            ShapeKind::InvertedTriangle => "invertedTriangle",
            ShapeKind::RightParallelogram => "rightParallelogram",
            ShapeKind::LeftParallelogram => "leftParallelogram",
            ShapeKind::Trapezoid => "basic-trapezoid",
            ShapeKind::ManualInput => "flowchart-manual-input",
            ShapeKind::OffPageConnector => "flowchart-off-page-connector",
            ShapeKind::Pentagon => "basic-pentagon",
            ShapeKind::Hexagon => "basic-hexagon",
            ShapeKind::Octagon => "basic-octagon",
            ShapeKind::Star => "basic-star",
            ShapeKind::Cross => "basic-cross",
            ShapeKind::ArrowRight => "basic-arrow-right",
            ShapeKind::ArrowLeft => "basic-arrow-left",
            ShapeKind::ArrowLeftRight => "basic-arrow-left-right",
            ShapeKind::SpeechBubble => "basic-speech-bubble",
            ShapeKind::Terminator => "flowchart-terminator",
            ShapeKind::Process => "flowchart-process",
            ShapeKind::PredefinedProcess => "flowchart-predefined-process",
            ShapeKind::Document => "flowchart-document",
            ShapeKind::Database => "flowchart-database",
            ShapeKind::BpmnTask => "bpmn-task",
        }
    }

    /// The kind of a stored `shapeType`; the names the web client draws alike
    /// map to one kind, and a name nobody here draws is `Unlisted`.
    #[must_use]
    pub fn from_stored_name(name: &str) -> Self {
        match name {
            "square" => ShapeKind::Square,
            "roundedRectangle" => ShapeKind::RoundedRectangle,
            "ellipse"
            | "flowchart-connector"
            | "bpmn-event-start"
            | "bpmn-event-intermediate"
            | "bpmn-event-end"
            | "flowchart-or"
            | "flowchart-summing-junction" => ShapeKind::Ellipse,
            "triangle" => ShapeKind::Triangle,
            "invertedTriangle" | "flowchart-merge" => ShapeKind::InvertedTriangle,
            "diamond" | "flowchart-decision" | "bpmn-gateway" => ShapeKind::Diamond,
            "rightParallelogram" | "flowchart-data" => ShapeKind::RightParallelogram,
            "leftParallelogram" => ShapeKind::LeftParallelogram,
            "basic-trapezoid" | "flowchart-manual-operation" => ShapeKind::Trapezoid,
            "flowchart-manual-input" => ShapeKind::ManualInput,
            "flowchart-off-page-connector" => ShapeKind::OffPageConnector,
            "basic-pentagon" => ShapeKind::Pentagon,
            "basic-hexagon" | "flowchart-preparation" | "bpmn-conversation" => {
                ShapeKind::Hexagon
            }
            "basic-octagon" => ShapeKind::Octagon,
            "basic-star" => ShapeKind::Star,
            "basic-cross" => ShapeKind::Cross,
            "basic-arrow-right" => ShapeKind::ArrowRight,
            "basic-arrow-left" => ShapeKind::ArrowLeft,
            "basic-arrow-left-right" => ShapeKind::ArrowLeftRight,
            "basic-speech-bubble" => ShapeKind::SpeechBubble,
            "flowchart-terminator" => ShapeKind::Terminator,
            "flowchart-process" => ShapeKind::Process,
            "flowchart-predefined-process" => ShapeKind::PredefinedProcess,
            "flowchart-document" => ShapeKind::Document,
            "flowchart-database" | "bpmn-data-store" => ShapeKind::Database,
            other if other.starts_with("bpmn-") => ShapeKind::BpmnTask,
            _ => ShapeKind::Unlisted,
        }
    }
}

impl FromStr for ShapeKind {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "square" | "rectangle" => Ok(ShapeKind::Square),
            "rounded-rectangle" | "roundedrectangle" => Ok(ShapeKind::RoundedRectangle),
            "ellipse" | "circle" => Ok(ShapeKind::Ellipse),
            "triangle" => Ok(ShapeKind::Triangle),
            "diamond" => Ok(ShapeKind::Diamond),
            _ => Err(Error::BoardChange(BoardChangeError::NotShapeKind {
                raw: raw.into(),
                known: SHAPE_KINDS,
            })),
        }
    }
}
