//! Addresses, ids and values the caller gave.

use holst_macros::Hint;

use crate::{
    command::{
        board::{PictureFormat, rights::Right},
        object::add_args::AddKind,
        table::ValueKind,
    },
    domain::{board::ObjectKind, change::value::Alignment},
    error::{Error, flag::Flag, lookup::Candidates},
};

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

    #[error("`{flag}` cannot be used with `{other}`")]
    Conflict { flag: Flag, other: Flag },

    #[error("`{flag}` needs `{needs}`")]
    Requires { flag: Flag, needs: Flag },

    #[error("give one of {}", flag_list(.flags))]
    OneOf { flags: &'static [Flag] },

    /// `slot` counts from 1.
    #[error("{error}, in item {slot}")]
    InItem { slot: usize, error: Box<InputError> },

    #[error("`{flag}` cannot be used for {kind}")]
    NotFor { flag: Flag, kind: AddKind },

    #[error("`{flag}` is empty: give a visible value")]
    Blank { flag: Flag },

    #[error("there is no home directory: give `{flag}`")]
    NoHomeDirectory { flag: Flag },

    /// `slot` counts from 1.
    #[error("`{flag}` #{slot} is not an earlier item")]
    NoEarlierItem { flag: Flag, slot: usize },

    #[error("`{flag}` must be {kind}")]
    NotKind { flag: Flag, kind: ValueKind },

    #[error(
        "\"{raw}\" is not a side: give one of {}",
        <Alignment as strum::VariantNames>::VARIANTS.join(", ")
    )]
    NotAlignSide { raw: String },

    #[error("`{flag}` cannot be used with several objects")]
    NotForSeveralObjects { flag: Flag },

    #[error("arrow {arrow} is stuck to an object left out of the copy")]
    ArrowEndLeftOut { arrow: String },

    #[error("{} {object} cannot be copied", .kind.short_name())]
    NotCopyable { object: String, kind: ObjectKind },

    #[error(
        "`{flag}` must be the board of the objects while {} {object} is among them",
        .kind.short_name()
    )]
    FileStaysOnBoard {
        flag: Flag,
        object: String,
        kind: ObjectKind,
    },

    #[error("`{flag}` must be one of {}", right_list(.allowed))]
    NotRight {
        flag: Flag,
        allowed: &'static [Right],
    },

    #[error("`{flag}` must be hud or hocuspocus")]
    NotProvider { flag: Flag },

    #[error("output must be one of text, json")]
    NotOutput,
    #[error(
        "`--stroke-width` must be a whole number 0 to 10 (a drawing 1 to 30), or for an arrow 1, 1.5, 2, 4, 5, 6, 9, 13 or 17"
    )]
    StrokeWidth,

    #[error("`--opacity` must be a number 0.1 to 1")]
    Opacity,

    #[error("`--rotate` must be a number of degrees")]
    RotateNotFinite,

    #[error("`--scale` must be a number above 0")]
    ScaleNotPositive,

    #[error("`--width` must be auto or a number above 0")]
    WidthNotAutoOrPositive,

    #[error("`--text-size` must be auto or a whole number 4 to 64")]
    TextSizeRange,

    #[error("`{flag}` must be row,column from 1, e.g. 2,3")]
    NotRowColumn { flag: Flag },

    #[error(
        "`{flag}` must be r1,c1,r2,c2 from 1, the first row and column not after the last, e.g. 1,1,2,2"
    )]
    NotBlock { flag: Flag },

    #[error("`--cell` needs `--text` or `--color`")]
    CellNeedsTextOrColor,

    #[error("`--text` is written into one cell: give `--cell` as row,column, e.g. 2,3")]
    CellTextNeedsOneCell,

    #[error("`--cell` takes `--text` or `--color`, not both")]
    CellTextAndColor,

    #[error("`--faces` must be one of 2, 4, 6, 8, 10, 12 or 20")]
    FacesNotDie,

    #[error("`--reaction` takes reaction ids, or none alone")]
    ReactionsMixed,

    #[error("`--reaction` takes at most {most} different reactions")]
    TooManyReactions { most: usize },

    #[error("`{flag}` must be an item number from 1")]
    NotItemNumber { flag: Flag },

    #[error("`{flag}` must be a day as YYYY-MM-DD, e.g. 2026-11-06")]
    NotDay { flag: Flag },

    #[error("`--start` must be on or before the `--due` day")]
    StartAfterDue,

    #[error("`{flag}` takes one title")]
    OneTitle { flag: Flag },

    #[error("`{flag}` cannot be used with `--kanban` none")]
    KanbanNone { flag: Flag },

    #[error("`--roll` is for at most {most} objects at once")]
    TooManyRolls { most: usize },

    #[error("`--path` must be a file with a png, jpg, gif or webp picture")]
    PathNotPicture,

    #[error(
        "`--assignee` is for members of the workspace: a guest cannot assign people"
    )]
    AssigneeNeedsMember,

    #[error("`--object` is needed: give it, or link an object in `--board`")]
    ItemNeedsObject,

    #[error("`--path` is for one object: give one `--object`")]
    PathWithSeveralObjects,
    #[error("an offline search needs a board given by id or link")]
    OfflineNeedsBoardId,
    #[error("a board is searched for its text: leave it out with mode names")]
    BoardWithNamesMode,
    #[error("mode names takes the kind board, folder or any")]
    KindNotNodeInNamesMode,
    #[error("mode board_text takes an object kind (sticker, text, shape, frame…)")]
    KindNotObjectInTextMode,
    #[error("only names look into the trash: leave it out with mode board_text")]
    TrashWithBoardText,
    #[error("a board is given: the kind must be an object kind")]
    KindNotObjectWithBoard,
    #[error(
        "only names look into the trash: leave it out with a board or an object kind"
    )]
    TrashWithBoardOrObjectKind,
    #[error("the cover must be a png, jpg, gif or webp picture, or reset")]
    CoverNotPicture,
    #[error("the cover must be a picture drawn in memory")]
    CoverNotInMemory,
    #[error("an object list is saved beside a file: give a file, not -")]
    ObjectListToStdout,
    #[error("the file's name gives no board name: give a name")]
    ImportNameNotFromFile,
    #[error("the file must be named with .svg or .png")]
    FileNotPictureName,
    #[error("the picture format must be {found}, as the file's name says")]
    PictureFormatNotName { found: PictureFormat },
    #[error("this build draws no PNG: the picture format must be svg")]
    NoPng,
    #[error("a folder is listed in the tree and both views only")]
    FolderWithView,
    #[error("the cursor must be one a previous page printed")]
    CursorUnknown,
    #[error("`{flag}` cannot be used with several boards")]
    SeveralBoards { flag: Flag },
    #[error("the note format applies to the note: add note to the parts shown")]
    NoteFormatWithoutNote,
    #[error("a category is listed in the standard view only")]
    CategoryWithoutStandardView,
    #[error("only a reply of your own can be removed; another author's reply stays")]
    OthersReply,
    #[error("the point must be two numbers like 120,80")]
    AtNotPoint,
    #[error("the time must be ISO 8601, e.g. 2026-10-27T09:30:00Z")]
    TimeNotIso,
    #[error("`{flag}` must name {wanted}")]
    WrongObject { flag: Flag, wanted: &'static str },

    #[error("`{flag}` must be one of {}", .titles.join(", "))]
    NotATitle { flag: Flag, titles: Vec<String> },

    #[error(
        "`{flag}` must be one of {}, or the title of a status of the board",
        .ids.join(", ")
    )]
    UnknownStatus { flag: Flag, ids: Vec<&'static str> },

    #[error("`{flag}` \"{raw}\" is not #N, the number of an earlier item")]
    NotItemRef { flag: Flag, raw: String },

    /// `slot` counts from 1.
    #[error("`{flag}` #{slot} is not a frame")]
    ItemNotFrame { flag: Flag, slot: usize },

    /// `slot` counts from 1.
    #[error("`{flag}` #{slot} is an arrow, which cannot be an arrow end")]
    ArrowEndIsArrow { flag: Flag, slot: usize },

    #[error("`{flag}` must be one of {}", .allowed.join(", "))]
    NotAmong {
        flag: Flag,
        allowed: Vec<&'static str>,
    },

    #[error("`{flag}` must be one of {}", number_list(.allowed))]
    NotAmongNumbers { flag: Flag, allowed: &'static [u32] },

    #[error("`{flag}` must be at least {least}")]
    Below { flag: Flag, least: u32 },

    #[error("`{flag}` needs at least {least} values")]
    TooFew { flag: Flag, least: usize },

    #[error("`{flag}` takes {least} to {most} values")]
    ValueCount {
        flag: Flag,
        least: usize,
        most: usize,
    },

    #[error("`{flag}` labels are at most {most} characters long")]
    LabelTooLong { flag: Flag, most: usize },

    #[error("`{flag}` must be a whole number from {least} to {most}")]
    NotWhole { flag: Flag, least: u32, most: u32 },

    #[error("`{flag}` times `{other}` is at most {limit} cells")]
    TooManyCells { flag: Flag, other: Flag, limit: u32 },

    #[error("`{flag}` must be a day as YYYY-MM-DD, e.g. 2026-11-06")]
    NotADay { flag: Flag },

    #[error("`{flag}` must be on or before `{other}`")]
    NotBefore { flag: Flag, other: Flag },

    #[error(
        "`{flag}` must be an icon name: lower-case letters, digits and dashes, e.g. rocket-launch"
    )]
    NotIconName { flag: Flag },

    #[error("`{flag}` must be square, e.g. 48,48")]
    NotSquare { flag: Flag },

    #[error("`{flag}` must be 192,192 or 384,192")]
    NotStackSize { flag: Flag },

    #[error("`{flag}` takes one value here")]
    OneValue { flag: Flag },

    #[error("`{flag}` must be a file")]
    NotFile { flag: Flag },

    #[error("`{flag}` must be a png, jpg, gif or webp picture")]
    NotPicture { flag: Flag },

    #[error("`{flag}` needs a file with pages: {id} has none counted")]
    NoPagesCounted { flag: Flag, id: String },

    #[error("`{flag}` needs a file with pages: {name} has no valid pages")]
    NoValidPages { flag: Flag, name: String },

    #[error("`{flag}` needs a file with pages: {id} is {}", .kind.short_name())]
    NotAPageFile {
        flag: Flag,
        id: String,
        kind: ObjectKind,
    },

    #[error("`{flag}` must be all, or pages and ranges such as 1-3,5, from page 1")]
    NotPages { flag: Flag },

    #[error("`{flag}` takes at most {most} pages")]
    TooManyPages { flag: Flag, most: usize },

    #[error("`{flag}` needs a file with a box on the board")]
    FileHasNoBox { flag: Flag },
}

