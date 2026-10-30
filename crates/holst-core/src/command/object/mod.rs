//! The `object` group: the objects on a board.

pub mod add;
pub mod add_args;
pub mod align;
pub mod change;
pub mod copy;
pub mod edit_args;
mod extract;
pub mod get;
pub mod group;
pub mod list;
pub mod lock;

pub use add_args::{AddArgs, AddFields, AddItem, AddKind};
pub use edit_args::{EditArgs, EditFields, EditItem};
use holst_macros::CommandArgs;
use serde::Deserialize;

use crate::{
    command::{
        board::{AreaArgs, Caching},
        table::{Command, CommandArgs, Confirmation, EnumArgument, NewFile},
    },
    domain::{
        board::{
            arrow_look::ArrowType,
            frame_look::{FrameBackground, FrameRatio},
            shape_type::{ShapeType, StrokeStyle},
        },
        change::{
            media::{CropMask, FileDisplay, FlipState},
            value::{Axis, LayerOrder},
        },
        rich_text::TextFormat,
    },
    effect::board_write::WriteMode,
};

/// The verbs of `holst object`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum ObjectVerb {
    Ls,
    Add,
    Edit,
    Cp,
    Group,
    Ungroup,
    Lock,
    Unlock,
    Align,
    Rm,
    Get,
}

impl ObjectVerb {
    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            ObjectVerb::Ls => &LsArgs::COMMAND,
            ObjectVerb::Add => &AddArgs::COMMAND,
            ObjectVerb::Edit => &EditArgs::COMMAND,
            ObjectVerb::Cp => &CopyArgs::COMMAND,
            ObjectVerb::Group => &GroupArgs::COMMAND,
            ObjectVerb::Ungroup => &UngroupArgs::COMMAND,
            ObjectVerb::Lock => &LockArgs::COMMAND,
            ObjectVerb::Unlock => &UnlockArgs::COMMAND,
            ObjectVerb::Align => &AlignArgs::COMMAND,
            ObjectVerb::Rm => &RemoveArgs::COMMAND,
            ObjectVerb::Get => &GetArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "ls",
    risk = Read,
    block = Commands,
    about = "List the objects of a board or a part of it",
    help = "List the objects of a board or a part of it.\n\
            It prints one line per object with @N, kind, corner x,y, size and text.",
    example = "holst o ls \"Roadmap\" -k sticker --contains \"design\"",
    example = "holst o ls \"Roadmap\" --frame \"Q3\" -l 20",
    example = "holst o ls \"Roadmap\" --tag urgent",
    example = "holst o ls \"Roadmap\" --offset 20 -l 20"
)]
pub struct LsArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
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
        letter = 'k',
        value_name = "kind",
        help = "Only objects of this kind: sticker, text, shape, frame, arrow, image…"
    )]
    pub kind: Option<String>,
    #[argument(
        value_name = "text",
        help = "Only objects whose text holds this, ignoring case and ё"
    )]
    pub contains: Option<String>,
    #[argument(
        value_name = "format",
        help = "How the text is shown: plain, or markdown for headings, lists, checkboxes, links, mentions and marks; plain by default"
    )]
    pub text_format: Option<TextFormat>,
    #[argument(
        value_name = "tag",
        help = "Only objects with this tag; every object by default"
    )]
    pub tag: Option<String>,
    #[argument(value_name = "n", help = "Objects to skip; 0 by default")]
    pub offset: Option<u32>,
    #[argument(
        letter = 'l',
        value_name = "n",
        help = "At most this many objects; 0 shows all; all by default"
    )]
    pub limit: Option<u32>,
}

impl EnumArgument for ShapeType {}

impl EnumArgument for StrokeStyle {}

impl EnumArgument for FrameRatio {}

impl EnumArgument for FrameBackground {}

impl EnumArgument for ArrowType {}
impl EnumArgument for LayerOrder {}

impl EnumArgument for Axis {}

impl EnumArgument for TextFormat {}

impl EnumArgument for CropMask {}
impl EnumArgument for FileDisplay {}

impl EnumArgument for FlipState {}

/// A mark of the whole text, or `none` for no marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MarkChoice {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    None,
}

