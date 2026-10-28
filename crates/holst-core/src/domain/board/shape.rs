//! Shape kinds: the outlines a shape on the board can have.

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
