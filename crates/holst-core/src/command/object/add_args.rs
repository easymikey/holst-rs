//! The arguments of `object add`.

use holst_macros::{ArgumentGroup, CommandArgs};
use itertools::Itertools as _;
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::{Map, Value};

use crate::{
    command::table::{ArgumentGroup as _, EnumArgument, Items},
    domain::{
        board::{
            arrow_look::ArrowType,
            frame_look::{FrameBackground, FrameRatio},
            shape_type::{ShapeType, StrokeStyle},
        },
        kinds::mind_map::Direction,
        rich_text::TextFormat,
    },
    effect::board_write::WriteMode,
};

/// What `object add` makes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, strum::VariantNames,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
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
    Dice,
    Wheel,
    StickerStack,
    Flipcard,
    MindMap,
}

impl EnumArgument for AddKind {}

impl EnumArgument for Direction {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "add",
    risk = Write,
    block = Commands,
    about = "Add stickers, texts, shapes, frames, arrows, links, images, files, dice, a spinner wheel, sticker stacks, flip cards or mind-map nodes to a board",
    help = "Add objects of one kind to a board; the kind word is the default kind of every --items item.\n\
            Without --at each goes to a free corner, in --frame if given, else in the frame a linked object sits in. It prints one line per object with @N and its link.\n\
            An image or a file is uploaded first; a link card takes its title from the page. With --items, #N names an earlier item as an arrow end, and one write holds all of them.",
    example = "holst o add sticker \"Roadmap\" \"Ask design\" -c yellow4",
    example = "holst o add shape \"Roadmap\" \"Launch\" --shape diamond",
    example = "holst o add frame \"Roadmap\" \"Q4\" --items q4.json",
    example = "holst o add arrow \"Roadmap\" --from a1b2c3d4 --to e5f6a7b8",
    example = "holst o add link \"Roadmap\" https://example.com",
    example = "holst o add image \"Roadmap\" mock.png --size 400,300",
    example = "holst o add dice \"Roadmap\" --faces 20",
    example = "holst o add wheel \"Roadmap\" --choice Pizza --choice Sushi",
    example = "holst o add sticker-stack \"Roadmap\" Ideas --size 384,192",
    example = "holst o add flipcard \"Roadmap\" Question --back Answer",
    example = "holst o add mind-map \"Roadmap\" \"Budget\" --parent @3 --direction down"
)]
pub struct AddArgs {
    #[argument(
        subcommand,
        value_name = "kind",
        help = "What to add: sticker, text, shape, frame, arrow, link, image, file, dice, wheel, sticker-stack, flipcard or mind-map",
        tool_doc = "Sticker by default."
    )]
    pub kind: Option<AddKind>,
    #[argument(positional, help = "The board: id, link or part of its name")]
    pub board: String,
    #[argument(
        value_name = "format",
        help = "How the text is read: plain, or markdown for headings, lists, checkboxes, links and bold, italic, strike marks; plain by default",
        tool_doc = "plain or markdown.",
        kinds = [sticker, text, shape, mind_map]
    )]
    pub text_format: Option<TextFormat>,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "frame",
        help = "Put it inside this frame: id, link, title or part of a title; --at counts from its corner"
    )]
    pub frame: Option<String>,
    #[serde(flatten)]
    #[argument(flatten)]
    pub add: AddFields,
    #[argument(
        value_name = "file",
        help = "Many in one write: a JSON array or JSON lines file, - for stdin; its keys are the flags of one call",
        tool_doc = "Many at once, each with the arguments of one; at most 100 per call."
    )]
    pub items: Option<Items<AddItem>>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// The arguments of one object: what a call gives, or one of its `--items`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, ArgumentGroup)]
