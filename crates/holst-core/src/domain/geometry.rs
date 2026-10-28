//! Rectangles in board coordinates: areas, bounds and overlaps; no I/O.

use std::{fmt, str::FromStr};

use serde::Serialize;

use crate::error::{
    Error,
    Result,
    board_change::BoardChangeError,
    check_all::both,
    input::InputError,
    picture::PictureError,
};

/// A finite number: never NaN or an infinity.
///
/// ```
/// use holst_core::domain::geometry::Finite;
///
/// assert_eq!(Finite::new(-40.5).map(Finite::get).ok(), Some(-40.5));
/// assert!(Finite::new(f64::NAN).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Finite(f64);

impl Finite {
    pub const ZERO: Finite = Finite(0.0);

    /// Returns `value` when it is finite.
    ///
    /// # Errors
    /// [`InputError::NotFinite`] for NaN or an infinity.
    pub fn new(value: f64) -> Result<Finite> {
        if value.is_finite() {
            Ok(Finite(value))
        } else {
            Err(Error::Input(InputError::NotFinite { value }))
        }
    }

    /// Returns the number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl From<i32> for Finite {
    fn from(number: i32) -> Finite {
        Finite(f64::from(number))
    }
}

/// A point on the board, in board coordinates; both are finite.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Point {
    x: Finite,
    y: Finite,
}

impl Point {
    pub const ORIGIN: Point = Point {
        x: Finite::ZERO,
        y: Finite::ZERO,
    };

    /// Returns the point at `x`, `y`.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotFinite`] for NaN or an infinity,
    /// [`crate::error::input::InputError::Several`] when both are.
    pub fn new(x: f64, y: f64) -> Result<Point> {
        both(Finite::new(x), Finite::new(y)).map(|(x, y)| Point { x, y })
    }

    /// Returns the point at whole coordinates, which are always finite.
    #[must_use]
    pub fn whole(x: i32, y: i32) -> Point {
        Point {
            x: Finite::from(x),
            y: Finite::from(y),
        }
    }

    /// Returns the left edge.
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x.get()
    }

    /// Returns the top edge.
    #[must_use]
    pub const fn y(self) -> f64 {
        self.y.get()
    }

    /// Returns the point moved by `dx`, `dy`.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotFinite`] when the sum overflows.
    pub fn shifted(self, dx: f64, dy: f64) -> Result<Point> {
        Point::new(self.x() + dx, self.y() + dy)
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

impl fmt::Display for Finite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A finite number above zero, such as a width or a height.
///
/// ```
/// use holst_core::domain::geometry::Positive;
///
/// assert_eq!(Positive::new(192.0).map(Positive::get).ok(), Some(192.0));
/// assert!(Positive::new(0.0).is_err());
/// assert!(Positive::new(f64::INFINITY).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Positive(f64);

impl Positive {
    /// Returns `value` when it is finite and above zero.
    ///
    /// # Errors
    /// [`InputError::NotPositive`] for zero, a negative number, NaN or an
    /// infinity.
    pub fn new(value: f64) -> Result<Positive> {
        if value.is_finite() && value > 0.0 {
            Ok(Positive(value))
        } else {
            Err(Error::Input(InputError::NotPositive { value }))
        }
    }

    pub(crate) const fn literal(value: f64) -> Positive {
        Positive(value)
    }

    /// Returns the number.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl fmt::Display for Positive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// An axis-aligned rectangle; width and height are never negative.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rectangle {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rectangle {
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }

    /// The right edge.
    #[must_use]
    pub const fn right(&self) -> f64 {
        self.x + self.width
    }

    /// The bottom edge.
    #[must_use]
    pub const fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// The center point.
    #[must_use]
    pub const fn centre(&self) -> StoredPoint {
        StoredPoint::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Whether the two share any point; touching edges count.
    #[must_use]
    pub const fn intersects(&self, other: &Rectangle) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    /// Whether `other` lies wholly inside.
    #[must_use]
    pub const fn contains(&self, other: &Rectangle) -> bool {
        self.x <= other.x
            && self.y <= other.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// The smallest rectangle holding both.
    #[must_use]
    pub const fn union(self, other: Rectangle) -> Rectangle {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rectangle::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    /// Grown by `by` on every side.
    #[must_use]
    pub const fn padded(self, by: f64) -> Rectangle {
        Rectangle::new(
            self.x - by,
            self.y - by,
            self.width + 2.0 * by,
            self.height + 2.0 * by,
        )
    }

    /// The smallest rectangle holding every point; `None` without points.
    #[must_use]
    pub fn around(points: impl IntoIterator<Item = StoredPoint>) -> Option<Rectangle> {
        Rectangle::bounding(
            points
                .into_iter()
                .map(|spot| Rectangle::new(spot.x, spot.y, 0.0, 0.0)),
        )
    }

    /// The smallest rectangle holding every rectangle; `None` without any.
    #[must_use]
    pub fn bounding(rects: impl IntoIterator<Item = Rectangle>) -> Option<Rectangle> {
        rects.into_iter().reduce(Rectangle::union)
    }

    /// The rectangle when every number is finite and within [`MAX_COORDINATE`];
    /// `None` for broken data.
    #[must_use]
    pub fn sane(self) -> Option<Rectangle> {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite() && n.abs() <= MAX_COORDINATE)
            .then_some(self)
    }
}

/// A point in board coordinates, as stored: not yet known to be finite.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StoredPoint {
    pub x: f64,
    pub y: f64,
}

impl StoredPoint {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns the straight distance to `other`.
    #[must_use]
    pub fn distance_to(self, other: StoredPoint) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// A width and a height in board units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredSize {
    pub width: f64,
    pub height: f64,
}

impl StoredSize {
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// A unit vector: which way a line leaves or an arrowhead points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Direction {
    pub dx: f64,
    pub dy: f64,
}

impl Direction {
    /// The opposite direction.
    #[must_use]
    pub const fn reversed(self) -> Direction {
        Direction {
            dx: -self.dx,
            dy: -self.dy,
        }
    }
}

pub const MAX_COORDINATE: f64 = 1e12;

impl FromStr for Rectangle {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let numbers: Vec<f64> = raw
            .split(',')
            .map(|part| part.trim().parse::<f64>().ok())
            .collect::<Option<_>>()
            .unwrap_or_default();
        let corner = |x: f64, y: f64| Finite::new(x).ok().zip(Finite::new(y).ok());
        let sides = |width: f64, height: f64| {
            Positive::new(width).ok().zip(Positive::new(height).ok())
        };
        let rectangle = match numbers.as_slice() {
            [x, y, width, height] => corner(*x, *y)
                .zip(sides(*width, *height))
                .and_then(|((x, y), (width, height))| {
                    Rectangle::new(x.get(), y.get(), width.get(), height.get()).sane()
                }),
            _ => None,
        };
        rectangle
            .ok_or_else(|| Error::Picture(PictureError::NotArea { raw: raw.into() }))
    }
}

