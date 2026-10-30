//! A reaction stamp: one of eight pictures, loose or stuck to an object.

use serde_json::json;

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Rectangle, Size},
    },
    error::Result,
    protocol::document::{HEIGHT, PARENT_ID, ROTATION, WIDTH},
};

/// The stamps the web client offers, `like` first.
pub const KEYS: [&str; 8] = [
    "like",
    "heart",
    "star",
    "+1",
    "dislike",
    "figma-question",
    "check",
    "cross",
];

/// The stamp the web client places when none is picked.
pub const DEFAULT_KEY: &str = "like";

/// The stored type of a reaction stamp.
const STORED: &str = "reaction-stamp";

const STAMP_KEY: &str = "stampKey";
const STICKY_POSITION: &str = "stickyPosition";

/// What the web client places a stamp as, in board units.
const SIDE: f64 = 60.0;

/// The most a stamp is tilted either way, in degrees.
const TILT: f64 = 15.0;

/// The object a stamp sticks to: its box before rotation, and the rotation.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub id: &'a str,
    pub area: Rectangle,
    pub rotation: f64,
}

/// Whether the object is a reaction stamp.
#[must_use]
pub fn is_stamp(object: &Object) -> bool {
    object.kind.as_str() == STORED
}

/// A new loose stamp as the web client places one, tilted by the `draw` in
/// 0..1; the corner is the caller's to set.
#[must_use]
pub fn made(key: &str, draw: f64) -> Made {
    Made {
        kind: ObjectKind::Other(STORED.to_owned()),
        at: Point::ORIGIN,
        size: Size::new(SIDE, SIDE).ok(),
        parent: None,
        fields: vec![
            (STAMP_KEY, json!(key)),
            (ROTATION, json!(rounded(TILT * draw * 2.0 - TILT, 10.0))),
            (WIDTH, json!(SIDE)),
            (HEIGHT, json!(SIDE)),
        ],
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// `made` stuck to `target`: at `at` from the target's corner, else with its
/// middle on the target's middle.
///
/// # Errors
/// [`crate::error::input::InputError::NotFinite`] when the corner overflows.
pub fn stuck(mut made: Made, target: &Target<'_>, at: Option<Point>) -> Result<Made> {
    let Rectangle {
        x,
        y,
        width,
        height,
    } = target.area;
    let tilt = made
        .fields
        .iter()
        .find(|(name, _)| *name == ROTATION)
        .and_then(|(_, value)| value.as_f64())
        .unwrap_or_default();
    let (half_x, half_y) = turn(SIDE / 2.0, SIDE / 2.0, tilt - target.rotation);
    let (own_x, own_y) = at
        .map_or((width / 2.0 - half_x, height / 2.0 - half_y), |at| {
            (at.x(), at.y())
        });
    let (across, down) = turn(own_x, own_y, target.rotation);
    let (x_constraint, x_value) =
        constrained(own_x + half_x, own_x, width, ("left", "right"));
    let (y_constraint, y_value) =
        constrained(own_y + half_y, own_y, height, ("top", "bottom"));
    made.fields.push((
        STICKY_POSITION,
        json!({
            PARENT_ID: target.id,
            "constraints": {"x": x_constraint, "y": y_constraint},
            "x": x_value,
            "y": y_value,
        }),
    ));
    made.at = Point::new(rounded(x + across, FINE), rounded(y + down, FINE))?;
    Ok(made)
}

/// How the web client keeps a coordinate of the middle of the stamp against an
/// edge pair of its object: as a share of the side while inside, else as the
/// distance from the near edge.
fn constrained(
    middle: f64,
    corner: f64,
    side: f64,
    (near, far): (&'static str, &'static str),
) -> (&'static str, f64) {
    match middle {
        inside if (0.0..=side).contains(&inside) => {
            ("normalized", rounded(inside / side, FINE))
        }
        beyond if beyond.abs() <= (beyond - side).abs() => {
            (near, rounded(corner, FINE))
        }
        _ => (far, rounded(corner - side, FINE)),
    }
}

/// The 4 places the web client keeps of a position.
const FINE: f64 = 10_000.0;

/// `value` rounded half up to `1 / factor`, as `Math.round` does.
fn rounded(value: f64, factor: f64) -> f64 {
    (value * factor + 0.5).floor() / factor
}

/// The point turned clockwise by `degrees` about the origin.
fn turn(x: f64, y: f64, degrees: f64) -> (f64, f64) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    (x * cos - y * sin, x * sin + y * cos)
}

/// The `reaction` and what the stamp is stuck `on`, each when it is stored.
pub(super) fn detail(stamp: &Object) -> Detail {
    let word = |name, value: Option<&serde_json::Value>| {
        value
            .filter(|each| each.is_string())
            .map(|each| (name, each.clone()))
    };
    let sticky = stamp.fields.get(STICKY_POSITION);
    Detail {
        text: None,
        words: word("reaction", stamp.fields.get(STAMP_KEY))
            .into_iter()
            .chain(word("on", sticky.and_then(|sticky| sticky.get(PARENT_ID))))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::Doc;

    use super::{Target, is_stamp, made, stuck};
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            change::value::Made,
            geometry::{Point, Rectangle},
            kinds,
        },
    };

    fn field(made: &Made, name: &str) -> Option<Value> {
        made.fields
            .iter()
            .find(|(each, _)| *each == name)
            .map(|(_, value)| value.clone())
    }

    fn target(rotation: f64) -> Target<'static> {
        Target {
            id: "s1",
            area: Rectangle::new(100.0, 100.0, 200.0, 100.0),
            rotation,
        }
    }

    fn placed(made: &Made) -> (f64, f64) {
        (made.at.x(), made.at.y())
    }

    #[test]
    fn a_made_stamp_has_the_keys_and_size_of_the_web_client() {
        let stamp = made("star", 0.123);
        assert_eq!(
            (
                stamp.kind,
                stamp.fields,
                stamp.size.map(|size| (size.width(), size.height())),
                stamp.parent,
            ),
            (
                ObjectKind::Other("reaction-stamp".to_owned()),
                vec![
                    ("stampKey", json!("star")),
                    ("rotation", json!(-11.3)),
                    ("width", json!(60.0)),
                    ("height", json!(60.0)),
                ],
                Some((60.0, 60.0)),
                None,
            )
        );
    }

    #[test]
    fn the_rotation_spreads_the_draw_over_15_degrees_either_way() {
        let turn = |draw| field(&made("like", draw), "rotation");
        assert_eq!(
            [turn(0.0), turn(0.5), turn(0.9999)],
            [Some(json!(-15.0)), Some(json!(0.0)), Some(json!(15.0))]
        );
    }

    #[test]
    fn a_stamp_without_a_corner_sticks_with_its_middle_on_the_middle_of_its_object()
    -> Result<()> {
        let stamp = stuck(made("like", 0.5), &target(0.0), None)?;
        assert_eq!(
            (placed(&stamp), field(&stamp, "stickyPosition")),
            (
                (170.0, 120.0),
                Some(json!({
                    "parentId": "s1",
                    "constraints": {"x": "normalized", "y": "normalized"},
                    "x": 0.5,
                    "y": 0.5,
                }))
            )
        );
        Ok(())
    }

    #[test]
    fn a_stamp_with_a_corner_is_placed_from_the_corner_of_its_object() -> Result<()> {
        let stamp = stuck(
            made("like", 0.5),
            &target(0.0),
            Some(Point::new(10.0, 10.0)?),
        )?;
        assert_eq!(
            (placed(&stamp), field(&stamp, "stickyPosition")),
            (
                (110.0, 110.0),
                Some(json!({
                    "parentId": "s1",
                    "constraints": {"x": "normalized", "y": "normalized"},
                    "x": 0.2,
                    "y": 0.4,
                }))
            )
        );
        Ok(())
    }

    #[test]
    fn a_stamp_whose_middle_is_off_its_object_keeps_the_distance_to_the_near_edge()
    -> Result<()> {
        let sticky = |x: f64, y: f64| -> Result<Option<Value>> {
            let stamp =
                stuck(made("like", 0.5), &target(0.0), Some(Point::new(x, y)?))?;
            Ok(field(&stamp, "stickyPosition"))
        };
        assert_eq!(
            [sticky(-100.0, 20.0)?, sticky(190.0, 90.0)?],
            [
                Some(json!({
                    "parentId": "s1",
                    "constraints": {"x": "left", "y": "normalized"},
                    "x": -100.0,
                    "y": 0.5,
                })),
                Some(json!({
                    "parentId": "s1",
                    "constraints": {"x": "right", "y": "bottom"},
                    "x": -10.0,
                    "y": -10.0,
                })),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_turned_object_turns_the_place_of_its_stamp() -> Result<()> {
        let stamp = stuck(made("like", 0.5), &target(90.0), None)?;
        assert_eq!(
            (placed(&stamp), field(&stamp, "stickyPosition")),
            (
                (20.0, 170.0),
                Some(json!({
                    "parentId": "s1",
                    "constraints": {"x": "normalized", "y": "normalized"},
                    "x": 0.5,
                    "y": 0.5,
                }))
            )
        );
        Ok(())
    }

    fn listed(fields: &Value) -> Vec<(&'static str, Value)> {
        let mut stored = json!({"id": "r1", "type": "reaction-stamp",
            "position": {"x": 0, "y": 0}});
        stored
            .as_object_mut()
            .zip(fields.as_object())
            .into_iter()
            .for_each(|(stored, fields)| stored.extend(fields.clone()));
        object_of(stored)
            .iter()
            .flat_map(|object| kinds::detail(&Doc::new(), object).words)
            .collect()
    }

    #[test]
    fn a_stamp_lists_its_reaction_and_the_object_it_is_stuck_to() {
        let sticky = json!({"parentId": "s1", "constraints": {"x": "normalized",
            "y": "normalized"}, "x": 0.5, "y": 0.5});
        assert_eq!(
            [
                listed(&json!({"stampKey": "like", "stickyPosition": sticky})),
                listed(&json!({"stampKey": "star"})),
                listed(&json!({"stampKey": "star", "stickyPosition": null})),
            ],
            [
                vec![("reaction", json!("like")), ("on", json!("s1"))],
                vec![("reaction", json!("star"))],
                vec![("reaction", json!("star"))],
            ]
        );
    }

    #[test]
    fn only_a_reaction_stamp_is_one() {
        let kind = |name: &str| {
            object_of(json!({"id": "o", "type": name, "position": {"x": 0, "y": 0}}))
                .map(|object| is_stamp(&object))
        };
        assert_eq!(
            [kind("reaction-stamp"), kind("stamp"), kind("mind-map-node")],
            [Some(true), Some(false), Some(false)]
        );
    }
}