#[serde(default)]
pub struct AddFields {
    #[argument(
        value_name = "text",
        positional,
        help = "The text; lines become paragraphs; the title of a frame",
        tool_doc = "Lines are paragraphs.",
        kinds = [sticker, text, shape, frame, sticker_stack, flipcard, mind_map]
    )]
    pub text: Option<String>,
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
        value_name = "shape",
        help = "The kind of shape, e.g. ellipse, flowchart-decision or bpmn-gateway; square by default",
        tool_doc = "Shape kind.",
        kinds = [shape]
    )]
    pub shape: Option<ShapeType>,
    #[argument(
        value_name = "x,y",
        help = "The top-left corner in board units, from the frame's corner with --frame; a free corner by default",
        tool_doc = "x,y; add, cp: a free corner by default; edit: unchanged.",
        kinds = [sticker, text, shape, frame, link, image, file, dice, wheel, sticker_stack, flipcard, mind_map]
    )]
    pub at: Option<String>,
    #[argument(
        value_name = "w,h",
        help = "The size as width,height, e.g. 260,160; the kind's own size by default; an image's own size, at most 1000 on the long side, by default",
        tool_doc = "W,H.",
        kinds = [sticker, shape, frame, image, sticker_stack]
    )]
    pub size: Option<String>,
    #[argument(
        letter = 'c',
        value_name = "color",
        help = "The fill, or an arrow's line color: a palette token, e.g. yellow4; the kind's own color by default",
        tool_doc = "E.g. yellow4.",
        kinds = [sticker, text, shape, frame, arrow, dice, sticker_stack, flipcard, mind_map]
    )]
    pub color: Option<String>,
    #[argument(
        value_name = "n",
        help = "A die's faces: 2, 4, 6, 8, 10, 12 or 20; 6 by default",
        tool_doc = "Die faces 2, 4, 6, 8, 10, 12, 20.",
        kinds = [dice]
    )]
    pub faces: Option<u32>,
    #[argument(
        value_name = "text",
        help = "A wheel's choice, repeat for each: 2 to 20, each at most 40 characters",
        tool_doc = "Wheel choices, 2 to 20.",
        kinds = [wheel]
    )]
    pub choice: Vec<String>,
    #[argument(
        value_name = "text",
        help = "A flip card's back; the text is its front",
        tool_doc = "Flip card back.",
        kinds = [flipcard]
    )]
    pub back: Option<String>,
    #[argument(
        value_name = "object",
        help = "The mind-map node the new node hangs from: id, link, @N or the first 8 characters of its id; a new map without it",
        tool_doc = "Parent node; a new map without.",
        kinds = [mind_map]
    )]
    pub parent: Option<String>,
    #[argument(
        value_name = "side",
        help = "The side of its parent a new node goes: left, right, up or down; right by default",
        tool_doc = "Side of the parent.",
        kinds = [mind_map]
    )]
    pub direction: Option<Direction>,
    #[argument(
        help = "Keep the card plain even when the page can be embedded",
        tool_doc = "Plain card.",
        kinds = [link]
    )]
    pub no_preview: bool,
    #[argument(
        value_name = "object",
        help = "The object an arrow starts at: id, link, @N or the first 8 characters of its id; with --items also #N, an earlier item",
        tool_doc = "Or #N: an earlier item. On edit object:side or x,y.",
        kinds = [arrow]
    )]
    pub from: Option<String>,
    #[argument(
        value_name = "object",
        help = "The object an arrow points to: id, link, @N or the first 8 characters of its id; with --items also #N, an earlier item",
        tool_doc = "Arrow end; cp: the target board. On edit object:side or x,y.",
        kinds = [arrow]
    )]
    pub to: Option<String>,
    #[argument(
        value_name = "line",
        help = "An arrow's line: straight, elbow or curved; elbow by default",
        tool_doc = "Arrow line.",
        kinds = [arrow]
    )]
    pub line: Option<ArrowType>,
    #[argument(
        value_name = "start,end",
        help = "An arrow's heads, e.g. none,triangle-filled: none, figma, miro, arc, triangle-filled, triangle, circle-filled, circle, rhombus-filled, rhombus, stick, stick-stick, triadius, triadius-stick, triadius-circle, stick-circle or slash; an empty side keeps that end; none,miro by default",
        tool_doc = "start,end heads, an empty side keeps that end: none, figma, miro, arc, triangle[-filled], circle[-filled], rhombus[-filled], stick, stick-stick, triadius[-stick|-circle], stick-circle, slash.",
        kinds = [arrow]
    )]
    pub heads: Option<String>,
    #[argument(
        value_name = "style",
        help = "An arrow's line: solid, dashed or dotted; solid by default",
        tool_doc = "Outline style.",
        kinds = [arrow]
    )]
    pub stroke_style: Option<StrokeStyle>,
    #[argument(
        value_name = "n",
        help = "An arrow's line width: 1, 1.5, 2, 4, 5, 6, 9, 13 or 17; 2 by default",
        tool_doc = "Shape 0..10; arrow 1, 1.5, 2, 4, 5, 6, 9, 13, 17.",
        kinds = [arrow]
    )]
    pub stroke_width: Option<f64>,
    #[argument(
        value_name = "text[@pos]",
        help = "An arrow's label, repeat for several; text@0.25 puts it a quarter of the way from the start, in the middle by default",
        tool_doc = "Arrow labels text[@0..1]; an edit replaces the old.",
        kinds = [arrow]
    )]
    pub label: Vec<String>,
    #[argument(
        value_name = "ratio",
        help = "A frame's ratio: 16:9, 4:3, 1:1, a4 or a4-landscape; with --size the area is kept; 16:9 at 1920x1080 and the like by default",
        tool_doc = "Frame ratio 16:9, 4:3, 1:1, a4, a4_landscape.",
        kinds = [frame]
    )]
    pub ratio: Option<FrameRatio>,
    #[argument(
        value_name = "style",
        help = "A frame's background: fill, border or outline; border by default",
        tool_doc = "Frame background fill, border, outline.",
        kinds = [frame]
    )]
    pub background: Option<FrameBackground>,
    #[argument(
        help = "Hide the content of a frame; needs the board's right to hide frame content",
        tool_doc = "Hide frame content.",
        kinds = [frame]
    )]
    pub hide_content: bool,
}

