//! Board changes (add, edit, remove) as one Yjs v2 update; no I/O.

mod arrange;
pub mod build;
mod check;
mod journal;
pub mod receipt;
mod rich;
mod stacking;
#[cfg(test)] pub(crate) mod tests;
pub mod value;
mod write;
mod write_arrange;
mod write_group;
mod write_many;
mod write_shape;
