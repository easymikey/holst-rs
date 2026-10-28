//! Board areas for the model.

use std::fmt;

use holst_core::domain::geometry::Rectangle;

fn int(value: f64) -> impl fmt::Display {
    fmt::from_fn(move |f| write!(f, "{:.0}", value.round() + 0.0))
}

pub(crate) fn rounded(area: &Rectangle) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        write!(
            f,
            "{},{},{},{}",
            int(area.x),
            int(area.y),
            int(area.width),
            int(area.height)
        )
    })
}
