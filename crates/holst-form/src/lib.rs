//! Terminal pieces shared by the Holst command-line tools.

pub mod coloring;
pub mod detect;
pub mod draw;
pub mod error;
#[cfg(any(test, feature = "fake"))] pub mod fake;
pub mod form;
pub mod line;
pub mod prompt;
pub mod signals;
pub mod terminal;
pub mod tty;
