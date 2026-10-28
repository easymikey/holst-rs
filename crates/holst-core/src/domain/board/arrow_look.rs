//! How an arrow is drawn: its line, its heads and the widths it offers.

use std::str::FromStr;

use serde::Deserialize;

use crate::error::{Error, Result, board_change::BoardChangeError};

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

#[cfg(test)]
mod tests {
    use super::*;

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
