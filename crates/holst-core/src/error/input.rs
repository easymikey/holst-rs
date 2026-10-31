//! Addresses, ids and values the caller gave.

use holst_macros::Hint;

use crate::error::{Error, lookup::Candidates};

/// Characters of the offending line kept on each side of the column.
const SPOT_REACH: usize = 40;

/// Where JSON argument text goes wrong: 1-based line and column in the
/// argument, and the offending line cut to a window around the column, with
/// `…` where it was cut and a tab as one space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonSpot {
    pub line: usize,
    pub column: usize,
    pub text: String,
    /// Where the column falls in `text`, in characters.
    pub caret: usize,
}

impl JsonSpot {
    /// The position `error` reports in `text`.
    ///
    /// ```
    /// use holst_core::error::input::JsonSpot;
    ///
    /// let text = "{\n  \"a\": ,\n}";
    /// let error = serde_json::from_str::<serde_json::Value>(text).unwrap_err();
    /// let spot = JsonSpot::of(&error, text);
    /// assert_eq!((spot.line, spot.column), (2, 8));
    /// assert_eq!((spot.text.as_str(), spot.caret), ("  \"a\": ,", 7));
    ///
    /// let long = format!("[{}1 2{}]", "0,".repeat(100), ",0".repeat(100));
    /// let error = serde_json::from_str::<serde_json::Value>(&long).unwrap_err();
    /// let spot = JsonSpot::of(&error, &long);
    /// assert_eq!(spot.text.chars().count(), 1 + 40 + 40 + 1);
    /// assert!(spot.text.starts_with('…') && spot.text.ends_with('…'));
    /// ```
    #[must_use]
    pub fn of(error: &serde_json::Error, text: &str) -> Self {
        let line = text
            .lines()
            .nth(error.line().saturating_sub(1))
            .unwrap_or_default();
        let mut at = error.column().saturating_sub(1).min(line.len());
        while !line.is_char_boundary(at) {
            at -= 1;
        }
        let (before, after) = line.split_at(at);
        let skipped = before.chars().count().saturating_sub(SPOT_REACH);
        let before: String = before.chars().skip(skipped).collect();
        let lead = if skipped > 0 { "…" } else { "" };
        let tail = if after.chars().nth(SPOT_REACH).is_some() {
            "…"
        } else {
            ""
        };
        let after: String = after.chars().take(SPOT_REACH).collect();
        Self {
            line: error.line(),
            column: error.column(),
            text: format!("{lead}{before}{after}{tail}").replace('\t', " "),
            caret: lead.chars().count() + before.chars().count(),
        }
    }
}

