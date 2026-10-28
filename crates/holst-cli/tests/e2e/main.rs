//! The built `holst` against the fake server: piped login and error reports.

#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests use real temp files and the real environment"
)]

mod api;
mod board;
#[path = "../support/mod.rs"] mod fake;
mod login;
mod session;
mod storage;
mod support;
