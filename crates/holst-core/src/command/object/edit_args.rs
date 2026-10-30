//! The arguments of `object edit`.

use holst_macros::{ArgumentGroup, CommandArgs};
use itertools::Itertools as _;
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::{Map, Value};

use super::MarkChoice;
use crate::{
    command::table::{ArgumentGroup as _, Confirmation, Items},
    domain::{
        board::{
            arrow_look::ArrowType,
            frame_look::{FrameBackground, FrameRatio},
            shape_type::{ShapeType, StrokeStyle},
        },
        change::{
            media::{CropMask, FileDisplay, FlipState},
            value::LayerOrder,
        },
        kinds::code::{Language, Theme},
        rich_text::TextFormat,
    },
    effect::board_write::WriteMode,
};

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Object,
    verb = "edit",
    risk = Destroy,
    block = Commands,
    about = "Change text, corner, size, color, order or tags of objects",
    help = "Change text, corner, size, color, order or tags of objects. The objects are the linked object and every --object.\n\
            Moving a frame moves its content. It prints one line per changed object with @N and its link.\n\
            Deleting a table row or column whose cells hold text needs -y, since the text goes with it.",
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
    #[argument(
        value_name = "file",
        help = "The picture that replaces an image's file: it covers the image's box keeping its proportions and the mask; one image per call, not with --crop, --name or --items; unchanged by default",
        tool_doc = "Local file; edit: the new picture."
    )]
    pub path: Option<String>,
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
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; a terminal asks instead; without one only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// The arguments of one edit: what a call gives, or one of its `--items`.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a switch per flag: show and hide content, show and hide author, roll, flip"
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
        value_name = "name",
        help = "The name an image or a file shows on the board; the extension of the old name is kept when left out; unchanged by default",
        tool_doc = "Image or file name."
    )]
    pub name: Option<String>,
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
        help = "The new fill, an arrow's line color or a drawing's stroke color: a palette token, e.g. yellow4 (a task: gray, green, red...); unchanged by default",
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
        help = "A shape's or an image's outline color: a palette token, e.g. gray12; unchanged by default",
        tool_doc = "Outline color."
    )]
    pub stroke_color: Option<String>,
    #[argument(
        value_name = "n",
        help = "A shape's or an image's outline width, 0 to 10, a drawing's stroke width, 1 to 30, or an arrow's line width: 1, 1.5, 2, 4, 5, 6, 9, 13 or 17; unchanged by default",
        tool_doc = "Shape 0..10; arrow 1, 1.5, 2, 4, 5, 6, 9, 13, 17."
    )]
    pub stroke_width: Option<f64>,
    #[argument(
        value_name = "style",
        help = "A shape's or an image's outline, or an arrow's line: solid, dashed or dotted; unchanged by default",
        tool_doc = "Outline style."
    )]
    pub stroke_style: Option<StrokeStyle>,
    #[argument(
        value_name = "n",
        help = "A shape's or an image's corner radius, at most as its size allows; unchanged by default",
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
        help = "Roll the dice and spin the wheels named, as the web client does; at most 50 objects",
        tool_doc = "Roll dice, spin wheels."
    )]
    pub roll: bool,
    #[argument(
        value_name = "n",
        help = "A die's faces: 2, 4, 6, 8, 10, 12 or 20; its value stays among them; unchanged by default",
        tool_doc = "Die faces 2, 4, 6, 8, 10, 12, 20."
    )]
    pub faces: Option<u32>,
    #[argument(
        help = "Hide the children of a mind-map node",
        tool_doc = "Hide node children."
    )]
    pub hide_children: bool,
    #[argument(
        help = "Show the children of a mind-map node",
        tool_doc = "Show node children."
    )]
    pub show_children: bool,
    #[argument(
        help = "Turn a flip card over: the other side up, its width and height swapped about its centre",
        tool_doc = "Turn flip card over."
    )]
    pub flip: bool,
    #[argument(
        value_name = "text",
        help = "A flip card's new back, written in the --text-format; unchanged by default",
        tool_doc = "Flip card back."
    )]
    pub back: Option<String>,
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
    #[argument(
        value_name = "name",
        help = "A code block's language for its highlighting; unchanged by default",
        tool_doc = "Code language."
    )]
    pub language: Option<Language>,
    #[argument(
        value_name = "theme",
        help = "A code block's theme: light or dark; unchanged by default",
        tool_doc = "Code theme."
    )]
    pub theme: Option<Theme>,
    #[argument(
        value_name = "row,column",
        help = "A table's cell, counted from 1 in reading order, e.g. 2,3, or a block r1,c1,r2,c2: --text is written into the cell, -c colours the cells, a cell made for either when it has none; unchanged by default",
        tool_doc = "Table cell row,column or r1,c1,r2,c2 from 1; takes text or -c."
    )]
    pub cell: Option<String>,
    #[argument(
        value_name = "n",
        help = "Insert a row into a table before its n-th, counting from 1; one past the last adds at the end; one structural flag per call; unchanged by default",
        tool_doc = "Table: row before n-th."
    )]
    pub insert_row: Option<u32>,
    #[argument(
        value_name = "n",
        help = "Insert a column into a table before its n-th, counting from 1; one past the last adds at the end; unchanged by default",
        tool_doc = "Table: column before n-th."
    )]
    pub insert_column: Option<u32>,
    #[argument(
        value_name = "n",
        help = "Delete a table's n-th row with its cells, counting from 1; refused across a merged block; text in it needs -y; unchanged by default",
        tool_doc = "Table: delete n-th row."
    )]
    pub delete_row: Option<u32>,
    #[argument(
        value_name = "n",
        help = "Delete a table's n-th column with its cells, counting from 1; refused across a merged block; text in it needs -y; unchanged by default",
        tool_doc = "Table: delete n-th column."
    )]
    pub delete_column: Option<u32>,
    #[argument(
        value_name = "mask",
        help = "An image's mask: circle, square, portrait, landscape or wide cuts the largest such box out of its box, none gives back the whole picture; it sets the corner and size, so not with --at, --size or --rotate; unchanged by default",
        tool_doc = "circle, square, portrait, landscape, wide, none."
    )]
    pub crop: Option<CropMask>,
    #[argument(
        value_name = "axes",
        help = "An image's mirroring, set and not toggled: none, horizontal, vertical or both; unchanged by default",
        tool_doc = "none, horizontal, vertical, both."
    )]
    pub flip_image: Option<FlipState>,
    #[argument(
        value_name = "r1,c1,r2,c2",
        help = "Merge a block of a table's cells, counted from 1, e.g. 1,1,2,2: the first cell with text, in reading order, leads and its text is kept; unchanged by default",
        tool_doc = "Table: merge r1,c1,r2,c2."
    )]
    pub merge: Option<String>,
    #[argument(
        value_name = "row,column",
        help = "Split the merged block of a table that the cell leads, counted from 1, e.g. 1,1; unchanged by default",
        tool_doc = "Table: unmerge block led by r,c."
    )]
    pub unmerge: Option<String>,
    #[argument(
        value_name = "r1,c1,r2,c2",
        help = "Empty the text of a block of a table's cells, counted from 1, and remove what the cells hold; text in them needs -y; unchanged by default",
        tool_doc = "Table: clear r1,c1,r2,c2."
    )]
    pub clear: Option<String>,
    #[argument(
        value_name = "text",
        help = "A task's description, written in the --text-format; none empties it; unchanged by default",
        tool_doc = "Task description."
    )]
    pub description: Option<String>,
    #[argument(
        value_name = "status",
        help = "A task's status: backlog, to-do, in-progress, done, canceled, or the title of a status of the board; none takes it away; unchanged by default",
        tool_doc = "Task status."
    )]
    pub status: Option<String>,
    #[argument(
        value_name = "day",
        help = "The day a task starts, YYYY-MM-DD, e.g. 2026-11-02, on or before its due day; none takes it away; unchanged by default",
        tool_doc = "Task start YYYY-MM-DD."
    )]
    pub start: Option<String>,
    #[argument(
        value_name = "day",
        help = "The day a task is due, YYYY-MM-DD, e.g. 2026-11-06; none takes it away; unchanged by default",
        tool_doc = "Task due YYYY-MM-DD."
    )]
    pub due: Option<String>,
    #[argument(
        value_name = "view",
        help = "A file's view: stamp is the icon card, content shows the pages, the player or the video, which needs the file's page count, duration or video info; unchanged by default",
        tool_doc = "File: stamp or content."
    )]
    pub display: Option<FileDisplay>,
    #[argument(
        value_name = "n",
        help = "The page a file of pages opens on, from 1 to its page count; unchanged by default",
        tool_doc = "File start page from 1."
    )]
    pub start_page: Option<u32>,
    #[argument(
        help = "Hide the details of a video file shown as content",
        tool_doc = "Hide video details."
    )]
    pub hide_details: bool,
    #[argument(
        help = "Show the details of a video file shown as content",
        tool_doc = "Show video details."
    )]
    pub show_details: bool,
    #[argument(
        value_name = "pages",
        help = "Put pages of a file of pages on the board as file objects of one page each, in a row below it: all, or pages and ranges such as 1-3,5, at most 50; takes one file and no other field; it prints one line per new object with @N and its link",
        tool_doc = "File: all or pages, e.g. 1-3,5."
    )]
    pub extract: Option<String>,
    #[argument(
        value_name = "user",
        help = "The people a task is assigned to, repeat for several: members of the board's workspace by email, name or id, any part, any case; they replace the assignees, none clears; needs a member of the workspace, not a guest; unchanged by default",
        tool_doc = "Assignees; none clears."
    )]
    pub assignee: Vec<String>,
    #[argument(
        value_name = "object|none",
        help = "Move a task card into this kanban, as a card in its cell: id, link, @N or the first 8 characters of its id; --column and --swimlane pick the cell, alone they move it within its own kanban; none takes it out onto the board where it is drawn; unchanged by default",
        tool_doc = "Kanban of a task."
    )]
    pub kanban: Option<String>,
    #[argument(
        value_name = "text",
        help = "The title of the column a task card moves to, in --kanban or the kanban it is in; the first by default for another kanban",
        tool_doc = "Kanban column titles."
    )]
    pub column: Vec<String>,
    #[argument(
        value_name = "text",
        help = "The title of the swimlane a task card moves to, in --kanban or the kanban it is in; the first by default for another kanban",
        tool_doc = "Kanban swimlane titles."
    )]
    pub swimlane: Vec<String>,
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
        const EDIT_KEYS: [&str; 69] = [
            "object",
            "text",
            "name",
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
            "roll",
            "faces",
            "hide_children",
            "show_children",
            "flip",
            "back",
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
            "language",
            "theme",
            "cell",
            "insert_row",
            "insert_column",
            "delete_row",
            "delete_column",
            "crop",
            "flip_image",
            "merge",
            "unmerge",
            "clear",
            "description",
            "status",
            "start",
            "due",
            "display",
            "start_page",
            "hide_details",
            "show_details",
            "extract",
            "assignee",
            "kanban",
            "column",
            "swimlane",
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
                "unknown field `colour`, expected one of `object`, `text`, `name`, `at`,"
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
