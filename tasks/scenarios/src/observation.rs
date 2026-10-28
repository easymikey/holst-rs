//! What a surface showed, in terms both surfaces share.

use serde_json::Value;

use crate::whole::ShownBoard;

/// A workspace of the login; the counts are `None` when not counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountedWorkspace {
    pub name: String,
    pub boards: Option<usize>,
    pub folders: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Observation {
    Workspaces(Vec<String>),
    /// A change went through.
    Done,
    /// A change was only shown: the request it would have sent.
    DryRun {
        path: String,
        body: Value,
    },
    /// What a read showed, as the surface printed it.
    Shown(String),
    /// A right to take away was not held; nothing was sent.
    NotHeld,
    /// The login is accepted: whose it is, and the workspaces it reaches.
    LoggedIn {
        name: String,
        workspaces: Vec<CountedWorkspace>,
    },
    NotLoggedIn,
    /// The server refused the saved login; `words` is what the user was told.
    LoginNotAccepted {
        message: String,
    },
    Objects(Vec<ListedObject>),
    /// A page of the tree of boards and folders.
    Tree(ListedTree),
    /// What a search found: names of boards and folders, and text on boards.
    Matches(Vec<ListedMatch>),
    /// A board was saved as a folder; these originals could not be saved.
    Saved {
        not_saved: Vec<String>,
    },
    /// A picture of a board, in the format it came in.
    Picture(Option<PictureFormat>),
    /// The boards a `show` told of, the references not found, and a bad `exit`.
    Boards {
        shown: Vec<ShownBoard>,
        missing: usize,
        exit: Option<ErrorExit>,
    },
    /// The size of a picture in pixels.
    Size {
        width: u32,
        height: u32,
    },
    Failed {
        message: String,
        hint: String,
        signal: ErrorExit,
    },
}

impl Observation {
    /// Returns whose login it is and the workspaces it reaches, when accepted.
    pub(crate) fn login(&self) -> Option<(&str, &[CountedWorkspace])> {
        if let Self::LoggedIn { name, workspaces } = self {
            Some((name, workspaces))
        } else {
            None
        }
    }

    /// Returns the objects a list showed, when it showed some.
    pub(crate) fn objects(&self) -> Option<&[ListedObject]> {
        if let Self::Objects(rows) = self {
            Some(rows)
        } else {
            None
        }
    }

    /// Returns the matches a search showed, when it showed some.
    pub(crate) fn found(&self) -> Option<&[ListedMatch]> {
        if let Self::Matches(found) = self {
            Some(found)
        } else {
            None
        }
    }

    /// Returns the page of the tree a listing showed, when it showed one.
    pub(crate) fn tree(&self) -> Option<&ListedTree> {
        if let Self::Tree(page) = self {
            Some(page)
        } else {
            None
        }
    }

    /// Returns what a read showed, when it showed something.
    pub(crate) fn shown(&self) -> Option<&str> {
        if let Self::Shown(text) = self {
            Some(text)
        } else {
            None
        }
    }
}

/// A page of the tree: a row per node, indented two spaces for each folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTree {
    pub rows: Vec<String>,
    pub cursor: Option<String>,
}

/// How a surface says that it failed, which each surface does in its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorExit {
    ExitCode(i32),
    Killed,
    ToolError,
}

/// One object of a board as a surface lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedObject {
    /// The full id on the CLI, the `@N` alias on MCP; `ends` use the same.
    pub id: String,
    pub kind: String,
    pub text: String,
    /// The top-left corner as `x,y`, rounded.
    pub corner: String,
    pub in_frame: bool,
    /// Shown with the lock flag.
    pub locked: bool,
    /// The clockwise degrees it is shown turned by, when not 0.
    pub rotation: Option<String>,
    /// The address or object link it opens, when it has one.
    pub link_to: Option<String>,
    /// The size as `WxH`, when the object has one.
    pub size: Option<String>,
    pub color: Option<String>,
    /// A shape's kind and outline in the words of a listing line.
    pub look: Option<String>,
    pub tags: Vec<String>,
    /// Marks and colors of the whole text: `bold`, `text:red6`, `bg:yellow4`.
    pub text_look: Vec<String>,
    /// Where an arrow starts and ends, when attached.
    pub ends: Option<(Option<String>, Option<String>)>,
}

/// One hit of a text search across boards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTextHit {
    pub board: String,
    pub kind: String,
    pub text: String,
}

/// One match of a search: a board or folder by its name, or text on a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListedMatch {
    Name(ListedName),
    Text(ListedTextHit),
}

/// A board or folder a search found by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedName {
    pub name: String,
    /// `board` or `folder`.
    pub kind: String,
    pub in_trash: bool,
}

/// The format of a picture a surface gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureFormat {
    Svg,
    Png,
}

impl PictureFormat {
    /// Returns the format the bytes are in, if they are a picture.
    pub(crate) fn of(bytes: &[u8]) -> Option<Self> {
        match bytes {
            png if png.starts_with(b"\x89PNG") => Some(Self::Png),
            text if String::from_utf8_lossy(text).contains("<svg") => Some(Self::Svg),
            _ => None,
        }
    }

    /// Returns the extension of the format's files.
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }

    /// Returns the format a media type names.
    pub(crate) fn named(media_type: &str) -> Option<Self> {
        match media_type {
            "image/png" => Some(Self::Png),
            "image/svg+xml" => Some(Self::Svg),
            _ => None,
        }
    }
}
