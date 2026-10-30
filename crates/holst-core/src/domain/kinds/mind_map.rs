//! A mind-map node: a rich text in a pill, hung from its parent node.

use serde::Deserialize;
use serde_json::{Value, json};

use super::Detail;
use crate::{
    domain::{
        area::scene::{Scene, text_scale},
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Rectangle, Size},
        palette::Color,
    },
    error::Result,
    protocol::document::{COLOR, DOCUMENT_ID, OPACITY, STROKE_STYLE},
};

/// The stored type of a node.
pub(crate) const NODE: &str = "mind-map-node";

const NODE_TYPE: &str = "nodeType";
pub(crate) const NODE_COLOR: &str = "nodeColor";
pub(crate) const HIDE_CHILDREN: &str = "hideChildren";

/// The outline the web client gives a new node (`nodeType` PILL).
const PILL: i64 = 1;

/// The box the scene measures a node as, at text scale 1.
const DRAWN: (f64, f64) = (160.0, 100.0);

/// The room between a node and the children it hangs, at text scale 1.
const MAJOR_GAP: f64 = 100.0;

/// The room between the children of one node, at text scale 1.
const MINOR_GAP: f64 = 16.0;

/// The side of its parent a node goes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Direction {
    Left,
    #[default]
    Right,
    Up,
    Down,
}

/// The node a new one hangs from, with the board it is in.
#[derive(Debug)]
pub struct Under<'a> {
    pub scene: &'a Scene<'a>,
    pub parent: &'a Object,
    pub direction: Direction,
}

/// Whether the object is a node of a mind map.
#[must_use]
pub fn is_node(object: &Object) -> bool {
    object.kind.as_str() == NODE
}

