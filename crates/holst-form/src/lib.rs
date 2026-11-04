//! Deprecated: use [clacky](https://crates.io/crates/clacky). holst-cli and holst-mcp moved to it, and this crate gets no more releases.
//!
//! Terminal pieces shared by the Holst command-line tools.

pub mod color_choice;
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
