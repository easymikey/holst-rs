//! How an arrow is drawn: its line, its heads and the widths it offers.

use std::{fmt, str::FromStr};

use serde::Deserialize;

use crate::{
    domain::geometry::Point,
    error::{Error, Result, board_change::BoardChangeError},
};

/// The line of an arrow: what `arrowType` can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ArrowType {
    Straight,
    Elbow,
    Curved,
}

impl ArrowType {
    /// The name the document stores in `arrowType`.
    #[must_use]
    pub const fn stored_name(self) -> &'static str {
        match self {
            ArrowType::Straight => "straight",
            ArrowType::Elbow => "elbow",
            ArrowType::Curved => "curved",
        }
    }
}

/// BUNDLE: the thickness picker of an arrow (main.a33651a4.js 166235).
pub const ARROW_WIDTHS: [f64; 9] = [1.0, 1.5, 2.0, 4.0, 5.0, 6.0, 9.0, 13.0, 17.0];

/// BUNDLE: the 17 arrowhead names (main.a33651a4.js 167425-167450).
const HEADS: [&str; 17] = [
    "none",
    "figma",
    "miro",
    "arc",
    "triangle-filled",
    "triangle",
    "circle-filled",
    "circle",
    "rhombus-filled",
    "rhombus",
    "stick",
    "stick-stick",
    "triadius",
    "triadius-stick",
    "triadius-circle",
    "stick-circle",
    "slash",
];

const HEADS_EXPECTED: &str = "start,end heads, an empty one keeps that end: none, figma, miro, arc, triangle-filled, triangle, \
     circle-filled, circle, rhombus-filled, rhombus, stick, stick-stick, triadius, \
     triadius-stick, triadius-circle, stick-circle or slash";

/// One of the arrowheads the web client draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrowHead(&'static str);

impl ArrowHead {
    /// The name the document stores in `arrowheadStart` and `arrowheadEnd`.
    #[must_use]
    pub const fn stored_name(self) -> &'static str {
        self.0
    }
}

/// The heads an edit sets: `start,end`, an empty side leaves that end alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArrowHeads {
    pub start: Option<ArrowHead>,
    pub end: Option<ArrowHead>,
}

impl FromStr for ArrowHeads {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let bad = || {
            Error::BoardChange(BoardChangeError::NotArrowLook {
                raw: raw.into(),
                expected: HEADS_EXPECTED,
            })
        };
        let side = |name: &str| match name.trim() {
            "" => Ok(None),
            name => HEADS
                .iter()
                .find(|known| **known == name)
                .map(|known| Some(ArrowHead(known)))
                .ok_or_else(bad),
        };
        let (start, end) = raw.split_once(',').ok_or_else(bad)?;
        let heads = Self {
            start: side(start)?,
            end: side(end)?,
        };
        (heads.start.is_some() || heads.end.is_some())
            .then_some(heads)
            .ok_or_else(bad)
    }
}

const BEND_EXPECTED: &str =
    "a bend: x,y for a straight or curved arrow, x, or ,y for an elbow";

const END_EXPECTED: &str =
    "an arrow end: object:side (top, right, bottom or left) or x,y";

/// Where an arrow bends: a point, or for an elbow one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bend {
    pub x: Option<f64>,
    pub y: Option<f64>,
}

impl Bend {
    /// Whether the bend suits the line: an elbow takes one axis, the others a
    /// point.
    #[must_use]
    pub fn suits(self, line: ArrowType) -> bool {
        let given = usize::from(self.x.is_some()) + usize::from(self.y.is_some());
        given == if line == ArrowType::Elbow { 1 } else { 2 }
    }

    /// The error for a bend the line cannot take.
    #[must_use]
    pub fn unfit(self) -> Error {
        let raw = format!("{self}");
        Error::BoardChange(BoardChangeError::NotArrowLook {
            raw,
            expected: BEND_EXPECTED,
        })
    }
}

impl fmt::Display for Bend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let axis = |value: Option<f64>| value.map(|value| value.to_string());
        write!(
            f,
            "{},{}",
            axis(self.x).unwrap_or_default(),
            axis(self.y).unwrap_or_default()
        )
    }
}

impl FromStr for Bend {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let bad = || {
            Error::BoardChange(BoardChangeError::NotArrowLook {
                raw: raw.into(),
                expected: BEND_EXPECTED,
            })
        };
        let axis = |text: &str| match text.trim() {
            "" => Ok(None),
            number => number
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Some)
                .ok_or_else(bad),
        };
        let (x, y) = raw.split_once(',').ok_or_else(bad)?;
        let bend = Self {
            x: axis(x)?,
            y: axis(y)?,
        };
        (bend.x.is_some() || bend.y.is_some())
            .then_some(bend)
            .ok_or_else(bad)
    }
}