/// A new node with `text`, as the web client makes one. A root has the corner
/// of the caller's to set; a child is placed beside its parent after the
/// children it has and takes the outline and color of the parent, `color`
/// aside.
///
/// # Errors
/// [`crate::error::input::InputError::NotFinite`] when the corner overflows.
pub fn made(
    text: &str,
    color: Option<&Color>,
    under: Option<&Under<'_>>,
) -> Result<Made> {
    let own = color.map(|color| json!({ (COLOR): color.as_str(), (OPACITY): 1 }));
    let stored = |parent: &Object, name: &'static str| {
        parent.fields.get(name).cloned().map(|value| (name, value))
    };
    let fields = match under {
        None => vec![(NODE_TYPE, json!(PILL))]
            .into_iter()
            .chain(own.map(|own| (NODE_COLOR, own)))
            .collect(),
        Some(Under { parent, .. }) => [
            Some(stored(parent, NODE_TYPE).unwrap_or((NODE_TYPE, json!(PILL)))),
            own.map(|own| (NODE_COLOR, own))
                .or_else(|| stored(parent, NODE_COLOR)),
            stored(parent, STROKE_STYLE),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    Ok(Made {
        kind: ObjectKind::of(NODE),
        at: under.map_or(Ok(Point::ORIGIN), corner)?,
        size: Size::new(DRAWN.0, DRAWN.1).ok(),
        parent: under.map(|under| under.parent.id.clone()),
        fields,
        texts: vec![(DOCUMENT_ID, text.to_owned())],
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    })
}

/// Where a child goes: `MAJOR_GAP` beyond the edge of its parent, and past the
/// children the parent has along that edge.
fn corner(under: &Under<'_>) -> Result<Point> {
    let Under {
        scene,
        parent,
        direction,
    } = under;
    let scale = text_scale(parent);
    let (width, height) = (DRAWN.0 * scale, DRAWN.1 * scale);
    let beside = scene
        .rectangle_of(parent)
        .unwrap_or_else(|| Rectangle::new(parent.x, parent.y, width, height));
    let earlier = scene
        .objects()
        .iter()
        .filter(|object| object.parent.as_deref() == Some(parent.id.as_str()))
        .count();
    let rank = f64::from(u32::try_from(earlier).unwrap_or(u32::MAX));
    let (major, minor) = (MAJOR_GAP * scale, MINOR_GAP * scale);
    match direction {
        Direction::Right => {
            Point::new(beside.right() + major, beside.y + rank * (height + minor))
        }
        Direction::Left => {
            Point::new(beside.x - major - width, beside.y + rank * (height + minor))
        }
        Direction::Down => {
            Point::new(beside.x + rank * (width + minor), beside.bottom() + major)
        }
        Direction::Up => {
            Point::new(beside.x + rank * (width + minor), beside.y - major - height)
        }
    }
}

/// The `node` outline, and `children=hidden` when the branch is collapsed.
pub(super) fn detail(node: &Object) -> Detail {
    let outline = match node.fields.get(NODE_TYPE).and_then(Value::as_i64) {
        Some(1) => "pill",
        Some(2) => "rectangle",
        Some(3) => "rounded-rectangle",
        _ => "plain",
    };
    let collapsed = node
        .fields
        .get(HIDE_CHILDREN)
        .and_then(Value::as_bool)
        .filter(|hidden| *hidden)
        .map(|_| ("children", json!("hidden")));
    Detail {
        text: None,
        words: std::iter::once(("node", json!(outline)))
            .chain(collapsed)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::Doc;

    use super::{Direction, Under, made};
    use crate::{
        Result,
        domain::{
            area::scene::Scene,
            board::{Object, object_of},
            kinds::detail,
        },
    };

    fn node(extra: &[(&str, Value)]) -> Option<Object> {
        let mut fields = json!({"id": "m1", "type": "mind-map-node",
            "position": {"x": 0, "y": 0}});
        fields.as_object_mut().into_iter().for_each(|map| {
            map.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
        });
        object_of(fields)
    }

    fn keys(made: &crate::domain::change::value::Made) -> Value {
        json!({
            "parent": made.parent,
            "texts": made.texts,
            "fields": made.fields.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
        })
    }

    #[test]
    fn a_root_has_a_pill_a_text_and_no_parent() -> Result<()> {
        let root = made("Plan", None, None)?;
        assert_eq!(
            keys(&root),
            json!({"parent": null, "texts": [["documentId", "Plan"]],
                   "fields": {"nodeType": 1}})
        );
        Ok(())
    }

    #[test]
    fn a_child_takes_its_parents_outline_and_color_unless_given_its_own() -> Result<()>
    {
        let stored = node(&[
            ("nodeType", json!(3)),
            ("nodeColor", json!({"color": "red4", "opacity": 0.2})),
            ("strokeStyle", json!("dashed")),
        ]);
        let objects: Vec<Object> = stored.into_iter().collect();
        let scene = Scene::new(&objects);
        let under = objects.first().map(|hung| Under {
            scene: &scene,
            parent: hung,
            direction: Direction::Right,
        });
        let inherited = made("a", None, under.as_ref())?;
        let colored = made("a", Some(&"blue4".parse()?), under.as_ref())?;
        assert_eq!(
            [keys(&inherited), keys(&colored)],
            [
                json!({"parent": "m1", "texts": [["documentId", "a"]], "fields": {
                    "nodeType": 3, "strokeStyle": "dashed",
                    "nodeColor": {"color": "red4", "opacity": 0.2}}}),
                json!({"parent": "m1", "texts": [["documentId", "a"]], "fields": {
                    "nodeType": 3, "strokeStyle": "dashed",
                    "nodeColor": {"color": "blue4", "opacity": 1}}}),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_node_lists_its_outline_and_a_collapsed_branch() {
        let words = |extra: &[(&str, Value)]| -> Vec<String> {
            node(extra)
                .iter()
                .flat_map(|object| detail(&Doc::new(), object).words)
                .map(|(word, value)| format!("{word}={value}"))
                .collect()
        };
        assert_eq!(
            [
                words(&[("nodeType", json!(1))]),
                words(&[("nodeType", json!(2)), ("hideChildren", json!(true))]),
                words(&[("nodeType", json!(3)), ("hideChildren", json!(false))]),
                words(&[("nodeType", Value::Null)]),
                words(&[]),
            ],
            [
                vec![r#"node="pill""#.to_owned()],
                vec![
                    r#"node="rectangle""#.to_owned(),
                    r#"children="hidden""#.to_owned()
                ],
                vec![r#"node="rounded-rectangle""#.to_owned()],
                vec![r#"node="plain""#.to_owned()],
                vec![r#"node="plain""#.to_owned()],
            ]
        );
    }
}
