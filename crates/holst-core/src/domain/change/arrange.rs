//! Where aligned and spread objects go: a shift for each box; no I/O.

use std::{cmp::Ordering, iter};

use crate::domain::{
    change::value::{Alignment, Axis},
    geometry::Rectangle,
};

/// The box around the boxes that have a size; an empty one if none has.
// BUNDLE: main.a33651a4.js 242565-242578.
fn covering<'a>(boxes: impl IntoIterator<Item = &'a Rectangle>) -> Rectangle {
    let sized = boxes
        .into_iter()
        .copied()
        .filter(|each| each.width > 0.0 || each.height > 0.0);
    Rectangle::bounding(sized).unwrap_or(Rectangle::new(0.0, 0.0, 0.0, 0.0))
}

/// The shift of each box that puts it against the side of the box around all.
// BUNDLE: main.a33651a4.js 242937-242949 (the six sides).
pub(super) fn aligned(boxes: &[Rectangle], to: Alignment) -> Vec<(f64, f64)> {
    let all = covering(boxes);
    boxes
        .iter()
        .map(|own| match to {
            Alignment::Left => (all.x - own.x, 0.0),
            Alignment::Center => {
                (all.x + all.width / 2.0 - own.width / 2.0 - own.x, 0.0)
            }
            Alignment::Right => (all.right() - own.width - own.x, 0.0),
            Alignment::Top => (0.0, all.y - own.y),
            Alignment::Middle => {
                (0.0, all.y + all.height / 2.0 - own.height / 2.0 - own.y)
            }
            Alignment::Bottom => (0.0, all.bottom() - own.height - own.y),
        })
        .collect()
}

fn start(axis: Axis, own: &Rectangle) -> f64 {
    match axis {
        Axis::Horizontal => own.x,
        Axis::Vertical => own.y,
    }
}

fn extent(axis: Axis, own: &Rectangle) -> f64 {
    match axis {
        Axis::Horizontal => own.width,
        Axis::Vertical => own.height,
    }
}

fn end(axis: Axis, own: &Rectangle) -> f64 {
    start(axis, own) + extent(axis, own)
}

fn compared(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

/// The boxes by where they start along the axis; the ones that end at the far
/// edge go last, those that start together by their middles.
// BUNDLE: main.a33651a4.js 243022-243047.
fn in_order(axis: Axis, boxes: &mut [(usize, Rectangle)]) {
    let far = boxes
        .iter()
        .map(|(_, own)| end(axis, own))
        .fold(0.0, f64::max);
    boxes.sort_by(|(_, one), (_, other)| {
        let at_far = |own: &Rectangle| compared(end(axis, own), far).is_eq();
        match (at_far(one), at_far(other)) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (true, true) | (false, false) => {
                compared(start(axis, one), start(axis, other)).then_with(|| {
                    compared(
                        start(axis, one) + extent(axis, one) / 2.0
                            - start(axis, other)
                            - extent(axis, other) / 2.0,
                        0.0,
                    )
                })
            }
        }
    });
}

