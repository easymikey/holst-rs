//! Addresses, ids and values the caller gave.

use crate::error::{
    Error,
    hint::{Hint, Surface},
    http::HttpMethod,
    lookup::Candidates,
};

/// A value that cannot be used as given; nothing was sent.
#[derive(Debug, thiserror::Error)]
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

    #[error(
        "\"{reference}\" names both the workspace root and the folder {folders}: give / or no \
         folder for the root, the folder's id for the folder"
    )]
    RootOrFolder {
        reference: String,
        folders: Candidates,
    },

    #[error("cannot move \"{name}\" into itself or into a folder inside it")]
    MoveIntoItself { name: String },

    #[error(
        "\"{name}\" is your own board, so you cannot leave it: remove it with `holst b rm`"
    )]
    LeaveOwnBoard { name: String },

    #[error("the name is empty: a board or folder needs a visible name")]
    BlankName,

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

    // The words are those of a value's own parse, which are ours.
    #[error("{0}")]
    NotCommandArguments(#[source] serde_json::Error),

    #[error(
        "the new empty board {board} is left in the workspace: moving it to the trash failed ({failed})"
    )]
    ImportBoardLeft { board: String, failed: Box<Error> },
    #[error("{} problems:{}", .errors.len(), listed(.errors))]
    Several { errors: Vec<Error> },

    #[error(transparent)]
    Refused(Refusal),

    #[error("holst api does not send login calls: {method} {path}")]
    LoginCall { method: HttpMethod, path: String },

    #[error("{method} {template} needs the query {key}")]
    MissingQuery {
        method: HttpMethod,
        template: &'static str,
        key: &'static str,
    },

    #[error(
        "{method} {template} changes access or workspace settings, which MCP does not send through `api`"
    )]
    AdminWrite {
        method: HttpMethod,
        template: &'static str,
    },

    #[error("no workspaces found")]
    NoWorkspaces,

    #[error("there are several workspaces; choose one: {workspaces}")]
    WorkspaceNotChosen { workspaces: Candidates },
}

