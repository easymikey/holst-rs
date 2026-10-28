//! The `board` group: boards and folders, their tree, changes and rights.

pub mod file;
pub mod own;
pub mod picture;
pub mod rights;
pub mod settings;
pub mod show;
pub mod tree;
pub mod tree_change;

use holst_macros::{ArgumentGroup, CommandArgs};
use serde::{Deserialize, Serialize};

use crate::{
    command::{
        board::rights::{OnOff, Right},
        table::{Command, CommandArgs, Confirmation, EnumArgument, NewFile, OneOrMany},
    },
    effect::board_write::WriteMode,
};

/// The verbs of `holst board`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum BoardVerb {
    Ls,
    Create,
    Show,
    Snap,
    Save,
    Export,
    Import,
    Edit,
    Rename,
    Mv,
    Cp,
    Rm,
    Restore,
    Favorite,
    Unfavorite,
    Touch,
    Leave,
    Grant,
    Revoke,
    Share,
    Transfer,
}

impl BoardVerb {
    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            BoardVerb::Ls => &LsArgs::COMMAND,
            BoardVerb::Create => &CreateArgs::COMMAND,
            BoardVerb::Show => &ShowArgs::COMMAND,
            BoardVerb::Snap => &SnapArgs::COMMAND,
            BoardVerb::Save => &SaveArgs::COMMAND,
            BoardVerb::Export => &ExportArgs::COMMAND,
            BoardVerb::Import => &ImportArgs::COMMAND,
            BoardVerb::Edit => &EditArgs::COMMAND,
            BoardVerb::Rename => &RenameArgs::COMMAND,
            BoardVerb::Mv => &MoveArgs::COMMAND,
            BoardVerb::Cp => &CopyArgs::COMMAND,
            BoardVerb::Rm => &RemoveArgs::COMMAND,
            BoardVerb::Restore => &RestoreArgs::COMMAND,
            BoardVerb::Favorite => &FavoriteArgs::COMMAND,
            BoardVerb::Unfavorite => &UnfavoriteArgs::COMMAND,
            BoardVerb::Touch => &TouchArgs::COMMAND,
            BoardVerb::Leave => &LeaveArgs::COMMAND,
            BoardVerb::Grant => &GrantArgs::COMMAND,
            BoardVerb::Revoke => &RevokeArgs::COMMAND,
            BoardVerb::Share => &ShareArgs::COMMAND,
            BoardVerb::Transfer => &TransferArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "ls",
    risk = Read,
    block = General,
    about = "List boards and folders as a tree or another view",
    help = "List boards and folders as a tree, or as another view.\n\
            It prints one line per node with its link.\n\
            --view recent is per account across workspaces; -w does not filter it.",
    example = "holst b ls -w easy",
    example = "holst b ls -w easy --folder \"Plans\" -l 20",
    example = "holst b ls --view recent"
)]
pub struct LsArgs {
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "folder",
        help = "List only this folder: a folder id, link or part of its name; / is the root; the whole workspace by default"
    )]
    pub folder: Option<String>,
    #[argument(
        value_name = "view",
        help = "Which boards: tree, trash, both, favorites, recent or sidebar; tree by default"
    )]
    pub view: Option<ListView>,
    #[argument(
        letter = 'l',
        value_name = "n",
        help = "At most this many nodes; 0 shows all; all by default"
    )]
    pub limit: Option<u32>,
    #[argument(
        value_name = "cursor",
        help = "Continue after this cursor, as the end of the previous page printed it; the first page by default"
    )]
    pub cursor: Option<String>,
}

/// Which boards and folders `board ls` lists.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ListView {
    #[default]
    Tree,
    Trash,
    Both,
    Favorites,
    Recent,
    Sidebar,
}

impl EnumArgument for ListView {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "create",
    risk = Write,
    block = General,
    about = "Create a board or a folder",
    help = "Create a board or a folder with this name.\n\
            It prints the new node with its link.",
    example = "holst b create \"Roadmap\" -w easy",
    example = "holst b create \"Plans\" -k folder -w easy"
)]
pub struct CreateArgs {
    #[argument(positional, help = "The name of the new board or folder")]
    pub name: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "The workspace a new thing goes to: id or part of its name; needed when you have several"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "folder",
        help = "The folder it goes into: a folder id, link or part of its name; / is the root; the workspace root by default"
    )]
    pub folder: Option<String>,
    #[argument(
        letter = 'k',
        value_name = "kind",
        help = "What to create: board or folder; board by default"
    )]
    pub kind: Option<NewKind>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// What `board create` makes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum NewKind {
    #[default]
    Board,
    Folder,
}

