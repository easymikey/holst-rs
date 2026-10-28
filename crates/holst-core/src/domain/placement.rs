//! A free spot for a new object among the objects already there; no I/O.

use std::ops::Range;

use itertools::Itertools;

use crate::{
    domain::{
        area::scene::Scene,
        board::Object,
        geometry::{Point, Rectangle, Size},
    },
    error::Result,
};

pub const GAP: f64 = 24.0;

/// A free spot with `placed` (board coordinates) taken: objects
/// that are not on the board yet.
///
/// # Errors
/// [`crate::error::input::InputError::NotFinite`] when the spot overflows.
pub fn free_spot_among(
    scene: &Scene<'_>,
    frame: Option<&Object>,
    size: Size,
    placed: &[Rectangle],
) -> Result<Point> {
    let rectangles = |is_included: &dyn Fn(&Object) -> bool| -> Vec<Rectangle> {
        scene
            .objects()
            .iter()
            .filter(|object| is_included(object))
            .filter_map(|object| scene.covered_by(object))
            .collect()
    };
    let area = frame.and_then(|frame| {
        scene
            .rectangle_of(frame)
            .map(|rectangle| (frame, rectangle))
    });
    area.map_or_else(
        || {
            let taken: Vec<Rectangle> = rectangles(&|_: &Object| true)
                .into_iter()
                .chain(placed.iter().copied())
                .collect();
            let whole = Rectangle::bounding(taken.iter().copied())
                .unwrap_or(Rectangle::new(0.0, 0.0, 0.0, 0.0));
            below(whole, &taken)
        },
        |(parent_frame, rectangle)| {
            let taken: Vec<Rectangle> = rectangles(&|object: &Object| {
                object.id != parent_frame.id
                    && scene
                        .covered_by(object)
                        .is_some_and(|covered| rectangle.intersects(&covered))
            })
            .into_iter()
            .chain(
                placed
                    .iter()
                    .copied()
                    .filter(|other| rectangle.intersects(other)),
            )
            .collect();
            free_corner(rectangle, &taken, size)?.shifted(-rectangle.x, -rectangle.y)
        },
    )
}

/// Returns the top-left corner of the first free `size` spot inside `frame` in
/// reading order, `GAP` clear of every `taken` rectangle; when none fits, the
/// spot below the lowest of them.
///
/// ```
/// use holst_core::domain::{
///     geometry::{Point, Rectangle, Size},
///     placement::free_corner,
/// };
///
/// let frame = Rectangle::new(0.0, 0.0, 1000.0, 500.0);
/// let first = Rectangle::new(24.0, 24.0, 192.0, 192.0);
/// assert_eq!(
///     free_corner(frame, &[first], Size::STICKER)?,
///     Point::new(240.0, 24.0)?
/// );
/// let full = Rectangle::new(0.0, 0.0, 1000.0, 500.0);
/// assert_eq!(
///     free_corner(frame, &[full], Size::STICKER)?,
///     Point::new(24.0, 524.0)?
/// );
///
/// # Ok::<(), holst_core::Error>(())
/// ```
///
/// # Errors
/// [`crate::error::input::InputError::NotFinite`] when the spot overflows.
pub fn free_corner(frame: Rectangle, taken: &[Rectangle], size: Size) -> Result<Point> {
    let xs = candidates(frame.x, taken.iter().map(Rectangle::right));
    let ys = candidates(frame.y, taken.iter().map(Rectangle::bottom));
    let blockers: Vec<Rectangle> =
        taken.iter().map(|other| other.padded(GAP - 1.0)).collect();
    ys.iter()
        .find_map(|&y| {
            first_free_x(
                frame,
                &blockers,
                &xs,
                Rectangle::new(0.0, y, size.width(), size.height()),
            )
            .map(|x| (x, y))
        })
        .map_or_else(|| below(frame, taken), |(x, y)| Point::new(x, y))
}

/// Returns the corners to try along one axis, ascending and without repeats.
fn candidates(start: f64, edges: impl Iterator<Item = f64>) -> Vec<f64> {
    std::iter::once(start)
        .chain(edges)
        .map(|edge| edge + GAP)
        .sorted_by(f64::total_cmp)
        .dedup_by(|a, b| a.total_cmp(b).is_eq())
        .collect()
}

/// Returns the smallest of the ascending `xs` where a spot of `row`'s size at
/// `row.y` is inside `frame` and clear of every blocker.
///
/// A blocker hides a contiguous run of `xs`, so the runs are merged and the
/// first gap that also stays in the frame wins.
fn first_free_x(
    frame: Rectangle,
    blockers: &[Rectangle],
    xs: &[f64],
    candidate: Rectangle,
) -> Option<f64> {
    let reach = |x: f64| x + candidate.width;
    let hidden: Vec<Range<usize>> = blockers
        .iter()
        .filter(|blocker| {
            blocker.y <= candidate.bottom() && candidate.y <= blocker.bottom()
        })
        .map(|blocker| {
            xs.partition_point(|&x| blocker.x > reach(x))
                ..xs.partition_point(|&x| x <= blocker.right())
        })
        .filter(|run| run.start < run.end)
        .sorted_by_key(|run| run.start)
        .collect();
    let (gaps, cursor) = hidden.iter().fold(
        (Vec::<Range<usize>>::new(), 0),
        |(mut gaps, cursor), run| {
            gaps.push(cursor..run.start.max(cursor));
            (gaps, cursor.max(run.end))
        },
    );
    gaps.into_iter()
        .chain(std::iter::once(cursor..xs.len()))
        .flatten()
        .filter_map(|index| xs.get(index).copied())
        .find(|&x| {
            frame.contains(
                &Rectangle::new(x, candidate.y, candidate.width, candidate.height)
                    .padded(GAP),
            )
        })
}

