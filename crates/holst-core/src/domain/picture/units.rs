//! The numbers a picture request carries, each in its own unit and in range.

use std::{
    fmt,
    ops::{Bound, RangeBounds},
};

use serde::Serialize;

use crate::{
    domain::geometry::whole_u32,
    error::{Error, Result, picture::PictureError},
};

/// The longest side a picture may have, in pixels.
pub(super) const MAX_SIDE: f64 = 16_384.0;

/// The most pixels a picture may have in all.
pub(super) const MAX_PIXELS: f64 = 4096.0 * 4096.0;

/// The longest side of a picture written to a file.
pub const MAX_SIDE_FILE: PixelSide = PixelSide(8192.0);

/// The longest side of a picture shown to a model.
pub const MAX_SIDE_MODEL: PixelSide = PixelSide(1568.0);

const MIN_SIDE: f64 = 16.0;
const MAX_SCALE: f64 = 100.0;
const MAX_PADDING: f64 = 100_000.0;

fn within(
    name: &'static str,
    value: f64,
    range: impl RangeBounds<f64>,
    expected: &'static str,
) -> Result<f64> {
    range
        .contains(&value)
        .then_some(value)
        .ok_or(Error::Picture(PictureError::OutOfRange {
            name,
            value,
            expected,
        }))
}

/// Pixels drawn for one board unit: above 0, at most 100.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PixelsPerUnit(f64);

impl PixelsPerUnit {
    /// One pixel per board unit.
    pub const ONE: PixelsPerUnit = PixelsPerUnit(1.0);

    /// Returns `value` when it is above 0 and at most 100.
    ///
    /// # Errors
    /// [`PictureError::OutOfRange`] naming the scale.
    pub fn new(value: f64) -> Result<Self> {
        within(
            "scale",
            value,
            (Bound::Excluded(0.0), Bound::Included(MAX_SCALE)),
            "above 0, at most 100 pixels per board unit",
        )
        .map(Self)
    }

    /// Returns the number of pixels.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Returns the scale lowered to `limit` when that is above 0 and lower.
    #[must_use]
    pub fn lowered_to(self, limit: f64) -> Self {
        if limit > 0.0 {
            Self(self.0.min(limit))
        } else {
            self
        }
    }

    /// Returns the pixel count of `length` board units: rounded, 1 at least.
    #[must_use]
    pub fn pixels(self, length: f64) -> u32 {
        let clamped = (length * self.0).round().clamp(1.0, MAX_SIDE);
        whole_u32(clamped).unwrap_or_default()
    }
}

impl fmt::Display for PixelsPerUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The longest side of a picture in pixels: 16 to 16384.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct PixelSide(f64);

impl PixelSide {
    /// Returns `value` when it is 16 to 16384.
    ///
    /// # Errors
    /// [`PictureError::OutOfRange`] naming the max side.
    pub fn new(value: f64) -> Result<Self> {
        within("max side", value, MIN_SIDE..=MAX_SIDE, "16 to 16384 pixels").map(Self)
    }

    /// Returns the number of pixels.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// A margin in board units: 0 to 100000.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct BoardLength(f64);

impl BoardLength {
    /// Returns `value` when it is 0 to 100000.
    ///
    /// # Errors
    /// [`PictureError::OutOfRange`] naming the padding.
    pub fn new(value: f64) -> Result<Self> {
        within(
            "padding",
            value,
            0.0..=MAX_PADDING,
            "0 to 100000 board units",
        )
        .map(Self)
    }

    /// Returns the margin when given; `None` stays the selection's default.
    ///
    /// # Errors
    /// As [`BoardLength::new`].
    pub fn optional(value: Option<f64>) -> Result<Option<Self>> {
        value.map(Self::new).transpose()
    }

    /// Returns the number of board units.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::{
        domain::picture::units::{BoardLength, PixelSide, PixelsPerUnit},
        error::{Error, picture::PictureError},
    };

    #[rstest]
    #[case(0.0)]
    #[case(-1.0)]
    #[case(100.5)]
    #[case(f64::NAN)]
    #[case(f64::INFINITY)]
    fn a_scale_out_of_range_is_refused(#[case] value: f64) {
        assert!(matches!(
            PixelsPerUnit::new(value),
            Err(Error::Picture(PictureError::OutOfRange {
                name: "scale",
                ..
            }))
        ));
    }

    #[rstest]
    #[case(0.01)]
    #[case(1.0)]
    #[case(100.0)]
    fn a_scale_in_range_is_kept(#[case] value: f64) {
        assert_eq!(
            PixelsPerUnit::new(value).map(PixelsPerUnit::get).ok(),
            Some(value)
        );
    }

    #[rstest]
    #[case(15.0)]
    #[case(16_385.0)]
    #[case(f64::NAN)]
    fn a_side_out_of_range_is_refused(#[case] value: f64) {
        assert!(matches!(
            PixelSide::new(value),
            Err(Error::Picture(PictureError::OutOfRange {
                name: "max side",
                ..
            }))
        ));
    }

    #[rstest]
    #[case(16.0)]
    #[case(1568.0)]
    #[case(16_384.0)]
    fn a_side_in_range_is_kept(#[case] value: f64) {
        assert_eq!(PixelSide::new(value).map(PixelSide::get).ok(), Some(value));
    }

    #[rstest]
    #[case(-0.5)]
    #[case(100_001.0)]
    #[case(f64::INFINITY)]
    #[case(f64::NAN)]
    fn a_margin_out_of_range_is_refused(#[case] value: f64) {
        assert!(matches!(
            BoardLength::new(value),
            Err(Error::Picture(PictureError::OutOfRange {
                name: "padding",
                ..
            }))
        ));
    }

    #[rstest]
    #[case(0.0)]
    #[case(100_000.0)]
    fn a_margin_in_range_is_kept(#[case] value: f64) {
        assert_eq!(
            BoardLength::new(value).map(BoardLength::get).ok(),
            Some(value)
        );
    }

    #[test]
    fn no_margin_stays_none() {
        assert_eq!(BoardLength::optional(None).ok(), Some(None));
    }

    #[test]
    fn a_scale_is_only_ever_lowered_to_a_positive_limit() {
        let scale = PixelsPerUnit::ONE;
        let lowered = |limit: f64| (scale.lowered_to(limit).get() - 1.0).abs() < 1e-12;
        assert!((scale.lowered_to(0.5).get() - 0.5).abs() < 1e-12);
        assert!(lowered(4.0) && lowered(0.0) && lowered(f64::NAN));
    }

    #[test]
    fn a_length_becomes_whole_pixels_between_one_and_the_longest_side() {
        let scale = PixelsPerUnit::ONE;
        assert_eq!(scale.pixels(0.2), 1);
        assert_eq!(scale.pixels(99.6), 100);
        assert_eq!(scale.pixels(1e9), 16_384);
    }
}