impl EnumArgument for NewKind {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "show",
    risk = Read,
    block = Look,
    about = "Show boards, or a part of one, with details",
    help = "Show one or more boards, or a part of one, with details.\n\
            It prints the name, size, owner and object counts, then the parts asked for with --with.\n\
            A link to a frame or an object shows that part. The area flags need exactly one board (else exit code 64).\n\
            With several boards, one that is not found is reported after the found ones are shown, and the exit code is not-found.",
    example = "holst b show \"Roadmap\"",
    example = "holst b show \"Roadmap\" --frame \"Q3\"",
    example = "holst b show \"Roadmap\" --with members,sharing"
)]
pub struct ShowArgs {
    #[argument(positional, help = "The boards to show: one, or several")]
    pub board: OneOrMany<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "parts",
        help = "Also show these parts, comma separated: members, rights, sharing; none by default"
    )]
    pub with: Vec<ShowPart>,
    #[serde(flatten)]
    #[argument(flatten)]
    pub selection: AreaArgs,
    #[serde(flatten)]
    #[argument(flatten)]
    pub caching: Caching,
}

/// A part of a board `board show` adds on request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ShowPart {
    Members,
    Rights,
    Sharing,
}

impl EnumArgument for ShowPart {}

/// The part of a board, shared by `show`, `snap` and `save`.
#[derive(Debug, Deserialize, ArgumentGroup)]
pub struct AreaArgs {
    #[argument(
        value_name = "x,y,w,h",
        help = "An area as a rectangle: x,y,width,height in board units"
    )]
    pub rectangle: Option<String>,
    #[argument(
        value_name = "frame",
        help = "An area as a frame: id, link, title or part of a title"
    )]
    pub frame: Option<String>,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(
        value_name = "units",
        help = "Padding around a frame or objects in board units; 16 and 40 by default"
    )]
    pub padding: Option<f64>,
}

/// How a board is read: from the cache or the network.
#[derive(Debug, Deserialize, ArgumentGroup)]
pub struct Caching {
    #[argument(help = "Ignore the cached copy and download the board again")]
    pub refresh: bool,
    #[argument(help = "No network: use the cached copy, or fail when there is none")]
    pub offline: bool,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "snap",
    risk = Read,
    block = Look,
    about = "Draw a board or a part of it as PNG or SVG",
    help = "Draw a board or a part of it as a PNG or SVG picture.\n\
            The file goes to ~/Downloads unless a file is given; it only reads the board.",
    example = "holst b snap \"Roadmap\"",
    example = "holst b snap \"Roadmap\" --frame \"Q3\" q3.png",
    example = "holst b snap \"Roadmap\" --object a1b2c3d4 --embed q3.svg"
)]
pub struct SnapArgs {
    #[argument(positional, help = "The board to draw")]
    pub board: String,
    #[argument(
        positional,
        value_name = "file",
        help = "A new .png or .svg file, never overwritten, - for stdout; ~/Downloads/<name>-<id>.png by default",
        surfaces = [Cli]
    )]
    pub file: Option<NewFile>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[serde(flatten)]
    #[argument(flatten)]
    pub selection: AreaArgs,
    #[serde(flatten)]
    #[argument(flatten)]
    pub caching: Caching,
    #[argument(
        value_name = "format",
        help = "The picture format: png or svg; from the file's extension, else png"
    )]
    pub format: Option<PictureFormat>,
    #[argument(
        value_name = "scale",
        help = "Pixels per board unit, lowered so the picture fits --max-side; 1 by default"
    )]
    pub scale: Option<f64>,
    #[argument(
        value_name = "px",
        help = "The longest side in pixels; 8192 by default"
    )]
    pub max_side: Option<u32>,
    #[argument(help = "Put the images into an SVG instead of links to the storage")]
    pub embed: bool,
}

/// The format of a picture.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PictureFormat {
    Png,
    Svg,
}

