//! An unofficial REST client for Holst (app.holst.so): a pure core, effect
//! ports and a thin shell.
//!
//! ```no_run
//! # async fn demo() -> holst_core::Result<()> {
//! let settings = holst_core::effect::settings::Settings::from_variables(|name| {
//!     std::env::var_os(name)
//! })?;
//! let session = settings.location.load()?.map(|(session, _)| session);
//! let (clock, ids) = (
//!     holst_core::system::clock::SystemClock,
//!     holst_core::system::id::RandomIds,
//! );
//! let client =
//!     holst_core::command::login::connect(session, settings.base_url, clock, ids)?;
//! let me = holst_core::remote::api::user::current(&client).await?;
//! println!("{}", me["name"]);
//! # Ok(())
//! # }
//! ```

pub mod command;
pub mod domain;
pub mod effect;
pub mod error;
#[cfg(test)] mod laws;
pub mod protocol;
pub mod remote;
pub mod storage;
pub mod system;

pub use self::error::{Error, Result};
