//! The roles of the web client's scale, as a board's settings name them.
/// A role on a board, the lowest first, so that roles compare as the web
/// client does.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    strum::Display,
    strum::EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum BoardRole {
    Viewer,
    Commentator,
    Editor,
    CoOwner,
    Owner,
}
