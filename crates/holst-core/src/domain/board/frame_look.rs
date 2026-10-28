//! The look of a frame: ratio, background style, hidden content.

use serde::Deserialize;

use crate::{domain::geometry::Size, error::Result};

/// BUNDLE: the ratio presets and sizes (main.a33651a4.js 169925-171274).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
pub enum FrameRatio {
    #[serde(rename = "16:9")]
    #[strum(serialize = "16:9")]
    Wide,
    #[serde(rename = "4:3")]
    #[strum(serialize = "4:3")]
    Standard,
    #[serde(rename = "1:1")]
    #[strum(serialize = "1:1")]
    Square,
    #[serde(rename = "a4")]
    #[strum(serialize = "a4")]
    A4,
    #[serde(rename = "a4_landscape")]
    #[strum(serialize = "a4_landscape")]
    A4Landscape,
}

impl FrameRatio {
    const ALL: [FrameRatio; 5] = [
        FrameRatio::Wide,
        FrameRatio::Standard,
        FrameRatio::Square,
        FrameRatio::A4,
        FrameRatio::A4Landscape,
    ];

    /// The number the document stores in `ratio`.
    #[must_use]
    pub const fn stored(self) -> u8 {
        match self {
            FrameRatio::Wide => 1,
            FrameRatio::Standard => 2,
            FrameRatio::Square => 3,
            FrameRatio::A4 => 4,
            FrameRatio::A4Landscape => 5,
        }
    }

    /// The ratio a stored number names.
    #[must_use]
    pub fn of_stored(number: u64) -> Option<FrameRatio> {
        Self::ALL
            .into_iter()
            .find(|ratio| u64::from(ratio.stored()) == number)
    }

    /// The name as `object edit` words it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            FrameRatio::Wide => "16:9",
            FrameRatio::Standard => "4:3",
            FrameRatio::Square => "1:1",
            FrameRatio::A4 => "a4",
            FrameRatio::A4Landscape => "a4_landscape",
        }
    }

    const fn default_sides(self) -> (f64, f64) {
        match self {
            FrameRatio::Wide => (1920.0, 1080.0),
            FrameRatio::Standard => (1024.0, 768.0),
            FrameRatio::Square => (496.0, 496.0),
            FrameRatio::A4 => (595.0, 842.0),
            FrameRatio::A4Landscape => (842.0, 595.0),
        }
    }

    /// The size a new frame of this ratio has.
    ///
    /// # Errors
    /// Never in practice: the presets are positive.
    pub fn default_size(self) -> Result<Size> {
        let (width, height) = self.default_sides();
        Size::new(width, height)
    }

    /// The `width` by `height` frame reshaped to this ratio, area kept
    /// (171387).
    ///
    /// # Errors
    /// A side that is not a finite number above zero.
    pub fn fitted(self, width: f64, height: f64) -> Result<Size> {
        let (wide, high) = self.default_sides();
        let shape = wide / high;
        let side = (width * height * shape).sqrt();
        Size::new(side, side / shape)
    }
}

/// BUNDLE: `backgroundStyle` 1 fill, 2 border (default), 3 outline (171263).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FrameBackground {
    Fill,
    Border,
    Outline,
}

impl FrameBackground {
    /// The number the document stores in `backgroundStyle`.
    #[must_use]
    pub const fn stored(self) -> u8 {
        match self {
            FrameBackground::Fill => 1,
            FrameBackground::Border => 2,
            FrameBackground::Outline => 3,
        }
    }

    /// The name as `object edit` words it, for a stored number.
    #[must_use]
    pub const fn name_of_stored(number: u64) -> Option<&'static str> {
        match number {
            1 => Some("fill"),
            2 => Some("border"),
            3 => Some("outline"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use super::*;

    #[test]
    fn a_fitted_ratio_keeps_the_area() -> Result<()> {
        let size = FrameRatio::Wide.fitted(800.0, 600.0)?;
        assert!((size.width() * size.height() - 480_000.0).abs() < 1e-6);
        assert!((size.width() / size.height() - 16.0 / 9.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn a_stored_number_names_its_ratio() {
        assert_eq!(FrameRatio::of_stored(5), Some(FrameRatio::A4Landscape));
        assert_eq!(FrameRatio::of_stored(0), None);
    }
}
