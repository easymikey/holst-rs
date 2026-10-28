//! Arrow routes: sides, tips and the corners between two objects.

use serde_json::Value;

use crate::{
    domain::{
        area::scene::{Scene, number},
        board::{DrawnArrowEnds, Object, ObjectKind, arrow_look::ArrowType},
        geometry::{Direction, Point, Rectangle, StoredPoint},
    },
    protocol::document,
};

const ELBOW_STUB: f64 = 32.0;

/// A side of a box, as arrows attach to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectSide {
    Top,
    Right,
    Bottom,
    Left,
}

impl ObjectSide {
    /// The unit vector pointing away from the box.
    #[must_use]
    pub const fn normal(self) -> Direction {
        match self {
            ObjectSide::Top => Direction { dx: 0.0, dy: -1.0 },
            ObjectSide::Right => Direction { dx: 1.0, dy: 0.0 },
            ObjectSide::Bottom => Direction { dx: 0.0, dy: 1.0 },
            ObjectSide::Left => Direction { dx: -1.0, dy: 0.0 },
        }
    }

    fn of_control(control: u64) -> Option<ObjectSide> {
        match control {
            1 => Some(ObjectSide::Top),
            2 => Some(ObjectSide::Right),
            3 => Some(ObjectSide::Bottom),
            4 => Some(ObjectSide::Left),
            _ => None,
        }
    }

    fn middle(self, rectangle: &Rectangle) -> StoredPoint {
        let centre = rectangle.centre();
        match self {
            ObjectSide::Top => StoredPoint::new(centre.x, rectangle.y),
            ObjectSide::Right => StoredPoint::new(rectangle.right(), centre.y),
            ObjectSide::Bottom => StoredPoint::new(centre.x, rectangle.bottom()),
            ObjectSide::Left => StoredPoint::new(rectangle.x, centre.y),
        }
    }

    fn nearest(rectangle: &Rectangle, at: StoredPoint) -> ObjectSide {
        let distances = [
            (ObjectSide::Top, (at.y - rectangle.y).abs()),
            (ObjectSide::Right, (rectangle.right() - at.x).abs()),
            (ObjectSide::Bottom, (rectangle.bottom() - at.y).abs()),
            (ObjectSide::Left, (at.x - rectangle.x).abs()),
        ];
        distances
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(ObjectSide::Right, |(side, _)| side)
    }

    const ALL: [ObjectSide; 4] = [
        ObjectSide::Top,
        ObjectSide::Right,
        ObjectSide::Bottom,
        ObjectSide::Left,
    ];
}

/// One end of an arrow: where it touches and which way it leaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowEnd {
    pub at: StoredPoint,
    pub side: Option<ObjectSide>,
}

/// An arrow's path: its ends and the points between.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrowRoute {
    pub line: ArrowType,
    pub points: Vec<StoredPoint>,
    pub start: ArrowEnd,
    pub end: ArrowEnd,
}

