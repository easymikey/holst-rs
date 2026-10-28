//! Areas of a board and the pictures drawn of them.

use crate::error::hint::Hint;

/// An area that cannot be chosen, or a picture that cannot be drawn.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum PictureError {
    #[error("\"{raw}\" is not an area: expected x,y,width,height, e.g. 0,0,800,600")]
    NotArea { raw: String },

    #[error("give one of an area, a frame or objects, not several")]
    SeveralAreas,

    #[error("the board is empty: there is no area to show")]
    EmptyBoard,

    #[error(
        "\"{reference}\" covers no place on the board, so there is no area to show"
    )]
    EmptyArea { reference: String },

    #[error("{name} {value:?} is out of range: expected {expected}")]
    OutOfRange {
        name: &'static str,
        value: f64,
        expected: &'static str,
    },

    #[cfg(feature = "png")]
    #[error("the picture could not be drawn as PNG")]
    SvgParse(#[source] resvg::usvg::Error),

    #[error(
        "a {width}×{height} px picture cannot be drawn: use a smaller --scale or --max-side, \
         or a smaller area"
    )]
    TooLarge { width: u32, height: u32 },

    #[error("the picture could not be encoded as PNG")]
    PngEncode,
}

impl PictureError {
    #[must_use]
    pub(super) fn mcp_words(&self) -> Option<String> {
        match self {
            PictureError::TooLarge { width, height } => Some(format!(
                "a {width}×{height} px picture cannot be drawn: use a smaller scale or max_side, \
                 or a smaller part of the board"
            )),
            #[cfg(feature = "png")]
            PictureError::SvgParse(_) => None,
            PictureError::NotArea { .. }
            | PictureError::SeveralAreas
            | PictureError::EmptyBoard
            | PictureError::EmptyArea { .. }
            | PictureError::OutOfRange { .. }
            | PictureError::PngEncode => None,
        }
    }

    /// What to do next, decided for every variant: the message says it all.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            #[cfg(feature = "png")]
            PictureError::SvgParse(_) => None,
            PictureError::NotArea { .. }
            | PictureError::SeveralAreas
            | PictureError::EmptyBoard
            | PictureError::EmptyArea { .. }
            | PictureError::OutOfRange { .. }
            | PictureError::TooLarge { .. }
            | PictureError::PngEncode => None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        hint::{Hint, decided::assert_every_variant},
        picture::{PictureError, PictureErrorDiscriminants},
    };

    pub(in crate::error) fn samples() -> Vec<(PictureError, Option<Hint>)> {
        let samples = vec![
            (PictureError::NotArea { raw: "x".into() }, None),
            (PictureError::SeveralAreas, None),
            (PictureError::EmptyBoard, None),
            (
                PictureError::EmptyArea {
                    reference: "g".into(),
                },
                None,
            ),
            (
                PictureError::OutOfRange {
                    name: "scale",
                    value: 0.0,
                    expected: "0.05..=8",
                },
                None,
            ),
            (
                PictureError::TooLarge {
                    width: 0,
                    height: 0,
                },
                None,
            ),
            (PictureError::PngEncode, None),
        ];
        samples.into_iter().chain(drawing()).collect()
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, PictureErrorDiscriminants>(
            samples(),
            PictureError::hint,
        );
    }

    #[cfg(feature = "png")]
    fn drawing() -> Vec<(PictureError, Option<Hint>)> {
        vec![(
            PictureError::SvgParse(resvg::usvg::Error::ElementsLimitReached),
            None,
        )]
    }

    #[cfg(not(feature = "png"))]
    fn drawing() -> Vec<(PictureError, Option<Hint>)> {
        Vec::new()
    }
}