/// The shift of each box that makes the gaps between the boxes equal, the
/// first and the last staying; none for fewer than three that can move. A box
/// as long as the whole span stays out, and so does what that leaves spanning.
// BUNDLE: main.a33651a4.js 209416-209460.
pub(super) fn distributed(boxes: &[Rectangle], axis: Axis) -> Option<Vec<(f64, f64)>> {
    let all: Vec<(usize, Rectangle)> = boxes.iter().copied().enumerate().collect();
    let kept = iter::successors(Some(all), |kept| {
        let span = covering(kept.iter().map(|(_, own)| own));
        let shorter: Vec<(usize, Rectangle)> = kept
            .iter()
            .copied()
            .filter(|(_, own)| extent(axis, own) < extent(axis, &span))
            .collect();
        (shorter.len() != kept.len()).then_some(shorter)
    })
    .last()?;
    let span = covering(kept.iter().map(|(_, own)| own));
    let mut sorted = kept;
    in_order(axis, &mut sorted);
    let (first, rest) = sorted.split_first()?;
    let (_, middle) = rest.split_last()?;
    let gaps = u32::try_from(sorted.len().checked_sub(1)?).ok()?;
    let total: f64 = sorted.iter().map(|(_, own)| extent(axis, own)).sum();
    let gap = (extent(axis, &span) - total) / f64::from(gaps);
    let limit = end(axis, &span);
    let moved: Vec<(usize, f64)> = middle
        .iter()
        .scan(
            (start(axis, &first.1), end(axis, &first.1)),
            |(previous, edge), (slot, own)| {
                let long = extent(axis, own);
                let (at, spill) = if gap < 0.0 && *edge + gap <= *previous {
                    (*previous + 1.0, *edge + gap - *previous - 1.0)
                } else {
                    (*edge + gap, 0.0)
                };
                let (at, spill) = if at + long > limit {
                    (limit - long, at + long - limit)
                } else {
                    (at, spill)
                };
                *previous = at;
                *edge = at + long + spill;
                Some((*slot, at - start(axis, own)))
            },
        )
        .collect();
    let shifts = (0..boxes.len())
        .map(|slot| {
            let along = moved
                .iter()
                .find(|(each, _)| *each == slot)
                .map_or(0.0, |(_, shift)| *shift);
            match axis {
                Axis::Horizontal => (along, 0.0),
                Axis::Vertical => (0.0, along),
            }
        })
        .collect();
    (sorted.len() >= 3).then_some(shifts)
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        change::{
            arrange::{aligned, distributed},
            value::{Alignment, Axis},
        },
        geometry::Rectangle,
    };

    fn boxes() -> Vec<Rectangle> {
        vec![
            Rectangle::new(0.0, 0.0, 100.0, 50.0),
            Rectangle::new(150.0, 40.0, 20.0, 20.0),
            Rectangle::new(300.0, 100.0, 100.0, 100.0),
        ]
    }

    #[test]
    fn left_puts_every_box_at_the_left_edge_of_all() {
        assert_eq!(
            aligned(&boxes(), Alignment::Left),
            vec![(0.0, 0.0), (-150.0, 0.0), (-300.0, 0.0)]
        );
    }

    #[test]
    fn center_and_middle_meet_at_the_middle_of_all() {
        assert_eq!(
            aligned(&boxes(), Alignment::Center),
            vec![(150.0, 0.0), (40.0, 0.0), (-150.0, 0.0)]
        );
        assert_eq!(
            aligned(&boxes(), Alignment::Middle),
            vec![(0.0, 75.0), (0.0, 50.0), (0.0, -50.0)]
        );
    }

    #[test]
    fn right_and_bottom_put_every_box_at_the_far_edges() {
        assert_eq!(
            aligned(&boxes(), Alignment::Right),
            vec![(300.0, 0.0), (230.0, 0.0), (0.0, 0.0)]
        );
        assert_eq!(
            aligned(&boxes(), Alignment::Bottom),
            vec![(0.0, 150.0), (0.0, 140.0), (0.0, 0.0)]
        );
    }

    #[test]
    fn a_spread_makes_the_gaps_equal_and_keeps_the_outer_two() {
        let spread = distributed(&boxes(), Axis::Horizontal);
        // 400 wide, 220 of boxes: two gaps of 90, so the middle starts at 190.
        assert_eq!(spread, Some(vec![(0.0, 0.0), (40.0, 0.0), (0.0, 0.0)]));
        let down = distributed(&boxes(), Axis::Vertical);
        // 200 tall, 170 of boxes: two gaps of 15, so the middle starts at 65.
        assert_eq!(down, Some(vec![(0.0, 0.0), (0.0, 25.0), (0.0, 0.0)]));
    }

    #[test]
    fn fewer_than_three_boxes_are_not_spread() {
        let two = boxes().into_iter().take(2).collect::<Vec<_>>();
        assert_eq!(distributed(&two, Axis::Horizontal), None);
    }

    #[test]
    fn a_box_as_long_as_the_whole_span_is_left_out() {
        let mut all = boxes();
        all.push(Rectangle::new(0.0, 300.0, 400.0, 10.0));
        let spread = distributed(&all, Axis::Horizontal);
        assert_eq!(
            spread,
            Some(vec![(0.0, 0.0), (40.0, 0.0), (0.0, 0.0), (0.0, 0.0)])
        );
    }
}