impl InputError {
    #[must_use]
    pub(super) fn mcp_words(&self) -> Option<String> {
        match self {
            // One line, as MCP replies are short.
            InputError::Several { errors: problems } => Some(format!(
                "{} problems: {}",
                problems.len(),
                problems
                    .iter()
                    .map(|problem| problem.on(Surface::Mcp))
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            InputError::RootOrFolder { reference, folders } => Some(format!(
                "\"{reference}\" names both the workspace root and the folder {folders}: give the \
                 workspace id for the root, the folder's id or @N for the folder"
            )),
            InputError::LeaveOwnBoard { name } => Some(format!(
                "\"{name}\" is your own board, so you cannot leave it: remove it with \
                 board_write action=rm"
            )),
            // `--offline` is the CLI's only: MCP never reads offline.
            InputError::OfflineByName { .. }
            | InputError::WorkspaceNotChosen { .. }
            | InputError::BaseNotOrigin { .. }
            | InputError::BaseNotUrl { .. }
            | InputError::BaseNotHttps { .. }
            | InputError::NotUrl { .. }
            | InputError::ForeignOrigin { .. }
            | InputError::NotApiPath { .. }
            | InputError::FullUrl
            | InputError::RelativePath
            | InputError::NotBoardId { .. }
            | InputError::NotWorkspaceId { .. }
            | InputError::NotFolderId { .. }
            | InputError::NotObjectId { .. }
            | InputError::NotPathSegment { .. }
            | InputError::NotFinite { .. }
            | InputError::NotPositive { .. }
            | InputError::NotNodeKind { .. }
            | InputError::NotObjectKind { .. }
            | InputError::NotHttpMethod { .. }
            | InputError::SessionNotHeaderSafe
            | InputError::EmptySharingPatch
            | InputError::TemplateNotObject
            | InputError::BlankName
            | InputError::GuestSharesNoTemplate { .. }
            | InputError::MoveIntoItself { .. }
            | InputError::CacheMaxNotSize { .. }
            | InputError::CacheTrustNotDuration { .. }
            | InputError::CacheDirRelative { .. }
            | InputError::NotArgument { .. }
            | InputError::NoArgument { .. }
            | InputError::NotArgumentValue { .. }
            | InputError::ReplyNotFound { .. }
            | InputError::NotCommandArguments(_)
            | InputError::ImportBoardLeft { .. }
            | InputError::Refused(_)
            | InputError::LoginCall { .. }
            | InputError::MissingQuery { .. }
            | InputError::AdminWrite { .. }
            | InputError::NoWorkspaces => None,
        }
    }

    /// What to do next, decided for every variant.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            InputError::WorkspaceNotChosen { .. } => Some(Hint::NarrowDown),
            InputError::LoginCall { .. } => Some(Hint::LoginAgain),
            InputError::BaseNotOrigin { .. }
            | InputError::BaseNotUrl { .. }
            | InputError::BaseNotHttps { .. }
            | InputError::NotUrl { .. }
            | InputError::ForeignOrigin { .. }
            | InputError::NotApiPath { .. }
            | InputError::FullUrl
            | InputError::RelativePath
            | InputError::NotBoardId { .. }
            | InputError::NotWorkspaceId { .. }
            | InputError::NotFolderId { .. }
            | InputError::NotObjectId { .. }
            | InputError::NotPathSegment { .. }
            | InputError::NotFinite { .. }
            | InputError::NotPositive { .. }
            | InputError::NotNodeKind { .. }
            | InputError::NotObjectKind { .. }
            | InputError::NotHttpMethod { .. }
            | InputError::SessionNotHeaderSafe
            | InputError::EmptySharingPatch
            | InputError::TemplateNotObject
            | InputError::BlankName
            // The message names the flag value to use instead.
            | InputError::GuestSharesNoTemplate { .. }
            | InputError::MoveIntoItself { .. }
            | InputError::CacheMaxNotSize { .. }
            | InputError::CacheTrustNotDuration { .. }
            | InputError::CacheDirRelative { .. }
            | InputError::OfflineByName { .. }
            | InputError::RootOrFolder { .. }
            // The message names the command to use instead.
            | InputError::LeaveOwnBoard { .. }
            | InputError::NoWorkspaces
            | InputError::NotArgument { .. }
            | InputError::NoArgument { .. }
            | InputError::NotArgumentValue { .. }
            | InputError::ReplyNotFound { .. }
            | InputError::NotCommandArguments(_)
            | InputError::ImportBoardLeft { .. }
            // The message names what the call takes or where to send it.
            | InputError::Refused(_)
            | InputError::MissingQuery { .. }
            | InputError::AdminWrite { .. }
            // Each problem's own words say what to change.
            | InputError::Several { .. } => None,
        }
    }
}

fn taken_list(taken: &[&str]) -> String {
    if taken.is_empty() {
        String::from("none")
    } else {
        taken.join(", ")
    }
}

/// Why a request is not a catalogue call; nothing was sent.
#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    #[error(
        "holst api sends only the calls holst knows; {method} {path} is not one of them"
    )]
    Unknown { method: HttpMethod, path: String },

    #[error(
        "holst api sends only the calls holst knows; {template} takes {allowed}, not {method}"
    )]
    Method {
        template: &'static str,
        allowed: String,
        method: HttpMethod,
    },

    #[error("\"{raw}\" is not an id segment in {template}")]
    Segment { raw: String, template: &'static str },

    #[error("{method} {template} takes {allowed}; not {key}")]
    Query {
        method: HttpMethod,
        template: &'static str,
        allowed: String,
        key: String,
    },

    #[error("{method} {template} takes no body")]
    NoBody {
        method: HttpMethod,
        template: &'static str,
    },

    #[error("the body does not fit {method} {template}: {error}")]
    Body {
        method: HttpMethod,
        template: &'static str,
        error: serde_json::Error,
    },
}