impl fmt::Display for Rectangle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

impl FromStr for Point {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let bad = || Error::BoardChange(BoardChangeError::NotPoint { raw: raw.into() });
        let (x, y) = raw.split_once(',').ok_or_else(bad)?;
        let (x, y) = number(x).zip(number(y)).ok_or_else(bad)?;
        Point::new(x, y)
    }
}

/// A width and a height, both positive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Size {
    width: Positive,
    height: Positive,
}

impl Size {
    pub const STICKER: Size = Size::literal(192.0, 192.0);
    pub const SHAPE: Size = Size::literal(200.0, 120.0);
    pub const FRAME: Size = Size::literal(800.0, 600.0);
    pub const CARD: Size = Size::literal(320.0, 64.0);

    /// Returns a size of `width` by `height`.
    ///
    /// # Errors
    /// [`crate::error::input::InputError::NotPositive`] for a side that is not
    /// a finite number above zero,
    /// [`crate::error::input::InputError::Several`] when both are not.
    pub fn new(width: f64, height: f64) -> Result<Size> {
        both(Positive::new(width), Positive::new(height))
            .map(|(width, height)| Size { width, height })
    }

    const fn literal(width: f64, height: f64) -> Size {
        Size {
            width: Positive::literal(width),
            height: Positive::literal(height),
        }
    }

    /// Returns the width.
    #[must_use]
    pub const fn width(self) -> f64 {
        self.width.get()
    }

    /// Returns the height.
    #[must_use]
    pub const fn height(self) -> f64 {
        self.height.get()
    }
}

impl FromStr for Size {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self> {
        let bad = || Error::BoardChange(BoardChangeError::NotSize { raw: raw.into() });
        let (width, height) = raw.split_once(['x', 'X', '×']).ok_or_else(bad)?;
        let (width, height) = number(width).zip(number(height)).ok_or_else(bad)?;
        Size::new(width, height)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

fn number(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok()
}

const MANTISSA_BITS: u32 = 52;
const EXPONENT_BIAS: u32 = 1023;
const EXPONENT_MASK: u64 = 0x7ff;
const IMPLICIT_ONE: u64 = 1 << MANTISSA_BITS;

/// Whether a rounded `value` is negative and its magnitude, read from its bits
/// (no cast, no text); `None` for NaN, infinity or a magnitude beyond `u64`.
fn whole_parts(value: f64) -> Option<(bool, u64)> {
    let bits = value.to_bits();
    let exponent = u32::try_from((bits >> MANTISSA_BITS) & EXPONENT_MASK).ok()?;
    let magnitude = match exponent.checked_sub(EXPONENT_BIAS) {
        // Below one a rounded value is zero.
        None => 0,
        Some(power) => ((bits & (IMPLICIT_ONE - 1)) | IMPLICIT_ONE)
            .checked_shr(MANTISSA_BITS.checked_sub(power)?)?,
    };
    Some((bits >> 63 != 0, magnitude))
}

/// The `i32` a rounded `value` equals; `None` for NaN, infinity or a
/// magnitude beyond `i32`.
pub(crate) fn whole_i32(value: f64) -> Option<i32> {
    let (negative, magnitude) = whole_parts(value)?;
    match (negative, i32::try_from(magnitude).ok()?) {
        (true, magnitude) => magnitude.checked_neg(),
        (false, magnitude) => Some(magnitude),
    }
}

/// The `u32` a rounded `value` equals; `None` for NaN, infinity, a magnitude
/// beyond `u32` or a negative value.
pub(crate) fn whole_u32(value: f64) -> Option<u32> {
    let (negative, magnitude) = whole_parts(value)?;
    u32::try_from(magnitude)
        .ok()
        .filter(|&magnitude| !negative || magnitude == 0)
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::geometry::{
            Finite,
            Positive,
            Rectangle,
            StoredPoint,
            whole_i32,
            whole_u32,
        },
        error::{Error, Result, check_all::both, hint::Surface, input::InputError},
    };

