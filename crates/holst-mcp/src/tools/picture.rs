//! The SVG a reply carries.

/// The most SVG a reply carries: the tokens of a full object page.
const SVG_INLINE_MAX: usize = 50_000;

/// The SVG while it fits the reply budget, else a note on what fits.
pub(super) fn inline_svg(svg: String) -> String {
    if svg.len() <= SVG_INLINE_MAX {
        svg
    } else {
        format!(
            "SVG is {} KB, too big to return inline (limit {} KB): use format png \
             (the default), or a smaller part: frame, rectangle or object",
            svg.len().div_ceil(1000),
            SVG_INLINE_MAX / 1000
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::picture::{SVG_INLINE_MAX, inline_svg};

    #[test]
    fn a_huge_svg_is_replaced_by_a_short_note() {
        let small = "<svg/>".repeat(10);
        assert_eq!(inline_svg(small.clone()), small);
        let edge = "x".repeat(SVG_INLINE_MAX);
        assert_eq!(inline_svg(edge.clone()), edge);
        let huge = "x".repeat(1_500_000);
        let note = inline_svg(huge);
        assert!(note.len() < 300, "the note is short: {note}");
        assert!(note.contains("1500 KB") && note.contains("png"));
        assert!(note.contains("frame") && note.contains("object"));
    }
}