fn number_list(numbers: &[u32]) -> String {
    numbers
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn right_list(rights: &[Right]) -> String {
    rights
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn flag_list(flags: &[Flag]) -> String {
    flags
        .iter()
        .map(|flag| format!("`{flag}`"))
        .collect::<Vec<_>>()
        .join(", ")
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
    use crate::{
        command::{
            board::{PictureFormat, rights::Right},
            object::add_args::AddKind,
            table::ValueKind,
        },
        domain::board::ObjectKind,
        error::{
            Error,
            flag::Flag,
            hint::decided::assert_all_listed,
            input::{InputError, InputErrorDiscriminants, JsonSpot},
            lookup::Candidates,
        },
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
        .into_iter()
        .chain(flag_samples())
        .chain(value_samples())
        .collect()
    }

    #[expect(clippy::too_many_lines, reason = "one sample per variant")]
    fn flag_samples() -> Vec<InputError> {
        vec![
            InputError::Conflict {
                flag: Flag::Offline,
                other: Flag::Refresh,
            },
            InputError::Requires {
                flag: Flag::Color,
                needs: Flag::Cell,
            },
            InputError::OneOf {
                flags: &[Flag::Name, Flag::Color],
            },
            InputError::InItem {
                slot: 2,
                error: Box::new(InputError::FullUrl),
            },
            InputError::NotFor {
                flag: Flag::From,
                kind: AddKind::Sticker,
            },
            InputError::Blank { flag: Flag::Name },
            InputError::NoHomeDirectory { flag: Flag::File },
            InputError::NoEarlierItem {
                flag: Flag::Frame,
                slot: 3,
            },
            InputError::NotKind {
                flag: Flag::Cell,
                kind: ValueKind::Integer,
            },
            InputError::NotAlignSide {
                raw: String::from("x"),
            },
            InputError::NotForSeveralObjects { flag: Flag::File },
            InputError::ArrowEndLeftOut {
                arrow: String::from("x"),
            },
            InputError::NotCopyable {
                object: String::from("x"),
                kind: ObjectKind::Group,
            },
            InputError::FileStaysOnBoard {
                flag: Flag::Into,
                object: String::from("x"),
                kind: ObjectKind::Image,
            },
            InputError::NotRight {
                flag: Flag::Right,
                allowed: &[Right::Viewer, Right::Editor],
            },
            InputError::NotProvider {
                flag: Flag::Provider,
            },
            InputError::NotOutput,
            InputError::StrokeWidth,
            InputError::Opacity,
            InputError::RotateNotFinite,
            InputError::ScaleNotPositive,
            InputError::WidthNotAutoOrPositive,
            InputError::TextSizeRange,
            InputError::NotRowColumn { flag: Flag::Cell },
            InputError::NotBlock { flag: Flag::Merge },
            InputError::CellNeedsTextOrColor,
            InputError::CellTextNeedsOneCell,
            InputError::CellTextAndColor,
            InputError::FacesNotDie,
            InputError::ReactionsMixed,
            InputError::TooManyReactions { most: 8 },
            InputError::NotItemNumber { flag: Flag::Check },
            InputError::NotDay { flag: Flag::Due },
            InputError::StartAfterDue,
            InputError::OneTitle { flag: Flag::Column },
            InputError::KanbanNone {
                flag: Flag::Swimlane,
            },
            InputError::TooManyRolls { most: 50 },
            InputError::PathNotPicture,
            InputError::AssigneeNeedsMember,
            InputError::ItemNeedsObject,
            InputError::PathWithSeveralObjects,
            InputError::OfflineNeedsBoardId,
            InputError::BoardWithNamesMode,
            InputError::KindNotNodeInNamesMode,
            InputError::KindNotObjectInTextMode,
            InputError::TrashWithBoardText,
            InputError::KindNotObjectWithBoard,
            InputError::TrashWithBoardOrObjectKind,
            InputError::CoverNotPicture,
            InputError::CoverNotInMemory,
            InputError::ObjectListToStdout,
            InputError::ImportNameNotFromFile,
            InputError::FileNotPictureName,
            InputError::PictureFormatNotName {
                found: PictureFormat::Svg,
            },
            InputError::NoPng,
            InputError::FolderWithView,
            InputError::CursorUnknown,
            InputError::SeveralBoards { flag: Flag::Frame },
            InputError::NoteFormatWithoutNote,
            InputError::CategoryWithoutStandardView,
            InputError::OthersReply,
            InputError::AtNotPoint,
            InputError::TimeNotIso,
        ]
    }

    fn value_samples() -> Vec<InputError> {
        let text = || String::from("x");
        vec![
            InputError::WrongObject {
                flag: Flag::Kanban,
                wanted: "a kanban",
            },
            InputError::NotATitle {
                flag: Flag::Column,
                titles: vec![text()],
            },
            InputError::UnknownStatus {
                flag: Flag::Status,
                ids: vec!["done"],
            },
            InputError::NotItemRef {
                flag: Flag::Frame,
                raw: text(),
            },
            InputError::ItemNotFrame {
                flag: Flag::Frame,
                slot: 2,
            },
            InputError::ArrowEndIsArrow {
                flag: Flag::From,
                slot: 2,
            },
            InputError::NotAmong {
                flag: Flag::Reaction,
                allowed: vec!["heart"],
            },
            InputError::NotAmongNumbers {
                flag: Flag::Faces,
                allowed: &[2, 4],
            },
            InputError::Below {
                flag: Flag::Rows,
                least: 1,
            },
            InputError::TooFew {
                flag: Flag::Point,
                least: 2,
            },
            InputError::ValueCount {
                flag: Flag::Choice,
                least: 2,
                most: 20,
            },
            InputError::LabelTooLong {
                flag: Flag::Choice,
                most: 40,
            },
            InputError::NotWhole {
                flag: Flag::StrokeWidth,
                least: 1,
                most: 30,
            },
            InputError::TooManyCells {
                flag: Flag::Rows,
                other: Flag::Columns,
                limit: 2500,
            },
            InputError::NotADay { flag: Flag::Due },
            InputError::NotBefore {
                flag: Flag::Start,
                other: Flag::Due,
            },
            InputError::NotIconName { flag: Flag::Text },
            InputError::NotSquare { flag: Flag::Size },
            InputError::NotStackSize { flag: Flag::Size },
            InputError::OneValue {
                flag: Flag::Reaction,
            },
            InputError::NotFile { flag: Flag::Path },
            InputError::NotPicture { flag: Flag::Path },
            InputError::NoPagesCounted {
                flag: Flag::Extract,
                id: text(),
            },
            InputError::NoValidPages {
                flag: Flag::Extract,
                name: text(),
            },
            InputError::NotAPageFile {
                flag: Flag::Extract,
                id: text(),
                kind: ObjectKind::Sticker,
            },
            InputError::NotPages {
                flag: Flag::Extract,
            },
            InputError::TooManyPages {
                flag: Flag::Extract,
                most: 50,
            },
            InputError::FileHasNoBox {
                flag: Flag::Extract,
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