/// A side of a box an arrow end sticks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndSide {
    Top,
    Right,
    Bottom,
    Left,
}

impl EndSide {
    fn named(name: &str) -> Option<Self> {
        match name {
            "top" => Some(EndSide::Top),
            "right" => Some(EndSide::Right),
            "bottom" => Some(EndSide::Bottom),
            "left" => Some(EndSide::Left),
            _ => None,
        }
    }

    /// BUNDLE: the stored `control`, top 1 to left 4 (main.a33651a4.js
    /// 167422-167426).
    #[must_use]
    pub const fn control(self) -> u8 {
        match self {
            EndSide::Top => 1,
            EndSide::Right => 2,
            EndSide::Bottom => 3,
            EndSide::Left => 4,
        }
    }

    /// BUNDLE: the middle of the side in the box's own 0..1 (main.a33651a4.js
    /// 160333-160336).
    #[must_use]
    pub const fn relative_point(self) -> (f64, f64) {
        match self {
            EndSide::Top => (0.5, 0.0),
            EndSide::Right => (1.0, 0.5),
            EndSide::Bottom => (0.5, 1.0),
            EndSide::Left => (0.0, 0.5),
        }
    }
}

/// Where an edit puts one end of an arrow: on a side of an object, or free.
#[derive(Debug, Clone, PartialEq)]
pub enum ArrowEndEdit {
    Free(Point),
    Side { object: String, side: EndSide },
}

impl FromStr for ArrowEndEdit {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let bad = || {
            Error::BoardChange(BoardChangeError::NotArrowLook {
                raw: raw.into(),
                expected: END_EXPECTED,
            })
        };
        match raw.rsplit_once(':') {
            Some((object, side)) => {
                let side = EndSide::named(side.trim()).ok_or_else(bad)?;
                let object = object.trim();
                (!object.is_empty())
                    .then(|| ArrowEndEdit::Side {
                        object: object.to_owned(),
                        side,
                    })
                    .ok_or_else(bad)
            }
            None => raw
                .parse::<Point>()
                .map(ArrowEndEdit::Free)
                .map_err(|_| bad()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_elbow_bends_on_one_axis_and_the_others_at_a_point() {
        let along = "120,".parse::<Bend>().ok();
        assert_eq!(along.map(|bend| bend.suits(ArrowType::Elbow)), Some(true));
        assert_eq!(along.map(|bend| bend.suits(ArrowType::Curved)), Some(false));
        let at = "120,-80.5".parse::<Bend>().ok();
        assert_eq!(at.map(|bend| bend.suits(ArrowType::Straight)), Some(true));
        assert_eq!(at.map(|bend| bend.suits(ArrowType::Elbow)), Some(false));
    }

    #[test]
    fn a_bend_with_no_number_is_refused() {
        assert!(",".parse::<Bend>().is_err());
        assert!("12".parse::<Bend>().is_err());
        assert!("a,3".parse::<Bend>().is_err());
        assert!("inf,3".parse::<Bend>().is_err());
    }

    #[test]
    fn an_end_is_a_side_of_an_object_or_a_point() {
        let side = "@3:left".parse::<ArrowEndEdit>().ok();
        assert_eq!(
            side,
            Some(ArrowEndEdit::Side {
                object: "@3".into(),
                side: EndSide::Left
            })
        );
        let free = "10,20.5".parse::<ArrowEndEdit>().ok();
        assert_eq!(free, Point::new(10.0, 20.5).ok().map(ArrowEndEdit::Free));
    }

    #[test]
    fn an_end_without_a_known_side_or_point_is_refused() {
        assert!("@3".parse::<ArrowEndEdit>().is_err());
        assert!("@3:middle".parse::<ArrowEndEdit>().is_err());
        assert!(":top".parse::<ArrowEndEdit>().is_err());
    }

    #[test]
    fn heads_are_a_pair_and_an_empty_side_is_left_alone() {
        let pair = "none,triangle-filled".parse::<ArrowHeads>().ok();
        assert_eq!(
            pair.and_then(|heads| heads.end),
            Some(ArrowHead("triangle-filled"))
        );
        let kept = ",miro".parse::<ArrowHeads>().ok();
        assert_eq!(kept.map(|heads| heads.start), Some(None));
    }

    #[test]
    fn a_head_the_web_lacks_or_no_head_is_refused() {
        assert!("none,pointy".parse::<ArrowHeads>().is_err());
        assert!("miro".parse::<ArrowHeads>().is_err());
        assert!(",".parse::<ArrowHeads>().is_err());
    }
}
