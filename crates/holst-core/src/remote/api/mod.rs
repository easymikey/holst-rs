//! REST API calls by entity; "not verified live": from the bundle only.

use serde::{Deserialize, de::IgnoredAny};

use crate::remote::catalogue::Endpoint;

/// Who can see and edit a board.
pub mod access;
/// Automatic board snapshots.
pub mod backup;
/// Boards and folders — "channels" in the API.
pub mod channel;
/// Comment threads on boards.
pub mod comment;
/// Favorite boards.
pub mod favorite;
/// Uploaded files.
pub mod files;
/// Board updates sent ahead of the socket frame.
pub mod hud;
/// Link previews.
pub mod link;
/// Logging in by email and out.
pub mod login;
/// Recently opened boards.
pub mod recent;
/// The workspace's plan.
pub mod subscription;
/// Board templates: standard, personal and shared.
pub mod template;
#[cfg(test)] mod tests;
/// The current user and other people.
pub mod user;
/// Workspaces: their trees, trash, members and settings.
pub mod workspace;

/// Every module's `ENDPOINTS`; a call missing there is unused code.
pub(crate) const CATALOGUE: &[&[Endpoint]] = &[
    recent::ENDPOINTS,
    favorite::ENDPOINTS,
    comment::ENDPOINTS,
    channel::ENDPOINTS,
    access::ENDPOINTS,
    backup::ENDPOINTS,
    template::ENDPOINTS,
    user::ENDPOINTS,
    link::ENDPOINTS,
    workspace::ENDPOINTS,
];

/// The reply of a call that carries nothing: `null`, an object, anything.
#[derive(Debug, Deserialize)]
pub struct Done(IgnoredAny);