/// One object of `--items` of `object add`: the arguments of one call.
#[derive(Debug, Clone, PartialEq)]
pub struct AddItem {
    pub kind: Option<AddKind>,
    pub frame: Option<String>,
    pub add: AddFields,
}

#[derive(Deserialize)]
struct Wire {
    kind: Option<AddKind>,
    frame: Option<String>,
    #[serde(flatten)]
    add: AddFields,
}

/// A key beside the arguments of one call is refused: `flatten` cannot deny it.
impl<'de> Deserialize<'de> for AddItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = Map::<String, Value>::deserialize(deserializer)?;
        let known = || {
            ["kind", "frame"]
                .into_iter()
                .chain(AddFields::ARGUMENTS.iter().map(|argument| argument.name))
        };
        map.keys()
            .find(|key| known().all(|name| name != key.as_str()))
            .map_or(Ok(()), |key| {
                let expected =
                    known().format_with(", ", |name, f| f(&format_args!("`{name}`")));
                Err(D::Error::custom(format_args!(
                    "unknown field `{key}`, expected one of {expected}"
                )))
            })?;
        let Wire { kind, frame, add } =
            Wire::deserialize(Value::Object(map)).map_err(D::Error::custom)?;
        Ok(Self { kind, frame, add })
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;

    use super::{AddFields, AddItem};
    use crate::{
        Result,
        command::{
            fixture::arguments,
            object::{AddArgs, AddKind},
            table::{ArgumentGroup as _, CommandArgs},
        },
    };

    #[test]
    fn an_item_with_a_stray_key_is_refused_naming_the_keys() {
        let refused = serde_json::from_value::<AddItem>(json!({"colour": "red"}))
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref().map(|text| text.starts_with(
                "unknown field `colour`, expected one of `kind`, `frame`, `text`,"
            )),
            Some(true)
        );
    }

    #[test]
    fn an_item_holds_its_kind_frame_and_the_fields_of_one_object() {
        let item: serde_json::Result<AddItem> = serde_json::from_value(json!({
            "kind": "frame", "frame": "#1", "text": "x", "hide_content": true,
            "label": ["a"], "faces": 6
        }));
        assert_eq!(
            item.ok(),
            Some(AddItem {
                kind: Some(AddKind::Frame),
                frame: Some("#1".to_owned()),
                add: AddFields {
                    text: Some("x".to_owned()),
                    hide_content: true,
                    label: vec!["a".to_owned()],
                    faces: Some(6),
                    ..AddFields::default()
                }
            })
        );
    }

    #[test]
    fn the_first_given_field_is_named_by_its_argument() {
        let fields = AddFields {
            stroke_width: Some(4.0),
            no_preview: true,
            ..AddFields::default()
        };
        assert_eq!(
            (fields.first_given(), AddFields::default().first_given()),
            (Some("no_preview"), None)
        );
    }

    #[test]
    fn the_kinds_table_names_the_kinds_of_each_argument() {
        let limited: Vec<_> = AddArgs::COMMAND
            .arguments
            .iter()
            .filter(|argument| !argument.kinds.is_empty())
            .map(|argument| (argument.name, argument.kinds))
            .collect();
        assert_eq!(
            limited,
            vec![
                ("text_format", &["sticker", "text", "shape", "mind-map"][..]),
                (
                    "text",
                    &[
                        "sticker",
                        "text",
                        "shape",
                        "frame",
                        "sticker-stack",
                        "flipcard",
                        "mind-map"
                    ][..]
                ),
                ("url", &["link"][..]),
                ("path", &["image", "file"][..]),
                ("shape", &["shape"][..]),
                (
                    "at",
                    &[
                        "sticker",
                        "text",
                        "shape",
                        "frame",
                        "link",
                        "image",
                        "file",
                        "dice",
                        "wheel",
                        "sticker-stack",
                        "flipcard",
                        "mind-map"
                    ][..]
                ),
                (
                    "size",
                    &["sticker", "shape", "frame", "image", "sticker-stack"][..]
                ),
                (
                    "color",
                    &[
                        "sticker",
                        "text",
                        "shape",
                        "frame",
                        "arrow",
                        "dice",
                        "sticker-stack",
                        "flipcard",
                        "mind-map"
                    ][..]
                ),
                ("faces", &["dice"][..]),
                ("choice", &["wheel"][..]),
                ("back", &["flipcard"][..]),
                ("parent", &["mind-map"][..]),
                ("direction", &["mind-map"][..]),
                ("no_preview", &["link"][..]),
                ("from", &["arrow"][..]),
                ("to", &["arrow"][..]),
                ("line", &["arrow"][..]),
                ("heads", &["arrow"][..]),
                ("stroke_style", &["arrow"][..]),
                ("stroke_width", &["arrow"][..]),
                ("label", &["arrow"][..]),
                ("ratio", &["frame"][..]),
                ("background", &["frame"][..]),
                ("hide_content", &["frame"][..]),
            ]
        );
    }

    #[test]
    fn the_keys_of_an_item_are_flags_of_one_call() {
        const ADD_KEYS: [&str; 25] = [
            "kind",
            "text",
            "shape",
            "at",
            "size",
            "color",
            "frame",
            "from",
            "to",
            "line",
            "heads",
            "stroke_style",
            "stroke_width",
            "label",
            "url",
            "path",
            "no_preview",
            "ratio",
            "background",
            "hide_content",
            "faces",
            "choice",
            "back",
            "parent",
            "direction",
        ];
        let strays: Vec<_> = ADD_KEYS
            .iter()
            .filter(|key| {
                AddArgs::COMMAND
                    .arguments
                    .iter()
                    .all(|argument| argument.name != **key)
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