/// A value that cannot be used as given; nothing was sent.
#[derive(Debug, thiserror::Error, Hint)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum InputError {
    #[error(
        "the API base address must be only a scheme, host and port, without a path, query or fragment: {base}"
    )]
    BaseNotOrigin { base: String },

    #[error("the API base address is not a URL: {base}")]
    BaseNotUrl { base: String },

    #[error("the API base address must use https: {base}")]
    BaseNotHttps { base: String },

    #[error("not a URL: {url}")]
    NotUrl { url: String },

    #[error("{origin} is a foreign address: the session is sent only to {base}")]
    ForeignOrigin { origin: String, base: String },

    #[error("not an API path: {path}")]
    NotApiPath { path: String },

    #[error(
        "expected a path, not a full URL: the session is sent only to its own server"
    )]
    FullUrl,

    #[error("the path must start with \"/\", for example /gapi/current_user")]
    RelativePath,

    #[error(
        "\"{raw}\" is not a board id: expected a UUID like 00000000-0000-0000-0000-000000000000"
    )]
    NotBoardId { raw: String },

    #[error(
        "\"{raw}\" is not a workspace id: expected a UUID like 00000000-0000-0000-0000-000000000000"
    )]
    NotWorkspaceId { raw: String },

    #[error(
        "\"{raw}\" is not a folder id: expected a UUID like 00000000-0000-0000-0000-000000000000"
    )]
    NotFolderId { raw: String },

    #[error(
        "\"{raw}\" is not an object id: expected a non-empty id without spaces or control characters"
    )]
    NotObjectId { raw: String },

    #[error("\"{raw}\" cannot be used as an id in a request path")]
    NotPathSegment { raw: String },

    #[error("{value} is not a coordinate: expected a finite number, e.g. -40.5")]
    NotFinite { value: f64 },

    #[error("{value} is not a positive number: sides are above 0, e.g. 192")]
    NotPositive { value: f64 },

    #[error("unknown kind \"{raw}\": expected board or folder")]
    NotNodeKind { raw: String },

    #[error("unknown method \"{raw}\"")]
    NotHttpMethod { raw: String },

    // The kinds are names: this layer does not know `ObjectKind`.
    #[error("\"{raw}\" is not an object kind: expected {}", .known.join(", "))]
    NotObjectKind { raw: String, known: Vec<String> },

    #[error(
        "\"{name}\" is not an icon: expected an icon name, e.g. rocket-launch{}",
        close_list(.close)
    )]
    NotIcon { name: String, close: Vec<String> },

    #[error("the session token contains characters that cannot be sent in a header")]
    SessionNotHeaderSafe,

    #[error("sharing settings must be a non-empty JSON object")]
    EmptySharingPatch,

    #[error("a template's `id` and `name` must be text")]
    TemplateNotObject,

    #[error("HOLST_CACHE_MAX must be a size like 256M or 1G, not \"{value}\"")]
    CacheMaxNotSize { value: String },

    #[error("HOLST_CACHE_TRUST must be a duration like 24h, 30m or 0, not \"{value}\"")]
    CacheTrustNotDuration { value: String },

    #[error("HOLST_CACHE_DIR must be an absolute path, not \"{value}\"")]
    CacheDirRelative { value: String },

    #[error(
        "--offline needs the board's id or link: \"{reference}\" is looked up on the server"
    )]
    OfflineByName { reference: String },

    #[error("\"{reference}\" names both the workspace root and the folder {folders}")]
    #[hint(NameRootOrFolder)]
    RootOrFolder {
        reference: String,
        folders: Candidates,
    },

    #[error("cannot move \"{name}\" into itself or into a folder inside it")]
    MoveIntoItself { name: String },

    #[error("\"{name}\" is your own board, so you cannot leave it")]
    #[hint(RemoveInstead)]
    LeaveOwnBoard { name: String },

    #[error("the name is empty: a board or folder needs a visible name")]
    BlankName,

    #[error("the name is empty: an image or a file needs a visible name")]
    BlankMediaName,

    #[error(
        "you are a guest of workspace \"{workspace}\", and a guest cannot make its templates: \
         make a template for yourself instead"
    )]
    GuestSharesNoTemplate { workspace: String },

    #[error("\"{name}\" is not an argument of `{command}`: it takes {}", taken_list(.taken))]
    NotArgument {
        command: String,
        name: String,
        taken: Vec<&'static str>,
    },

    #[error("reply {reply} not found in thread {thread}")]
    ReplyNotFound { reply: String, thread: String },
    #[error("{name} is required by `{command}`")]
    NoArgument { command: String, name: &'static str },

    #[error("{name} of `{command}` must be {expected}")]
    NotArgumentValue {
        command: String,
        name: &'static str,
        expected: String,
    },

    #[error("{name} of `{command}` must be a JSON object")]
    NotJsonObject {
        command: String,
        name: &'static str,
        spot: JsonSpot,
    },

    // The words are those of a value's own parse, which are ours.
    #[error(transparent)]
    NotCommandArguments(serde_json::Error),

    #[error(
        "the new empty board {board} is left in the workspace: moving it to the trash failed ({failed})"
    )]
    ImportBoardLeft { board: String, failed: Box<Error> },
    #[error("the cover was changed, but the note was not written")]
    CoverChanged,
    #[error("{} problems: {}", .errors.len(), joined(.errors))]
    Several { errors: Vec<Error> },

    #[error("no workspaces found")]
    NoWorkspaces,

    #[error("there are several workspaces; choose one: {workspaces}")]
    #[hint(NarrowDown)]
    WorkspaceNotChosen { workspaces: Candidates },
}

