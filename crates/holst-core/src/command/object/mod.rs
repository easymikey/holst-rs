//! The `object` group: the objects on a board.

pub mod add;
pub mod change;
pub mod copy;
pub mod get;
pub mod list;

use holst_macros::CommandArgs;
use serde::Deserialize;

use crate::{
    command::{
        board::{AreaArgs, Caching},
        table::{Command, CommandArgs, Confirmation, EnumArgument, Items, NewFile},
    },
    domain::{change::value::LayerOrder, rich_text::TextFormat},
    effect::board_write::WriteMode,
};

/// The verbs of `holst object`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum ObjectVerb {
    Ls,
    Add,
    Edit,
    Cp,
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
            It prints one line per object with @N, kind, corner, size and text.",
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
        help = "Only objects whose text holds this, ignoring case and ё",
        tool_doc = "Text holds this."
    )]
    pub contains: Option<String>,
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

/// What `object add` makes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AddKind {
    #[default]
    Sticker,
    Text,
    Shape,
    Frame,
    Arrow,
    Link,
    Image,
    File,
}

impl EnumArgument for AddKind {}

/// The outline of a new shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum NewShape {
    Square,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Diamond,
}

impl EnumArgument for NewShape {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "add",
    risk = Write,
    block = Commands,
    about = "Add stickers, texts, shapes, frames, arrows, links, images or files to a board",
    help = "Add objects of one kind to a board; the kind word is the default kind of every --items item.\n\
            Without --at each goes to a free corner, in --frame if given, else in the frame a linked object sits in. It prints one line per object with @N and its link.\n\
            An image or a file is uploaded first; a link card takes its title from the page. With --items, #N names an earlier item as an arrow end, and one write holds all of them.",
    example = "holst o add sticker \"Roadmap\" \"Ask design\" -c yellow4",
    example = "holst o add shape \"Roadmap\" \"Launch\" --shape diamond",
    example = "holst o add frame \"Roadmap\" \"Q4\" --items q4.json",
    example = "holst o add arrow \"Roadmap\" --from a1b2c3d4 --to e5f6a7b8",
    example = "holst o add link \"Roadmap\" https://example.com",
    example = "holst o add image \"Roadmap\" mock.png --size 400x300"
)]
pub struct AddArgs {
    #[argument(
        subcommand,
        value_name = "kind",
        help = "What to add: sticker, text, shape, frame, arrow, link, image or file",
        tool_doc = "Sticker by default."
    )]
    pub kind: Option<AddKind>,
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "text",
        positional,
        help = "The text; lines become paragraphs; the title of a frame",
        tool_doc = "Lines are paragraphs.",
        kinds = [sticker, text, shape, frame]
    )]
    pub text: Option<String>,
    #[argument(
        value_name = "format",
        help = "How the text is read: plain, or markdown for headings, lists, checkboxes, links and bold, italic, strike marks; plain by default",
        tool_doc = "plain or markdown.",
        kinds = [sticker, text, shape]
    )]
    pub text_format: Option<TextFormat>,
    #[argument(
        positional,
        help = "The page to link to, e.g. https://example.com",
        tool_doc = "Page address.",
        kinds = [link]
    )]
    pub url: Option<String>,
    #[argument(
        value_name = "file",
        positional,
        help = "The local file to upload",
        tool_doc = "Local file.",
        kinds = [image, file]
    )]
    pub path: Option<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "shape",
        help = "The outline: square, rounded-rectangle, ellipse, triangle or diamond; square by default",
        kinds = [shape]
    )]
    pub shape: Option<NewShape>,
    #[argument(
        value_name = "x,y",
        help = "The top-left corner in board units, from the frame's corner with --frame; a free corner by default",
        tool_doc = "x,y; add, cp: a free corner by default; edit: unchanged.",
        kinds = [sticker, text, shape, frame, link, image, file]
    )]
    pub at: Option<String>,
    #[argument(
        value_name = "wxh",
        help = "The size as width x height, e.g. 260x160; the kind's own size by default; an image's own size, at most 1000 on the long side, by default",
        tool_doc = "WxH.",
        kinds = [sticker, shape, frame, image]
    )]
    pub size: Option<String>,
    #[argument(
        letter = 'c',
        value_name = "color",
        help = "The fill, or an arrow's line color: a palette token, e.g. yellow4; the kind's own color by default",
        tool_doc = "E.g. yellow4.",
        kinds = [sticker, text, shape, frame, arrow]
    )]
    pub color: Option<String>,
    #[argument(
        help = "Keep the card plain even when the page can be embedded",
        tool_doc = "Plain card.",
        kinds = [link]
    )]
    pub no_embed: bool,
    #[argument(
        value_name = "frame",
        help = "Put it inside this frame: id, link, title or part of a title; --at counts from its corner"
    )]
    pub frame: Option<String>,
    #[argument(
        value_name = "object",
        help = "The object an arrow starts at: id, link, @N or the first 8 characters of its id; with --items also #N, an earlier item",
        tool_doc = "Or #N: an earlier item.",
        kinds = [arrow]
    )]
    pub from: Option<String>,
    #[argument(
        value_name = "object",
        help = "The object an arrow points to: id, link, @N or the first 8 characters of its id; with --items also #N, an earlier item",
        tool_doc = "Arrow end; cp: the target board.",
        kinds = [arrow]
    )]
    pub to: Option<String>,
    #[argument(
        value_name = "file",
        help = "Many in one write: a JSON array or JSON lines file, - for stdin; its keys are the flags of one call",
        tool_doc = "Many at once, each with the arguments of one; at most 100 per call."
    )]
    pub items: Option<Items<AddItem>>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// One object of `--items` of `object add`: the arguments of one call.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddItem {
    pub kind: Option<AddKind>,
    pub text: Option<String>,
    pub shape: Option<NewShape>,
    pub at: Option<String>,
    pub size: Option<String>,
    pub color: Option<String>,
    pub frame: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub no_embed: Option<bool>,
}