impl EnumArgument for PictureFormat {}

impl PictureFormat {
    /// The extension of the format's files.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            PictureFormat::Png => "png",
            PictureFormat::Svg => "svg",
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "save",
    risk = Write,
    block = Look,
    local_files,
    about = "Save a board or a part of it into a new folder",
    help = "Save a board or a part of it into a new folder: the picture, the SVG, the objects and the original images.\n\
            The folder is named <name>-<id> and goes to ~/Downloads unless --dir says otherwise.",
    example = "holst b save \"Roadmap\"",
    example = "holst b save \"Roadmap\" --frame \"Q3\" --dir ~/tmp"
)]
pub struct SaveArgs {
    #[argument(positional, help = "The board to save")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "dir",
        help = "The local folder the new folder goes into; ~/Downloads by default"
    )]
    pub dir: Option<String>,
    #[serde(flatten)]
    #[argument(flatten)]
    pub selection: AreaArgs,
    #[serde(flatten)]
    #[argument(flatten)]
    pub caching: Caching,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "export",
    risk = Write,
    block = Look,
    local_files,
    about = "Save the whole board as a board file",
    help = "Save the whole board as a board file, gzip by default; `holst b import` writes it back.\n\
            The file goes to ~/Downloads/<name>-<id>.ydoc.gz unless a file is given; it only reads the board.",
    example = "holst b export \"Roadmap\"",
    example = "holst b export \"Roadmap\" roadmap.ydoc --uncompressed --object-list"
)]
pub struct ExportArgs {
    #[argument(positional, help = "The board to save")]
    pub board: String,
    #[argument(
        positional,
        value_name = "file",
        help = "A new file, never overwritten and named as given, - for stdout; ~/Downloads/<name>-<id>.ydoc.gz by default",
        tool_doc = "export: a new file; import: a board file."
    )]
    pub file: Option<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(help = "Save without gzip; the default file name then has no .gz")]
    pub uncompressed: bool,
    #[argument(
        help = "Also save the objects beside the file, one per line: <file>.objects.jsonl"
    )]
    pub object_list: bool,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "import",
    risk = Destroy,
    block = Look,
    about = "Create a board from a board file, or write it into one",
    help = "Create a board from a board file, as the web app's Import does; it prints the new board with its link.\n\
            With --into it writes the file into an existing board instead, which needs -y.",
    example = "holst b import roadmap.ydoc.gz -w easy --name \"Roadmap copy\"",
    example = "holst b import roadmap.ydoc --into \"Roadmap\" --merge -y"
)]
pub struct ImportArgs {
    #[argument(
        positional,
        value_name = "file",
        help = "A board file from `holst b export` or `holst backup get`",
        tool_doc = "export: a new file; import: a board file."
    )]
    pub file: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "The workspace a new thing goes to: id or part of its name; needed when you have several"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "folder",
        help = "The folder the new board goes into: a folder id, link or part of its name; / is the root; the workspace root by default"
    )]
    pub folder: Option<String>,
    #[argument(
        value_name = "name",
        help = "The new board's name; after the file by default"
    )]
    pub name: Option<String>,
    #[argument(
        value_name = "board",
        help = "Write into this board instead of making one: id, link or the whole name, never a part of it"
    )]
    pub into: Option<String>,
    #[argument(
        help = "With --into, keep what is on the board and add the file's objects; refused otherwise"
    )]
    pub merge: bool,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "edit",
    risk = Write,
    block = Change,
    about = "Change a board's cover",
    help = "Change a board's cover; --cover is needed.\n\
            --cover reset returns to the automatic preview, drawn by holst.",
    example = "holst b edit \"Roadmap\" --cover cover.png",
    example = "holst b edit \"Roadmap\" --cover reset -n"
)]
pub struct EditArgs {
    #[argument(positional, help = "The board to change")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "file",
        help = "The cover: a picture file, or reset for the automatic preview"
    )]
    pub cover: String,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "rename",
    risk = Write,
    block = Change,
    about = "Rename a board or a folder",
    help = "Rename a board or a folder.\n\
            It prints the old and the new name.",
    example = "holst b rename \"Roadmap\" \"Roadmap 2027\"",
    example = "holst b rename \"Plans\" \"Old plans\" -n"
)]
pub struct RenameArgs {
    #[argument(positional, help = "The board or folder to rename")]
    pub board: String,
    #[argument(positional, help = "The new name")]
    pub name: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "mv",
    risk = Destroy,
    block = Change,
    about = "Move a board or a folder into a folder or /",
    help = "Move a board or a folder into a folder; / is the root. A folder takes its boards along.\n\
            Moving one board needs no -y. Moving a folder with boards reaches everyone who sees them, so it needs -y; without it only the dry run is shown.",
    example = "holst b mv \"Roadmap\" \"Plans\"",
    example = "holst b mv \"Plans\" / -y"
)]
pub struct MoveArgs {
    #[argument(positional, help = "The board or folder to move")]
    pub board: String,
    #[argument(
        positional,
        help = "The folder to move into: a folder id, link or part of its name; / is the root"
    )]
    pub folder: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "cp",
    risk = Write,
    block = Change,
    about = "Copy a board",
    help = "Copy a board; it prints the copy with its link.\n\
            The copy keeps the objects; its rights are not copied unless --rights is given.",
    example = "holst b cp \"Roadmap\"",
    example = "holst b cp \"Roadmap\" --name \"Roadmap Q4\" --folder \"Plans\" --rights"
)]
pub struct CopyArgs {
    #[argument(positional, help = "The board to copy")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "The workspace a new thing goes to: id or part of its name; needed when you have several"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "name",
        help = "The copy's name; \"<name>-copy\" by default, as in the web app"
    )]
    pub name: Option<String>,
    #[argument(
        value_name = "folder",
        help = "The folder the copy goes into: a folder id, link or part of its name; / is the root; the board's own folder by default"
    )]
    pub folder: Option<String>,
    #[argument(help = "Copy the board's rights too")]
    pub rights: bool,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "rm",
    risk = Destroy,
    block = Change,
    about = "Move a board to the trash, or delete it for good",
    help = "Move a board to the trash, or with --permanently delete it for good. A folder in the trash takes its boards along; a folder deleted for good leaves them in the folder it was in (or the root).\n\
            One board to the trash needs no -y. A folder with boards, and --permanently, need -y, since other people lose the boards; without it only the dry run is shown.",
    example = "holst b rm \"Old draft\"",
    example = "holst b rm \"Old draft\" --permanently -y",
    example = "holst b rm \"Old draft\" -n"
)]
pub struct RemoveArgs {
    #[argument(positional, help = "The board or folder to remove")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        help = "Delete for good: a board by its whole name, a folder by id or link"
    )]
    pub permanently: bool,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "restore",
    risk = Write,
    block = Change,
    about = "Bring a board or folder back from the trash",
    help = "Bring a board or a folder back from the trash.\n\
            Not yet tried against the live Holst server.",
    example = "holst b restore \"Old draft\"",
    example = "holst b ls --view trash"
)]
pub struct RestoreArgs {
    #[argument(positional, help = "The board or folder to bring back")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "favorite",
    risk = Write,
    block = Yours,
    about = "Add a board to favorites",
    help = "Add a board to your favorites.\n\
            It prints the board and its place in the favorites.",
    example = "holst b favorite \"Roadmap\"",
    example = "holst b favorite \"Roadmap\" --index 0"
)]
pub struct FavoriteArgs {
    #[argument(positional, help = "The board to add")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "n",
        help = "The index in the favorites; -1 is the end; -1 by default"
    )]
    pub index: Option<i64>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "unfavorite",
    risk = Write,
    block = Yours,
    about = "Remove a board from favorites",
    help = "Remove a board from your favorites.\n\
            Not yet tried against the live Holst server.",
    example = "holst b unfavorite \"Roadmap\"",
    example = "holst b ls --view favorites"
)]
pub struct UnfavoriteArgs {
    #[argument(positional, help = "The board to remove")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "touch",
    risk = Write,
    block = Yours,
    about = "Mark a board as recently opened",
    help = "Mark a board as recently opened, as opening it in the web app does.\n\
            It then leads `holst b ls --view recent`.",
    example = "holst b touch \"Roadmap\"",
    example = "holst b ls --view recent"
)]
pub struct TouchArgs {
    #[argument(positional, help = "The board to mark")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "leave",
    risk = Write,
    block = Yours,
    about = "Leave a board you were invited to",
    help = "Leave a board you were invited to; you lose your right on it.\n\
            Not yet tried against the live Holst server.",
    example = "holst b leave \"Roadmap\"",
    example = "holst b leave \"Roadmap\" -n"
)]
pub struct LeaveArgs {
    #[argument(positional, help = "The board to leave")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "grant",
    risk = Destroy,
    block = RightsAndSharing,
    about = "Give a user a right on a board",
    help = "Give a user a right on a board: viewer, commentator, editor or full.\n\
            It changes who can work on the board, so it needs -y; without it only the dry run is shown.",
    example = "holst b grant \"Roadmap\" anna --right editor -y",
    example = "holst b grant \"Roadmap\" anna --right viewer -n"
)]
pub struct GrantArgs {
    #[argument(positional, help = "The board to give the right on")]
    pub board: String,
    #[argument(
        positional,
        help = "A user: user id or part of a member's name (see `holst ws show --with members`)"
    )]
    pub user: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "right",
        help = "The board right: viewer, commentator, editor or full; required"
    )]
    pub right: Right,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "revoke",
    risk = Destroy,
    block = RightsAndSharing,
    about = "Take a user's right on a board away",
    help = "Take a user's right on a board away.\n\
            It changes who can work on the board, so it needs -y; without it only the dry run is shown.",
    example = "holst b revoke \"Roadmap\" anna -y",
    example = "holst b revoke \"Roadmap\" anna -n"
)]
pub struct RevokeArgs {
    #[argument(positional, help = "The board to take the right on")]
    pub board: String,
    #[argument(
        positional,
        help = "A user: user id or part of a member's name (see `holst ws show --with members`)"
    )]
    pub user: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "transfer",
    risk = Destroy,
    block = RightsAndSharing,
    about = "Make another user the owner of a board",
    help = "Make another user the owner of a board or a folder; you are no longer the owner.\n\
            It cannot be undone by you, so it needs -y; without it only the dry run is shown.",
    example = "holst b transfer \"Roadmap\" anna -y",
    example = "holst b transfer \"Plans\" anna --with-boards -n"
)]
pub struct TransferArgs {
    #[argument(positional, help = "The board or folder to hand over")]
    pub board: String,
    #[argument(
        positional,
        help = "The new owner: user id or part of a member's name (see `holst ws show --with members`)"
    )]
    pub user: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(help = "Also transfer the boards inside a folder")]
    pub with_boards: bool,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Board,
    verb = "share",
    risk = Destroy,
    block = RightsAndSharing,
    about = "Change who can open, edit and copy a board",
    help = "Change who can open, edit and copy a board. Only the flags you give change; read the current values with `b show --with sharing`.\n\
            It changes sharing, so it needs -y; without it only the dry run is shown. Paid-plan values are not yet tried against the live Holst server.",
    example = "holst b share \"Roadmap\" --link viewer -y",
    example = "holst b share \"Roadmap\" --members editor --copy full -y",
    example = "holst b share \"Roadmap\" --link none -n"
)]
pub struct ShareArgs {
    #[argument(positional, help = "The board to change")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "right",
        help = "Who can open the board by its link: none, viewer, commentator or editor; unchanged by default"
    )]
    pub link: Option<Right>,
    #[argument(
        value_name = "right",
        help = "What workspace members can do: none, viewer, commentator, editor or full; unchanged by default"
    )]
    pub members: Option<Right>,
    #[argument(
        value_name = "right",
        help = "Who can copy the content: viewer, editor or full; unchanged by default"
    )]
    pub copy: Option<Right>,
    #[argument(
        value_name = "right",
        help = "Who can delete objects: editor or full; unchanged by default"
    )]
    pub delete: Option<Right>,
    #[argument(
        value_name = "right",
        help = "Who can use the timer, voting and bring to me: editor or full; unchanged by default"
    )]
    pub tools: Option<Right>,
    #[argument(
        value_name = "right",
        help = "Who can lock and hide the content of a frame: editor or full; unchanged by default"
    )]
    pub lock: Option<Right>,
    #[argument(
        value_name = "state",
        help = "The public template link: on or off; unchanged by default"
    )]
    pub template: Option<OnOff>,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}
