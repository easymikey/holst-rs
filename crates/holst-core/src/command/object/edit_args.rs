//! The arguments of `object edit`.

use holst_macros::{ArgumentGroup, CommandArgs};
use itertools::Itertools as _;
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::{Map, Value};

use super::MarkChoice;
use crate::{
    command::table::{ArgumentGroup as _, Items},
    domain::{
        board::{
            arrow_look::ArrowType,
            frame_look::{FrameBackground, FrameRatio},
            shape_type::{ShapeType, StrokeStyle},
        },
        change::value::LayerOrder,
        rich_text::TextFormat,
    },
    effect::board_write::WriteMode,
};

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
        value_name = "format",
        help = "How the new text is read: plain, or markdown for headings, lists, checkboxes, links and bold, italic, strike marks; plain by default",
        tool_doc = "plain or markdown."
    )]
    pub text_format: Option<TextFormat>,
    #[serde(flatten)]
    #[argument(flatten)]
    pub edit: EditFields,
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

/// The arguments of one edit: what a call gives, or one of its `--items`.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a switch per flag: show and hide content, show and hide author"
)]
#[derive(Debug, Clone, PartialEq, Default, Deserialize, ArgumentGroup)]
#[serde(default)]
pub struct EditFields {
    #[argument(
        value_name = "text",
        help = "The new text; lines become paragraphs; unchanged by default",
        tool_doc = "Lines are paragraphs."
    )]
    pub text: Option<String>,
    #[argument(
        value_name = "x,y",
        help = "The new top-left corner in board units; unchanged by default",
        tool_doc = "x,y; add, cp: a free corner by default; edit: unchanged."
    )]
    pub at: Option<String>,
    #[argument(
        value_name = "w,h",
        help = "The new size as width,height, e.g. 260,160; unchanged by default",
        tool_doc = "W,H."
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
        value_name = "shape",
        help = "Change a shape's kind, e.g. ellipse, flowchart-decision or bpmn-gateway; unchanged by default",
        tool_doc = "Shape kind."
    )]
    pub shape: Option<ShapeType>,
    #[argument(
        value_name = "color",
        help = "A shape's outline color: a palette token, e.g. gray12; unchanged by default",
        tool_doc = "Outline color."
    )]
    pub stroke_color: Option<String>,
    #[argument(
        value_name = "n",
        help = "A shape's outline width, 0 to 10, or an arrow's line width: 1, 1.5, 2, 4, 5, 6, 9, 13 or 17; unchanged by default",
        tool_doc = "Shape 0..10; arrow 1, 1.5, 2, 4, 5, 6, 9, 13, 17."
    )]
    pub stroke_width: Option<f64>,
    #[argument(
        value_name = "style",
        help = "A shape's outline or an arrow's line: solid, dashed or dotted; unchanged by default",
        tool_doc = "Outline style."
    )]
    pub stroke_style: Option<StrokeStyle>,
    #[argument(
        value_name = "n",
        help = "A shape's corner radius, at most as the shape's size allows; unchanged by default",
        tool_doc = "Corner radius."
    )]
    pub radius: Option<u32>,
    #[argument(
        value_name = "n",
        help = "A shape's fill opacity, 0.1 to 1; unchanged by default",
        tool_doc = "0.1..1."
    )]
    pub opacity: Option<f64>,
    #[argument(
        value_name = "marks",
        help = "The marks of the whole text of a sticker, text or shape, and no others: any of bold, italic, underline, strikethrough, code, comma apart, or none; unchanged by default",
        tool_doc = "Whole text: bold,italic,underline,strikethrough,code or none."
    )]
    pub mark: Vec<MarkChoice>,
    #[argument(
        value_name = "color",
        help = "The color of the whole text of a sticker, text or shape: a palette token, e.g. red6; unchanged by default",
        tool_doc = "Whole text, e.g. red6."
    )]
    pub text_color: Option<String>,
    #[argument(
        value_name = "color",
        help = "The background of the whole text of a sticker, text or shape: a palette token, e.g. yellow4, or none; unchanged by default",
        tool_doc = "Whole text, e.g. yellow4 or none."
    )]
    pub text_background: Option<String>,
    #[argument(
        value_name = "auto|n",
        help = "A sticker's text size, 4 to 64, or auto to fit the note; unchanged by default",
        tool_doc = "Sticker: auto or 4..64."
    )]
    pub text_size: Option<String>,
    #[argument(
        help = "Show the author on a sticker",
        tool_doc = "Show sticker author."
    )]
    pub show_author: bool,
    #[argument(
        help = "Hide the author on a sticker",
        tool_doc = "Hide sticker author."
    )]
    pub hide_author: bool,
    #[argument(
        value_name = "reaction",
        help = "The emoji reactions you leave on a sticker, repeat for several, up to 8; they replace yours, none takes them away; unchanged by default",
        tool_doc = "Your reactions; none clears."
    )]
    pub reaction: Vec<String>,
    #[argument(
        value_name = "line",
        help = "An arrow's line: straight, elbow or curved; unchanged by default",
        tool_doc = "Arrow line."
    )]
    pub line: Option<ArrowType>,
    #[argument(
        value_name = "start,end",
        help = "An arrow's heads, e.g. none,triangle-filled: none, figma, miro, arc, triangle-filled, triangle, circle-filled, circle, rhombus-filled, rhombus, stick, stick-stick, triadius, triadius-stick, triadius-circle, stick-circle or slash; an empty side keeps that end; unchanged by default",
        tool_doc = "start,end heads, an empty side keeps that end: none, figma, miro, arc, triangle[-filled], circle[-filled], rhombus[-filled], stick, stick-stick, triadius[-stick|-circle], stick-circle, slash."
    )]
    pub heads: Option<String>,
    #[argument(
        value_name = "x,y",
        help = "Bend an arrow: x,y for a straight or curved one, x, or ,y for an elbow, whose middle run sits there; unchanged by default",
        tool_doc = "Arrow bend: x,y; an elbow x, or ,y."
    )]
    pub bend: Option<String>,
    #[argument(
        value_name = "end",
        help = "Move an arrow's start: object:side (top, right, bottom or left) or x,y; unchanged by default",
        tool_doc = "Or #N: an earlier item. On edit object:side or x,y."
    )]
    pub from: Option<String>,
    #[argument(
        value_name = "end",
        help = "Move an arrow's end: object:side (top, right, bottom or left) or x,y; unchanged by default",
        tool_doc = "Arrow end; cp: the target board. On edit object:side or x,y."
    )]
    pub to: Option<String>,
    #[argument(
        value_name = "text[@pos]",
        help = "An arrow's labels, repeat for several; they replace the ones it has; text@0.25 puts one a quarter of the way from the start, in the middle by default; unchanged by default",
        tool_doc = "Arrow labels text[@0..1]; an edit replaces the old."
    )]
    pub label: Vec<String>,
    #[argument(
        value_name = "ratio",
        help = "A frame's ratio: 16:9, 4:3, 1:1, a4 or a4-landscape; the area is kept; unchanged by default",
        tool_doc = "Frame ratio 16:9, 4:3, 1:1, a4, a4_landscape."
    )]
    pub ratio: Option<FrameRatio>,
    #[argument(
        value_name = "style",
        help = "A frame's background: fill, border or outline; unchanged by default",
        tool_doc = "Frame background fill, border, outline."
    )]
    pub background: Option<FrameBackground>,
    #[argument(
        help = "Hide the content of a frame; needs the board's right to hide frame content",
        tool_doc = "Hide frame content."
    )]
    pub hide_content: bool,
    #[argument(
        help = "Show the content of a frame again",
        tool_doc = "Show frame content."
    )]
    pub show_content: bool,
    #[argument(
        value_name = "frame|none",
        help = "Move it into this frame: id, link, title or part of a title; none takes it out onto the board. It keeps its place relative to the frame, or goes to a free spot in it when that would put it outside, and goes to the front there; unchanged by default"
    )]
    pub frame: Option<String>,
    #[argument(
        value_name = "deg",
        help = "Turn to this angle in clockwise degrees, any number taken modulo 360; the corner moves so that the object turns about its centre; not for a frame, a table, a table cell, a group or an arrow; unchanged by default",
        tool_doc = "Degrees clockwise; not frame, table, group, arrow."
    )]
    pub rotate: Option<f64>,
    #[argument(
        value_name = "factor",
        help = "Scale a link, drawing, code, file, kanban, flipcard, dice, wheel or task to this factor above 0, or a sticker, text or sticker stack text to it; the corner stays; not with --size; unchanged by default",
        tool_doc = "Factor; not with size."
    )]
    pub scale: Option<f64>,
    #[argument(
        value_name = "link",
        help = "What the object opens on click: a web address (http:// is added without a scheme), another object of the board by id, or none to take the link away; unchanged by default",
        tool_doc = "URL, object id, or none."
    )]
    pub link_to: Option<String>,
    #[argument(
        value_name = "auto|n",
        help = "A text's width: auto wraps it by its content, a number fixes it in board units; unchanged by default",
        tool_doc = "Text: auto or a width."
    )]
    pub width: Option<String>,
    #[argument(
        value_name = "n",
        help = "Tick the n-th checklist item of a sticker, text or shape, counting from 1 in reading order; unchanged by default",
        tool_doc = "Tick checklist item n, from 1."
    )]
    pub check: Option<u32>,
    #[argument(
        value_name = "n",
        help = "Untick the n-th checklist item of a sticker, text or shape, counting from 1 in reading order; unchanged by default",
        tool_doc = "Untick checklist item n."
    )]
    pub uncheck: Option<u32>,
    #[argument(
        value_name = "order",
        help = "Where it stands among the objects: front or back, or forward or backward for one step past the next overlapping object; unchanged by default"
    )]
    pub order: Option<LayerOrder>,
    #[argument(
        value_name = "tag",
        help = "The object's tags, repeat for several; a new tag is made, '' or none clears; unchanged by default"
    )]
    pub tag: Vec<String>,
}