impl EnumArgument for LayerOrder {}

impl EnumArgument for TextFormat {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "edit",
    risk = Write,
    block = Commands,
    about = "Change text, corner, size, color, order or tags of objects",
    help = "Change text, corner, size, color, order or tags of objects. The objects are the linked object and every --object.\n\
            Moving a frame moves its content. It prints one line per changed object with @N and its link.",
    example = "holst o edit \"Roadmap\" --object a1b2c3d4 --text \"Ask design by Friday\"",
    example = "holst o edit \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8 -c yellow4 --order front",
    example = "holst o edit \"Roadmap\" --items fixes.json"
)]
pub struct EditArgs {
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
        value_name = "text",
        help = "The new text; lines become paragraphs; unchanged by default",
        tool_doc = "Lines are paragraphs."
    )]
    pub text: Option<String>,
    #[argument(
        value_name = "format",
        help = "How the new text is read: plain, or markdown for headings, lists, checkboxes, links and bold, italic, strike marks; plain by default",
        tool_doc = "plain or markdown."
    )]
    pub text_format: Option<TextFormat>,
    #[argument(
        value_name = "x,y",
        help = "The new top-left corner in board units; unchanged by default",
        tool_doc = "x,y; add, cp: a free corner by default; edit: unchanged."
    )]
    pub at: Option<String>,
    #[argument(
        value_name = "wxh",
        help = "The new size as width x height, e.g. 260x160; unchanged by default",
        tool_doc = "WxH."
    )]
    pub size: Option<String>,
    #[argument(
        letter = 'c',
        value_name = "color",
        help = "The new fill, or an arrow's line color: a palette token, e.g. yellow4; unchanged by default",
        tool_doc = "E.g. yellow4."
    )]
    pub color: Option<String>,
    #[argument(
        value_name = "order",
        help = "Where it stands among the objects: front or back; unchanged by default"
    )]
    pub order: Option<LayerOrder>,
    #[argument(
        value_name = "tag",
        help = "The object's tags, repeat for several; a new tag is made, '' clears; unchanged by default"
    )]
    pub tag: Vec<String>,
    #[argument(
        value_name = "file",
        help = "Many in one write: a JSON array or JSON lines file, - for stdin; its keys are the flags of one call",
        tool_doc = "Many at once, each with the arguments of one; at most 100 per call."
    )]
    #[serde(default)]
    pub items: Items<EditItem>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// One object of `--items` of `object edit`: the arguments of one call.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditItem {
    pub object: Option<String>,
    pub text: Option<String>,
    pub at: Option<String>,
    pub size: Option<String>,
    pub color: Option<String>,
    pub order: Option<LayerOrder>,
    #[serde(default)]
    pub tag: Vec<String>,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "cp",
    risk = Write,
    block = Commands,
    about = "Copy objects on their board or onto another board",
    help = "Copy objects on their board, or with --to onto another board; the copies get new ids and keep their content.\n\
            They go to a free corner unless --at gives the corner of the copied group. Images and files are uploaded again for another board. It prints one line per copy with @N and its link.",
    example = "holst o cp \"Roadmap\" --object a1b2c3d4 --object e5f6a7b8",
    example = "holst o cp \"Roadmap\" --object a1b2c3d4 --at 800,0",
    example = "holst o cp \"Roadmap\" --object a1b2c3d4 --to \"Retro\""
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
        help = "Copy onto this board: id, link or part of its name; the same board by default",
        tool_doc = "Arrow end; cp: the target board."
    )]
    pub to: Option<String>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "x,y",
        help = "The top-left corner of the copied group in board units; a free corner by default",
        tool_doc = "x,y; add, cp: a free corner by default; edit: unchanged."
    )]
    pub at: Option<String>,
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
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
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
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use strum::IntoEnumIterator as _;

    use crate::{
        Result,
        command::{
            fixture::arguments,
            object::{AddArgs, AddKind, EditArgs, ObjectVerb},
            table::{CommandArgs, ValueKind},
        },
    };

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
        let limited: Vec<_> = AddArgs::COMMAND
            .arguments
            .iter()
            .filter(|argument| !argument.kinds.is_empty())
            .map(|argument| (argument.name, argument.kinds))
            .collect();
        assert_eq!(
            (subcommands, limited),
            (
                vec![(
                    ObjectVerb::Add,
                    "kind",
                    ValueKind::Choice(&[
                        "sticker", "text", "shape", "frame", "arrow", "link", "image",
                        "file"
                    ])
                )],
                vec![
                    ("text", &["sticker", "text", "shape", "frame"][..]),
                    ("text_format", &["sticker", "text", "shape"][..]),
                    ("url", &["link"][..]),
                    ("path", &["image", "file"][..]),
                    ("shape", &["shape"][..]),
                    (
                        "at",
                        &["sticker", "text", "shape", "frame", "link", "image", "file"]
                            [..]
                    ),
                    ("size", &["sticker", "shape", "frame", "image"][..]),
                    ("color", &["sticker", "text", "shape", "frame", "arrow"][..]),
                    ("no_embed", &["link"][..]),
                    ("from", &["arrow"][..]),
                    ("to", &["arrow"][..]),
                ]
            )
        );
    }

    #[test]
    fn the_keys_of_an_item_are_flags_of_one_call() {
        const ADD_KEYS: [&str; 12] = [
            "kind", "text", "shape", "at", "size", "color", "frame", "from", "to",
            "url", "path", "no_embed",
        ];
        const EDIT_KEYS: [&str; 7] =
            ["object", "text", "at", "size", "color", "order", "tag"];
        let strays: Vec<_> = [
            (AddArgs::COMMAND.arguments, &ADD_KEYS[..]),
            (EditArgs::COMMAND.arguments, &EDIT_KEYS[..]),
        ]
        .into_iter()
        .flat_map(|(arguments, keys)| {
            keys.iter().filter(move |key| {
                arguments.iter().all(|argument| argument.name != **key)
            })
        })
        .collect();
        assert!(strays.is_empty(), "{strays:?}");
    }

    #[test]
    fn an_add_reads_its_kind_items_and_kind_only_arguments() -> Result<()> {
        let arrow: AddArgs = arguments(json!({
            "kind": "arrow", "board": "A", "from": "@1", "to": "@2",
            "items": [{"kind": "frame", "text": "Q4"}, {"text": "x"}]
        }))?;
        let plain: AddArgs = arguments(json!({ "board": "A" }))?;
        assert_eq!(
            (
                arrow.kind,
                arrow.items.as_ref().map(|items| items.0.len()),
                arrow
                    .items
                    .as_ref()
                    .and_then(|items| items.0.first())
                    .and_then(|item| item.kind),
                plain.kind.unwrap_or_default(),
                plain.items.as_ref().map(|items| items.0.len())
            ),
            (
                Some(AddKind::Arrow),
                Some(2),
                Some(AddKind::Frame),
                AddKind::Sticker,
                None
            )
        );
        let refused = arguments::<AddArgs>(json!({
            "kind": "sticker", "board": "A", "from": "@1"
        }))
        .err()
        .map(|error| error.to_string());
        assert!(
            refused.is_some_and(|text| text.starts_with(
                "\"from\" is not an argument of `object add sticker`: it takes kind, board, text"
            ))
        );
        Ok(())
    }
}