    #[test]
    fn area_parses_four_numbers_with_positive_size() {
        assert_eq!(
            "-10, 20,300,40.5".parse::<Rectangle>().ok(),
            Some(Rectangle::new(-10.0, 20.0, 300.0, 40.5))
        );
        let bad = ["1,2,3", "1,2,0,4", "1,2,3,-4", "a,b,c,d", "1,2,3,4,5", ""];
        assert!(bad.iter().all(|raw| raw.parse::<Rectangle>().is_err()));
    }

    #[test]
    fn absurd_numbers_are_not_a_sane_rectangle() {
        assert!("1e300,0,1e300,10".parse::<Rectangle>().is_err());
        assert_eq!(Rectangle::new(0.0, 0.0, f64::INFINITY, 1.0).sane(), None);
        assert_eq!(Rectangle::new(f64::NAN, 0.0, 1.0, 1.0).sane(), None);
        let far = Rectangle::new(1e9, 1e9, 192.0, 192.0);
        assert_eq!(far.sane(), Some(far));
    }

    #[test]
    fn overlap_union_and_padding() {
        let a = Rectangle::new(0.0, 0.0, 10.0, 10.0);
        let b = Rectangle::new(10.0, 5.0, 5.0, 20.0);
        assert!(a.intersects(&b), "touching edges overlap");
        assert!(!a.intersects(&Rectangle::new(11.0, 0.0, 1.0, 1.0)));
        assert_eq!(a.union(b), Rectangle::new(0.0, 0.0, 15.0, 25.0));
        assert_eq!(a.padded(2.0), Rectangle::new(-2.0, -2.0, 14.0, 14.0));
        assert!(a.padded(1.0).contains(&a));
        assert_eq!(
            Rectangle::around([
                StoredPoint::new(3.0, 4.0),
                StoredPoint::new(-1.0, 9.0)
            ]),
            Some(Rectangle::new(-1.0, 4.0, 4.0, 5.0))
        );
        assert_eq!(Rectangle::bounding([]), None);
    }

    fn words(result: Result<impl Sized>) -> Option<(String, String)> {
        result
            .err()
            .map(|err| (err.on(Surface::Cli), err.on(Surface::Mcp)))
    }

    #[test]
    fn validated_one_failure_keeps_its_message() {
        let alone = words(Positive::new(0.0));
        let combined =
            both(both(Finite::new(1.0), Positive::new(0.0)), Finite::new(2.0));
        assert!(
            matches!(combined, Err(Error::Input(InputError::NotPositive { .. }))),
            "{combined:?}"
        );
        assert_eq!(words(combined), alone);
    }

    #[test]
    fn validated_failures_are_listed_flat() {
        let inner = both(Finite::new(f64::NAN), Finite::new(f64::INFINITY));
        let outer = both(inner, Positive::new(-1.0));
        let cli = "3 problems:\n  - NaN is not a coordinate: expected a finite number, e.g. -40.5\
             \n  - inf is not a coordinate: expected a finite number, e.g. -40.5\
             \n  - -1 is not a positive number: sides are above 0, e.g. 192";
        let mcp = "3 problems: NaN is not a coordinate: expected a finite number, e.g. -40.5; \
             inf is not a coordinate: expected a finite number, e.g. -40.5; \
             -1 is not a positive number: sides are above 0, e.g. 192";
        assert_eq!(words(outer), Some((cli.into(), mcp.into())));
    }

    #[test]
    fn validated_successes_come_back_in_order() {
        let values = both(Finite::new(1.0), Positive::new(2.0))
            .map(|(first, second)| (first.get(), second.get()));
        assert_eq!(values.ok(), Some((1.0, 2.0)));
    }

    #[test]
    fn a_whole_float_becomes_an_integer_or_nothing() {
        assert_eq!(whole_u32(3.0), Some(3));
        assert_eq!(whole_u32(-0.0), Some(0));
        assert_eq!(whole_u32(f64::from(u32::MAX)), Some(u32::MAX));
        assert_eq!(whole_u32(f64::from(u32::MAX) + 1.0), None);
        assert_eq!(whole_u32(-3.0), None);
        assert_eq!(whole_u32(1e21), None);
        assert_eq!(whole_i32(-3.0), Some(-3));
        assert_eq!(whole_i32(f64::NAN), None);
    }
}