/// One object of `--items` of `object edit`: the arguments of one call.
#[derive(Debug, Clone, PartialEq)]
pub struct EditItem {
    pub object: Option<String>,
    pub edit: EditFields,
}

#[derive(Deserialize)]
struct Wire {
    object: Option<String>,
    #[serde(flatten)]
    edit: EditFields,
}

/// A key beside the arguments of one call is refused: `flatten` cannot deny it.
impl<'de> Deserialize<'de> for EditItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = Map::<String, Value>::deserialize(deserializer)?;
        let known = || {
            std::iter::once("object")
                .chain(EditFields::ARGUMENTS.iter().map(|argument| argument.name))
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
        let Wire { object, edit } =
            Wire::deserialize(Value::Object(map)).map_err(D::Error::custom)?;
        Ok(Self { object, edit })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{EditFields, EditItem};
    use crate::command::{
        object::EditArgs,
        table::{ArgumentGroup as _, CommandArgs},
    };

    #[test]
    fn the_keys_of_an_item_are_flags_of_one_call() {
        const EDIT_KEYS: [&str; 37] = [
            "object",
            "text",
            "at",
            "size",
            "color",
            "shape",
            "stroke_color",
            "stroke_width",
            "stroke_style",
            "radius",
            "opacity",
            "mark",
            "text_color",
            "text_background",
            "text_size",
            "show_author",
            "hide_author",
            "reaction",
            "line",
            "heads",
            "bend",
            "from",
            "to",
            "label",
            "ratio",
            "background",
            "hide_content",
            "show_content",
            "frame",
            "rotate",
            "scale",
            "link_to",
            "width",
            "check",
            "uncheck",
            "order",
            "tag",
        ];
        let strays: Vec<_> = EDIT_KEYS
            .iter()
            .filter(|key| {
                EditArgs::COMMAND
                    .arguments
                    .iter()
                    .all(|argument| argument.name != **key)
            })
            .collect();
        assert!(strays.is_empty(), "{strays:?}");
    }

    #[test]
    fn an_item_with_a_stray_key_is_refused_naming_the_keys() {
        let refused = serde_json::from_value::<EditItem>(json!({"colour": "red"}))
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref().map(|text| text.starts_with(
                "unknown field `colour`, expected one of `object`, `text`, `at`,"
            )),
            Some(true)
        );
    }

    #[test]
    fn an_item_holds_its_object_and_the_fields_of_one_edit() {
        let item: Result<EditItem, _> = serde_json::from_value(
            json!({"object": "a1", "text": "x", "hide_content": true, "tag": ["t"]}),
        );
        assert_eq!(
            item.ok(),
            Some(EditItem {
                object: Some("a1".to_owned()),
                edit: EditFields {
                    text: Some("x".to_owned()),
                    hide_content: true,
                    tag: vec!["t".to_owned()],
                    ..EditFields::default()
                }
            })
        );
    }

    #[test]
    fn the_first_given_field_is_named_by_its_argument() {
        let fields = EditFields {
            stroke_width: Some(4.0),
            hide_content: true,
            ..EditFields::default()
        };
        assert_eq!(
            (fields.first_given(), EditFields::default().first_given()),
            (Some("stroke_width"), None)
        );
    }
}