/// Returns the corner below everything in `taken`, at the left of `area`.
///
/// ```
/// use holst_core::domain::{
///     geometry::{Point, Rectangle},
///     placement::below,
/// };
///
/// let area = Rectangle::new(-50.0, 0.0, 100.0, 100.0);
/// assert_eq!(
///     below(area, &[Rectangle::new(0.0, 0.0, 10.0, 300.0)])?,
///     Point::new(-26.0, 324.0)?
/// );
///
/// # Ok::<(), holst_core::Error>(())
/// ```
///
/// # Errors
/// [`crate::error::input::InputError::NotFinite`] when the corner overflows.
pub fn below(frame: Rectangle, taken: &[Rectangle]) -> Result<Point> {
    let lowest = taken.iter().map(Rectangle::bottom).fold(frame.y, f64::max);
    Point::new(frame.x + GAP, lowest + GAP)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use proptest::{
        collection::vec,
        prelude::{Strategy, prop_assert_eq, proptest},
    };

    use crate::{
        domain::{
            geometry::{Point, Rectangle, Size},
            placement::{GAP, below, free_corner},
        },
        error::Result,
        laws::config,
    };

    /// The cubic search `free_corner` replaced: every corner, then the least.
    fn free_spot_by_every_corner(
        frame: Rectangle,
        taken: &[Rectangle],
        size: Size,
    ) -> Result<Point> {
        let xs =
            std::iter::once(frame.x + GAP).chain(taken.iter().map(|r| r.right() + GAP));
        let ys: Vec<f64> = std::iter::once(frame.y + GAP)
            .chain(taken.iter().map(|r| r.bottom() + GAP))
            .collect();
        xs.flat_map(|x| ys.iter().map(move |&y| (x, y)))
            .filter(|&(x, y)| {
                let placed = Rectangle::new(x, y, size.width(), size.height());
                frame.contains(&placed.padded(GAP))
                    && !taken
                        .iter()
                        .any(|other| other.padded(GAP - 1.0).intersects(&placed))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)))
            .map_or_else(|| below(frame, taken), |(x, y)| Point::new(x, y))
    }

    fn rectangle(
        span: std::ops::Range<i32>,
        side: std::ops::Range<i32>,
    ) -> impl Strategy<Value = Rectangle> {
        (span.clone(), span, side.clone(), side).prop_map(|(x, y, width, height)| {
            Rectangle::new(
                f64::from(x),
                f64::from(y),
                f64::from(width),
                f64::from(height),
            )
        })
    }

    proptest! {
        #![proptest_config(config())]

        /// The sweep picks the very corner the search over every corner picks.
        #[test]
        fn the_sweep_finds_the_same_spot_as_the_search_over_every_corner(
            frame in rectangle(-100..300, 0..900),
            taken in vec(rectangle(-100..800, 0..260), 0..14),
            side in (1_i32..300, 1_i32..300),
        ) {
            let size = Size::new(f64::from(side.0), f64::from(side.1)).ok();
            prop_assert_eq!(
                size.and_then(|size| free_corner(frame, &taken, size).ok()),
                size.and_then(|size| free_spot_by_every_corner(frame, &taken, size).ok())
            );
        }

        /// The same on coarse grids, where edges touch and corners repeat.
        #[test]
        fn the_sweep_agrees_on_grids_with_touching_edges(
            columns in 1_usize..6,
            filled in vec(0_usize..30, 0..24),
        ) {
            let cell = |count: usize| f64::from(u32::try_from(count).unwrap_or(0));
            let taken: Vec<Rectangle> = filled
                .iter()
                .map(|slot| {
                    Rectangle::new(
                        cell(slot % columns) * 216.0 + 24.0,
                        cell(slot / columns) * 216.0 + 24.0,
                        192.0,
                        192.0,
                    )
                })
                .collect();
            let frame = Rectangle::new(0.0, 0.0, cell(columns) * 216.0 + 24.0, 1500.0);
            prop_assert_eq!(
                free_corner(frame, &taken, Size::STICKER).ok(),
                free_spot_by_every_corner(frame, &taken, Size::STICKER).ok()
            );
        }
    }

    #[test]
    fn a_full_line_continues_below() -> Result<()> {
        let frame = Rectangle::new(0.0, 0.0, 480.0, 1000.0);
        let stickers = [
            Rectangle::new(24.0, 24.0, 192.0, 192.0),
            Rectangle::new(240.0, 24.0, 192.0, 192.0),
        ];
        assert_eq!(
            free_corner(frame, &stickers, Size::STICKER)?,
            Point::new(24.0, 240.0)?
        );
        assert_eq!(
            free_corner(frame, &[], Size::STICKER)?,
            Point::new(24.0, 24.0)?
        );
        Ok(())
    }
}