fn listed(problems: &[Error]) -> String {
    problems
        .iter()
        .flat_map(|problem| [String::from("\n  - "), problem.to_string()])
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        Error,
        hint::{Hint, decided::assert_every_variant},
        http::HttpMethod,
        input::{InputError, InputErrorDiscriminants, Refusal},
        lookup::Candidates,
    };

    #[expect(clippy::too_many_lines, reason = "one sample per variant")]
    pub(in crate::error) fn samples() -> Vec<(InputError, Option<Hint>)> {
        let text = || String::from("x");
        vec![
            (InputError::BaseNotOrigin { base: text() }, None),
            (InputError::BaseNotUrl { base: text() }, None),
            (InputError::BaseNotHttps { base: text() }, None),
            (InputError::NotUrl { url: text() }, None),
            (
                InputError::ForeignOrigin {
                    origin: text(),
                    base: text(),
                },
                None,
            ),
            (InputError::NotApiPath { path: text() }, None),
            (InputError::FullUrl, None),
            (InputError::RelativePath, None),
            (InputError::NotBoardId { raw: text() }, None),
            (InputError::NotWorkspaceId { raw: text() }, None),
            (InputError::NotFolderId { raw: text() }, None),
            (InputError::NotObjectId { raw: text() }, None),
            (InputError::NotPathSegment { raw: text() }, None),
            (InputError::NotFinite { value: f64::NAN }, None),
            (InputError::NotPositive { value: -1.0 }, None),
            (InputError::NotNodeKind { raw: text() }, None),
            (InputError::NotHttpMethod { raw: text() }, None),
            (
                InputError::NotObjectKind {
                    raw: text(),
                    known: vec![text()],
                },
                None,
            ),
            (InputError::SessionNotHeaderSafe, None),
            (InputError::EmptySharingPatch, None),
            (InputError::TemplateNotObject, None),
            (InputError::BlankName, None),
            (
                InputError::GuestSharesNoTemplate { workspace: text() },
                None,
            ),
            (InputError::LeaveOwnBoard { name: text() }, None),
            (InputError::MoveIntoItself { name: text() }, None),
            (InputError::CacheMaxNotSize { value: text() }, None),
            (InputError::CacheTrustNotDuration { value: text() }, None),
            (InputError::CacheDirRelative { value: text() }, None),
            (InputError::OfflineByName { reference: text() }, None),
            (
                InputError::RootOrFolder {
                    reference: text(),
                    folders: Candidates::default(),
                },
                None,
            ),
            (InputError::NoWorkspaces, None),
            (
                InputError::NotArgument {
                    command: text(),
                    name: text(),
                    taken: Vec::new(),
                },
                None,
            ),
            (
                InputError::NoArgument {
                    command: text(),
                    name: "x",
                },
                None,
            ),
            (
                InputError::NotArgumentValue {
                    command: text(),
                    name: "x",
                    expected: text(),
                },
                None,
            ),
            (
                InputError::ReplyNotFound {
                    reply: text(),
                    thread: text(),
                },
                None,
            ),
            (
                InputError::NotCommandArguments(serde::de::Error::custom("x")),
                None,
            ),
            (
                InputError::ImportBoardLeft {
                    board: text(),
                    failed: Box::new(Error::from(InputError::FullUrl)),
                },
                None,
            ),
            (
                InputError::Several {
                    errors: vec![
                        Error::from(InputError::FullUrl),
                        Error::from(InputError::RelativePath),
                    ],
                },
                None,
            ),
            (
                InputError::WorkspaceNotChosen {
                    workspaces: Candidates::default(),
                },
                Some(Hint::NarrowDown),
            ),
            (
                InputError::Refused(Refusal::NoBody {
                    method: HttpMethod::Post,
                    template: "x",
                }),
                None,
            ),
            (
                InputError::LoginCall {
                    method: HttpMethod::Post,
                    path: text(),
                },
                Some(Hint::LoginAgain),
            ),
            (
                InputError::MissingQuery {
                    method: HttpMethod::Get,
                    template: "x",
                    key: "k",
                },
                None,
            ),
            (
                InputError::AdminWrite {
                    method: HttpMethod::Patch,
                    template: "x",
                },
                None,
            ),
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, InputErrorDiscriminants>(samples(), InputError::hint);
    }
}
