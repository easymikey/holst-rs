//! REST API calls by entity; "not verified live": from the bundle only.

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
