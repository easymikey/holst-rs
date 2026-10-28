#![expect(
    clippy::disallowed_methods,
    reason = "the storage layer is where local file I/O lives"
)]

pub mod cache;
pub mod file;
pub mod local_data;
pub(crate) mod lock;
pub mod session_store;