fn close_list(close: &[String]) -> String {
    match close {
        [] => String::new(),
        _ => format!(" (close: {})", close.join(", ")),
    }
}

fn taken_list(taken: &[&str]) -> String {
    if taken.is_empty() {
        String::from("none")
    } else {
        taken.join(", ")
    }
}

fn joined(errors: &[Error]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        Error,
        hint::decided::assert_all_listed,
        input::{InputError, InputErrorDiscriminants, JsonSpot},
        lookup::Candidates,
    };

    pub(in crate::error) fn samples() -> Vec<InputError> {
        let text = || String::from("x");
        vec![
            InputError::BaseNotOrigin { base: text() },
            InputError::BaseNotUrl { base: text() },
            InputError::BaseNotHttps { base: text() },
            InputError::NotUrl { url: text() },
            InputError::ForeignOrigin {
                origin: text(),
                base: text(),
            },
            InputError::NotApiPath { path: text() },
            InputError::FullUrl,
            InputError::RelativePath,
            InputError::NotBoardId { raw: text() },
            InputError::NotWorkspaceId { raw: text() },
            InputError::NotFolderId { raw: text() },
            InputError::NotObjectId { raw: text() },
            InputError::NotPathSegment { raw: text() },
            InputError::NotFinite { value: f64::NAN },
            InputError::NotPositive { value: -1.0 },
            InputError::NotNodeKind { raw: text() },
            InputError::NotHttpMethod { raw: text() },
            InputError::NotObjectKind {
                raw: text(),
                known: vec![text()],
            },
            InputError::SessionNotHeaderSafe,
            InputError::EmptySharingPatch,
            InputError::TemplateNotObject,
            InputError::BlankName,
            InputError::BlankMediaName,
            InputError::NotIcon {
                name: text(),
                close: Vec::new(),
            },
            InputError::GuestSharesNoTemplate { workspace: text() },
            InputError::LeaveOwnBoard { name: text() },
            InputError::MoveIntoItself { name: text() },
            InputError::CacheMaxNotSize { value: text() },
            InputError::CacheTrustNotDuration { value: text() },
            InputError::CacheDirRelative { value: text() },
            InputError::OfflineByName { reference: text() },
            InputError::RootOrFolder {
                reference: text(),
                folders: Candidates::default(),
            },
            InputError::NoWorkspaces,
            InputError::CoverChanged,
            InputError::NotArgument {
                command: text(),
                name: text(),
                taken: Vec::new(),
            },
            InputError::NoArgument {
                command: text(),
                name: "x",
            },
            InputError::NotArgumentValue {
                command: text(),
                name: "x",
                expected: text(),
            },
            InputError::NotJsonObject {
                command: text(),
                name: "x",
                spot: JsonSpot {
                    line: 2,
                    column: 5,
                    text: text(),
                    caret: 4,
                },
            },
            InputError::ReplyNotFound {
                reply: text(),
                thread: text(),
            },
            InputError::NotCommandArguments(serde::de::Error::custom("x")),
            InputError::ImportBoardLeft {
                board: text(),
                failed: Box::new(Error::from(InputError::FullUrl)),
            },
            InputError::Several {
                errors: vec![
                    Error::from(InputError::FullUrl),
                    Error::from(InputError::RelativePath),
                ],
            },
            InputError::WorkspaceNotChosen {
                workspaces: Candidates::default(),
            },
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_all_listed::<_, InputErrorDiscriminants>(&samples());
    }

    #[test]
    fn several_problems_are_listed_on_one_line() {
        let several = InputError::Several {
            errors: vec![
                Error::from(InputError::FullUrl),
                Error::from(InputError::RelativePath),
            ],
        };
        assert_eq!(
            several.to_string(),
            format!(
                "2 problems: {}; {}",
                InputError::FullUrl,
                InputError::RelativePath
            )
        );
    }
}
