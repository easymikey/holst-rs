//! The fields an edit writes for a shape's look and an arrow's line.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{
    domain::{
        board::{
            arrow_look::{ArrowEndEdit, ArrowType},
            shape_type::ShapeType,
        },
        change::{
            check::{find, rectangle_of},
            value::{ArrowEdit, ShapeEdit},
        },
        geometry::{Size, whole_u32},
        id_source::IdSource,
    },
    protocol::document::{
        ARROW_TYPE,
        ARROWHEAD_END,
        ARROWHEAD_START,
        BORDER_RADIUS,
        BPMN_RESOURCES,
        COLOR,
        CONTROL,
        EDITOR,
        ELBOW_MIDWAY_CONTROLS,
        END,
        FILL_COLOR,
        FIXED_SIZE,
        HEIGHT,
        ID,
        MIDWAY_CONTROLS,
        OBJECT_ID,
        OPACITY,
        POINT,
        POSITION,
        RELATIVE_POINT,
        SHAPE_TYPE,
        START,
        STROKE_COLOR,
        STROKE_STYLE,
        STROKE_WIDTH,
        TEXT,
        TEXTS,
        WIDTH,
    },
};

type Changes = Vec<(&'static str, Value)>;

/// The fields `edit` sets on a shape whose stored fields are `fields`; a new
/// size in the same edit is the one the radius is checked against.
pub(super) fn shape_changes(
    edit: &ShapeEdit,
    fields: &Value,
    size: Option<Size>,
    colored: bool,
) -> Changes {
    let kind = edit.kind.map(|kind| kind_changes(kind, fields));
    let stroke = edit.stroke.as_ref().map(|color| {
        let opacity = fields
            .get(STROKE_COLOR)
            .and_then(|stroke| stroke.get(OPACITY))
            .cloned()
            .unwrap_or_else(|| json!(1));
        (
            STROKE_COLOR,
            json!({ (COLOR): color.as_str(), (OPACITY): opacity }),
        )
    });
    let width = edit.stroke_width.map(|width| (STROKE_WIDTH, json!(width)));
    let style = edit
        .stroke_style
        .map(|style| (STROKE_STYLE, json!(style.stored_name())));
    let radius = edit.radius.map(|radius| {
        (
            BORDER_RADIUS,
            json!(radius.min(max_radius(edit, fields, size))),
        )
    });
    // A fill color in the same edit writes the opacity itself.
    let opacity = edit.opacity.filter(|_| !colored).map(|opacity| {
        let mut fill = fields.get(FILL_COLOR).cloned().unwrap_or_else(|| json!({}));
        if let Some(map) = fill.as_object_mut() {
            map.insert(OPACITY.to_owned(), json!(opacity));
        }
        (FILL_COLOR, fill)
    });
    kind.into_iter()
        .flatten()
        .chain(
            [stroke, width, style, radius, opacity]
                .into_iter()
                .flatten(),
        )
        .collect()
}

/// The line an arrow has: the one an edit gives, else the stored, else elbow.
pub(super) fn line_of(chosen: Option<ArrowType>, fields: &Value) -> ArrowType {
    chosen.unwrap_or_else(|| {
        ArrowType::of_stored(fields.get(ARROW_TYPE).and_then(Value::as_str))
    })
}

/// The fields `edit` sets on an arrow whose stored fields are `fields`: its
/// line, heads, bend, ends and labels.
pub(super) fn arrow_changes(
    edit: &ArrowEdit,
    fields: &Value,
    states: &HashMap<String, Value>,
    ids: &dyn IdSource,
) -> Changes {
    // BUNDLE: an elbow bends on one axis (main.a33651a4.js 162183).
    let bend = edit.bend.map(|bend| {
        let name = if line_of(edit.line, fields) == ArrowType::Elbow {
            ELBOW_MIDWAY_CONTROLS
        } else {
            MIDWAY_CONTROLS
        };
        (name, json!([{ "x": bend.x, "y": bend.y }]))
    });
    // BUNDLE: text, position, editor (main.a33651a4.js 162034-162041).
    let labels = (!edit.labels.is_empty()).then(|| {
        let texts = edit.labels.iter().map(|label| {
            json!({
                (ID): ids.new_id(),
                (TEXT): label.text,
                (POSITION): label.position,
                (EDITOR): [{ "type": "paragraph", "children": [{ (TEXT): label.text }] }],
            })
        });
        (TEXTS, Value::Array(texts.collect()))
    });
    let end = |name, asked: &Option<ArrowEndEdit>| {
        asked
            .as_ref()
            .and_then(|asked| attached(asked, states))
            .map(|value| (name, value))
    };
    [
        edit.line
            .map(|line| (ARROW_TYPE, json!(line.stored_name()))),
        edit.heads
            .start
            .map(|head| (ARROWHEAD_START, json!(head.stored_name()))),
        edit.heads
            .end
            .map(|head| (ARROWHEAD_END, json!(head.stored_name()))),
        bend,
        end(START, &edit.from),
        end(END, &edit.to),
        labels,
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// BUNDLE: an end on a side keeps the object, the side as `control`, its
/// middle as `relativePoint` and on the board as `point`; a free end only its
/// point (main.a33651a4.js 129402-129417).
fn attached(edit: &ArrowEndEdit, states: &HashMap<String, Value>) -> Option<Value> {
    match edit {
        ArrowEndEdit::Free(at) => Some(json!({ (POINT): at })),
        ArrowEndEdit::Side { object, side } => {
            let id = find(states, object).ok()?;
            let rect = rectangle_of(states.get(&id)?)?;
            let (across, down) = side.relative_point();
            Some(json!({
                (OBJECT_ID): id,
                (CONTROL): side.control(),
                (RELATIVE_POINT): { "x": across, "y": down },
                (POINT): {
                    "x": rect.x + across * rect.width,
                    "y": rect.y + down * rect.height,
                },
            }))
        }
    }
}

/// The kind and what follows from it: BPMN kinds drop their markers and keep
/// the shape's footprint in the ratio of the default sizes, the others become
/// fixed in size unless they already have the new kind's default height.
fn kind_changes(kind: ShapeType, fields: &Value) -> Changes {
    let named = (SHAPE_TYPE, json!(kind.stored_name()));
    if kind.is_bpmn() {
        let old = stored_kind(fields).default_sides();
        let new = kind.default_sides();
        let grown = rectangle_of(fields).map(|rect| {
            let scale = (rect.width / old.0).min(rect.height / old.1);
            let (width, height) = (new.0 * scale, new.1 * scale);
            let (x, y) = (
                rect.x + rect.width / 2.0 - width / 2.0,
                rect.y + rect.height / 2.0 - height / 2.0,
            );
            [
                (WIDTH, json!(width)),
                (HEIGHT, json!(height)),
                (POSITION, json!({ "x": x, "y": y })),
            ]
        });
        [named, (BPMN_RESOURCES, json!({}))]
            .into_iter()
            .chain(grown.into_iter().flatten())
            .collect()
    } else {
        let fixed = fields.get(FIXED_SIZE).and_then(Value::as_bool) == Some(true);
        let same =
            fields.get(HEIGHT).and_then(Value::as_f64) == Some(kind.default_sides().1);
        [named]
            .into_iter()
            .chain((!fixed && !same).then_some((FIXED_SIZE, json!(true))))
            .collect()
    }
}

/// The kind the shape holds; a name the picker lacks counts as a square.
fn stored_kind(fields: &Value) -> ShapeType {
    fields
        .get(SHAPE_TYPE)
        .and_then(Value::as_str)
        .and_then(ShapeType::from_stored_name)
        .unwrap_or(ShapeType::Square)
}

/// The largest radius the shape allows: a half or a sixth of the shorter side.
fn max_radius(edit: &ShapeEdit, fields: &Value, size: Option<Size>) -> u32 {
    let kind = edit.kind.unwrap_or_else(|| stored_kind(fields));
    let divisor = if kind.rounds_by_half() { 2.0 } else { 6.0 };
    let sides = size.map(|size| (size.width(), size.height())).or_else(|| {
        fields
            .get(WIDTH)
            .and_then(Value::as_f64)
            .zip(fields.get(HEIGHT).and_then(Value::as_f64))
    });
    sides.map_or(u32::MAX, |(width, height)| {
        let rounded = (width.min(height) / divisor)
            .floor()
            .clamp(0.0, f64::from(u32::MAX));
        whole_u32(rounded).unwrap_or_default()
    })
}
