//! Areas of a board and the pictures drawn of them.

use holst_macros::Hint;

/// An area that cannot be chosen, or a picture that cannot be drawn.
#[derive(Debug, thiserror::Error, Hint)]
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

    #[error("a {width}×{height} px picture cannot be drawn")]
    #[hint(ShrinkPicture)]
    TooLarge { width: u32, height: u32 },

    #[error("the picture could not be encoded as PNG")]
    PngEncode,
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        hint::decided::assert_all_listed,
        picture::{PictureError, PictureErrorDiscriminants},
    };

    pub(in crate::error) fn samples() -> Vec<PictureError> {
        let samples = vec![
            PictureError::NotArea { raw: "x".into() },
            PictureError::SeveralAreas,
            PictureError::EmptyBoard,
            PictureError::EmptyArea {
                reference: "g".into(),
            },
            PictureError::OutOfRange {
                name: "scale",
                value: 0.0,
                expected: "0.05..=8",
            },
            PictureError::TooLarge {
                width: 0,
                height: 0,
            },
            PictureError::PngEncode,
        ];
        samples.into_iter().chain(drawing()).collect()
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_all_listed::<_, PictureErrorDiscriminants>(&samples());
    }

    #[cfg(feature = "png")]
    fn drawing() -> Vec<PictureError> {
        vec![PictureError::SvgParse(
            resvg::usvg::Error::ElementsLimitReached,
        )]
    }

    #[cfg(not(feature = "png"))]
    fn drawing() -> Vec<PictureError> {
        Vec::new()
    }
}