impl EnumArgument for MarkChoice {}
#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "cp",
    risk = Write,
    block = Commands,
    about = "Copy objects on their board or onto another board",
    help = "Copy objects on their board, or with --into onto another board; the copies get new ids and keep their content.\n\
            They go to a free corner unless --at gives the corner of the copied group. Images and files are uploaded again for another board. It prints one line per copy with @N and its link.",
    example = "holst o cp \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8",
    example = "holst o cp \"Roadmap\" --object a1b2c3d4 --at 800,0",
    example = "holst o cp \"Roadmap\" --object a1b2c3d4 --into \"Retro\""
)]
pub struct CopyArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(
        value_name = "board",
        help = "Copy onto this board: id, link or part of its name; the same board by default"
    )]
    pub into: Option<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "x,y",
        help = "The top-left corner of the copied group in board units; a free corner by default",
        tool_doc = "x,y; add, cp: a free corner by default."
    )]
    pub at: Option<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "group",
    risk = Write,
    block = Commands,
    about = "Put objects into a group",
    help = "Put objects into one new group, so that they are picked and moved together. The objects are the linked object and every --object, at least two.\n\
            Frames, table cells, mind-map nodes, cards in a kanban column and objects already in a group stay out. It prints one line per changed object with @N and its link.",
    example = "holst o group \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8",
    example = "holst o group \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8 -n"
)]
pub struct GroupArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
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
    group = Object,
    verb = "ungroup",
    risk = Write,
    block = Commands,
    about = "Dissolve groups; their objects stay",
    help = "Dissolve groups: their objects stay where they are, in the group's frame if it stood in one. The groups are the linked object and every --object.\n\
            A group left with fewer than 2 objects by `o rm` dissolves by itself. It prints one line per changed object with @N and its link.",
    example = "holst o ungroup \"Roadmap\" --object a1b2c3d4",
    example = "holst o ungroup \"Roadmap\" --object a1b2c3d4 -n"
)]
pub struct UngroupArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "A group, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
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
    group = Object,
    verb = "lock",
    risk = Write,
    block = Commands,
    about = "Lock objects so that they cannot be changed",
    help = "Lock objects: they are put into one new locked group, which `o edit` and `o rm` then refuse to touch until `o unlock`. The objects are the linked object and every --object.\n\
            With --only a frame or a table is locked by itself, its content stays free; nothing else can be locked that way. Needs the board's right to manage locks. Table cells and mind-map nodes cannot be locked. It prints one line per changed object with @N and its link.",
    example = "holst o lock \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8",
    example = "holst o lock \"Roadmap\" --object a1b2c3d4 --only -n"
)]
pub struct LockArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(help = "Lock a frame or a table itself, not what is in it")]
    pub only: bool,
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
    group = Object,
    verb = "unlock",
    risk = Write,
    block = Commands,
    about = "Unlock locked groups, frames and tables",
    help = "Unlock what `o lock` locked: a locked group is dissolved and its objects stay, a locked frame or table is unlocked. The objects are the linked object and every --object; --all unlocks everything locked on the board.\n\
            Needs the board's right to manage locks. It prints one line per changed object with @N and its link.",
    example = "holst o unlock \"Roadmap\" --object a1b2c3d4",
    example = "holst o unlock \"Roadmap\" --all -n"
)]
pub struct UnlockArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "A locked group, frame or table, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(help = "Unlock everything locked on the board")]
    pub all: bool,
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
    group = Object,
    verb = "align",
    risk = Write,
    block = Commands,
    about = "Align objects to a side or spread them evenly",
    help = "Align objects to one side of the box they cover together, or spread them so that the gaps between are equal and the outer two stay. The objects are the linked object and every --object: at least two to align, three to spread.\n\
            Each moves with what it holds. A locked object, a table cell, a mind-map node or a card in a kanban column refuses the whole call; an arrow stuck to an object follows it. It prints one line per changed object with @N and its link.",
    example = "holst o align \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8 --side left",
    example = "holst o align \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8 --object 9f8e7d6c --distribute horizontal -n"
)]
pub struct AlignArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(
        value_name = "side",
        help = "Line the objects up against this side of the box they cover together: left, center, right, top, middle or bottom; center is between left and right, middle between top and bottom"
    )]
    pub side: Option<String>,
    #[argument(
        value_name = "axis",
        help = "Spread the objects so that the gaps between them are equal: horizontal or vertical; use instead of --side",
        tool_doc = "Spread evenly; align instead with side: left, center, right, top, middle, bottom."
    )]
    pub distribute: Option<Axis>,
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
    group = Object,
    verb = "rm",
    risk = Destroy,
    block = Commands,
    about = "Remove objects with what a frame holds and their arrows",
    help = "Remove objects with what a frame holds and their arrows. The objects are the linked object and every --object.\n\
            One object needs no -y. Several objects, or a frame with its content, need -y, since other people see them go; without it only the dry run is shown.",
    example = "holst o rm \"Roadmap\" --object a1b2c3d4",
    example = "holst o rm \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8 -y"
)]
pub struct RemoveArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; a terminal asks instead; without one only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "get",
    risk = Write,
    block = Commands,
    local_files,
    about = "Download the original of an image or file object",
    help = "Download the original of an image or file object.\n\
            The file goes to ~/Downloads under its own name unless a file or --dir says otherwise; it only reads the board.",
    example = "holst o get \"Roadmap\" --object a1b2c3d4",
    example = "holst o get \"Roadmap\" --object a1b2c3d4 mock.png"
)]
pub struct GetArgs {
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "file",
        positional,
        help = "A new file, never overwritten, - for stdout; the object's own name in ~/Downloads by default",
        tool_doc = "Where to save it."
    )]
    pub file: Option<NewFile>,
    #[argument(
        value_name = "object",
        help = "An object: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "dir",
        help = "The local folder it goes into under its own name; ~/Downloads by default"
    )]
    pub dir: Option<String>,
    #[argument(help = "Ignore the cached copy and download the board again")]
    pub refresh: bool,
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use crate::command::{object::ObjectVerb, table::ValueKind};

    #[test]
    fn only_add_has_a_subcommand_and_it_names_the_kinds() {
        let subcommands: Vec<_> = ObjectVerb::iter()
            .flat_map(|verb| {
                verb.command()
                    .arguments
                    .iter()
                    .filter(|argument| argument.subcommand)
                    .map(move |argument| (verb, argument.name, argument.kind))
            })
            .collect();
        assert_eq!(
            subcommands,
            vec![(
                ObjectVerb::Add,
                "kind",
                ValueKind::Choice(&[
                    "sticker",
                    "text",
                    "shape",
                    "frame",
                    "arrow",
                    "link",
                    "image",
                    "file",
                    "dice",
                    "wheel",
                    "sticker-stack",
                    "flipcard",
                    "mind-map",
                    "document",
                    "stamp",
                    "code",
                    "table",
                    "reaction-stamp",
                    "icon",
                    "task",
                    "kanban",
                    "drawing"
                ])
            )]
        );
    }
}
