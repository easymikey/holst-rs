//! The core against its fakes and mocks; nothing goes to Holst.
#![expect(
    clippy::panic_in_result_fn,
    reason = "asserting is the point of a test"
)]

mod admin;
mod board_read;
mod board_write;
mod comments;
mod common;
mod fake_routes;
mod fill_speed;
mod page_count;
mod requests;
mod resolve;
mod session_on_fake;
mod shared_cache;
mod transport;
mod tree_cache;