impl Scene<'_> {
    /// Where an arrow runs, from its stored ends and the objects they hold;
    /// `None` when an end names an object that is gone or has no box.
    #[must_use]
    pub fn route(&self, arrow: &Object) -> Option<ArrowRoute> {
        let fields = &arrow.fields;
        let start = fields.get(document::START)?;
        let end = fields.get(document::END)?;
        // PROTOCOL: arrow line names.
        let line = ArrowType::of_stored(
            fields.get(document::ARROW_TYPE).and_then(Value::as_str),
        );
        let (start_box, end_box) = match (self.end_hold(start), self.end_hold(end)) {
            (ArrowEndHold::Missing, _) | (_, ArrowEndHold::Missing) => None,
            (start, end) => Some((start.rectangle(), end.rectangle())),
        }?;
        let RoutedEnds { start, end } = routed_ends(start, start_box, end, end_box);
        let points = match line {
            ArrowType::Elbow => elbow(start, end),
            ArrowType::Straight | ArrowType::Curved => vec![start.at, end.at],
        };
        Some(ArrowRoute {
            line,
            points,
            start,
            end,
        })
    }

    /// An arrow's ends as [`Scene::route`] runs, else as kept; others `None`.
    #[must_use]
    pub fn drawn_ends(&self, object: &Object) -> Option<DrawnArrowEnds> {
        let point = |end: ArrowEnd| Point::new(end.at.x, end.at.y).ok();
        let routed = || {
            self.route(object).and_then(|route| {
                point(route.start)
                    .zip(point(route.end))
                    .map(|(start, end)| DrawnArrowEnds { start, end })
            })
        };
        let kept = || object.ends.as_ref().and_then(|ends| ends.drawn);
        if object.kind == ObjectKind::Arrow {
            routed().or_else(kept)
        } else {
            None
        }
    }

    /// Where a listing puts an object: an arrow where its route starts.
    #[must_use]
    pub fn point(&self, object: &Object) -> Point {
        self.drawn_ends(object).map_or_else(
            || Point::new(object.x, object.y).unwrap_or(Point::ORIGIN),
            |ends| ends.start,
        )
    }

    fn end_hold(&self, end: &Value) -> ArrowEndHold {
        let id = end
            .get(document::OBJECT_ID)
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        id.map_or(ArrowEndHold::Free, |id| {
            self.get(id)
                .filter(|object| object.kind != ObjectKind::Arrow)
                .and_then(|object| self.rectangle_of(object))
                .map_or(ArrowEndHold::Missing, ArrowEndHold::On)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArrowEndHold {
    Free,
    On(Rectangle),
    Missing,
}

impl ArrowEndHold {
    const fn rectangle(self) -> Option<Rectangle> {
        match self {
            ArrowEndHold::On(rectangle) => Some(rectangle),
            ArrowEndHold::Free | ArrowEndHold::Missing => None,
        }
    }
}

fn stored_point(end: &Value) -> StoredPoint {
    let point = end.get(document::POINT);
    let coordinate = |axis| point.and_then(|p| number(p, axis)).unwrap_or_default();
    StoredPoint::new(coordinate(document::X), coordinate(document::Y))
}

/// Both ends of an arrow as drawn.
struct RoutedEnds {
    start: ArrowEnd,
    end: ArrowEnd,
}

fn routed_ends(
    start: &Value,
    start_box: Option<Rectangle>,
    end: &Value,
    end_box: Option<Rectangle>,
) -> RoutedEnds {
    let fixed = |stored: &Value, rectangle: Option<Rectangle>| {
        let rectangle = rectangle?;
        let side = stored
            .get(document::CONTROL)
            .and_then(Value::as_u64)
            .and_then(ObjectSide::of_control);
        let relative = stored.get(document::RELATIVE_POINT).map(|point| {
            let at = |axis, from: f64, size: f64| {
                from + number(point, axis).unwrap_or(0.5) * size
            };
            StoredPoint::new(
                at(document::X, rectangle.x, rectangle.width),
                at(document::Y, rectangle.y, rectangle.height),
            )
        });
        match (side, relative) {
            (Some(side), _) => Some(ArrowEnd {
                at: side.middle(&rectangle),
                side: Some(side),
            }),
            (None, Some(at)) => Some(ArrowEnd {
                at,
                side: Some(ObjectSide::nearest(&rectangle, at)),
            }),
            (None, None) => None,
        }
    };
    let loose = |stored: &Value, rectangle: Option<Rectangle>, other: StoredPoint| {
        fixed(stored, rectangle).unwrap_or_else(|| {
            rectangle.map_or(
                ArrowEnd {
                    at: stored_point(stored),
                    side: None,
                },
                |rectangle| closest_side(&rectangle, other),
            )
        })
    };
    let hint = |stored: &Value, rectangle: Option<Rectangle>| {
        fixed(stored, rectangle).map_or_else(
            || {
                rectangle.map_or_else(
                    || stored_point(stored),
                    |rectangle| rectangle.centre(),
                )
            },
            |fixed| fixed.at,
        )
    };
    RoutedEnds {
        start: loose(start, start_box, hint(end, end_box)),
        end: loose(end, end_box, hint(start, start_box)),
    }
}

fn closest_side(rectangle: &Rectangle, towards: StoredPoint) -> ArrowEnd {
    ObjectSide::ALL
        .into_iter()
        .map(|side| (side, side.middle(rectangle)))
        .min_by(|a, b| {
            a.1.distance_to(towards)
                .total_cmp(&b.1.distance_to(towards))
        })
        .map_or(
            ArrowEnd {
                at: rectangle.centre(),
                side: None,
            },
            |(side, at)| ArrowEnd {
                at,
                side: Some(side),
            },
        )
}

fn elbow(start: ArrowEnd, end: ArrowEnd) -> Vec<StoredPoint> {
    let towards = |from: StoredPoint, to: StoredPoint| {
        let horizontal = (to.x - from.x).abs() >= (to.y - from.y).abs();
        match (horizontal, to.x >= from.x, to.y >= from.y) {
            (true, true, _) => ObjectSide::Right,
            (true, false, _) => ObjectSide::Left,
            (false, _, true) => ObjectSide::Bottom,
            (false, _, false) => ObjectSide::Top,
        }
    };
    let start_out = start.side.unwrap_or_else(|| towards(start.at, end.at));
    let end_out = end.side.unwrap_or_else(|| towards(end.at, start.at));
    let stub = |arrow_end: ArrowEnd, side: ObjectSide| {
        let Direction { dx, dy } = side.normal();
        let length = arrow_end.side.map_or(0.0, |_| ELBOW_STUB);
        StoredPoint::new(arrow_end.at.x + dx * length, arrow_end.at.y + dy * length)
    };
    let a = stub(start, start_out);
    let b = stub(end, end_out);
    let flat = |side: ObjectSide| matches!(side, ObjectSide::Left | ObjectSide::Right);
    let middle = match (flat(start_out), flat(end_out)) {
        (true, true) => {
            let mx = f64::midpoint(a.x, b.x);
            vec![StoredPoint::new(mx, a.y), StoredPoint::new(mx, b.y)]
        }
        (false, false) => {
            let my = f64::midpoint(a.y, b.y);
            vec![StoredPoint::new(a.x, my), StoredPoint::new(b.x, my)]
        }
        (true, false) => vec![StoredPoint::new(b.x, a.y)],
        (false, true) => vec![StoredPoint::new(a.x, b.y)],
    };
    let all: Vec<StoredPoint> = [start.at, a]
        .into_iter()
        .chain(middle)
        .chain([b, end.at])
        .collect();
    simplified(&all)
}

fn simplified(points: &[StoredPoint]) -> Vec<StoredPoint> {
    let near = |a: StoredPoint, b: StoredPoint| {
        (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01
    };
    let unique =
        points
            .iter()
            .copied()
            .fold(Vec::new(), |mut kept: Vec<StoredPoint>, p| {
                let repeat = kept.last().is_some_and(|last| near(*last, p));
                kept.extend((!repeat).then_some(p));
                kept
            });
    let straight = |a: StoredPoint, b: StoredPoint, c: StoredPoint| {
        ((b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x)).abs() < 0.01
    };
    let inner = unique.windows(3).filter_map(|w| match w {
        [a, b, c] => (!straight(*a, *b, *c)).then_some(*b),
        _ => None,
    });
    unique
        .first()
        .copied()
        .into_iter()
        .chain(inner)
        .chain(unique.last().copied().filter(|_| unique.len() > 1))
        .collect()
}
