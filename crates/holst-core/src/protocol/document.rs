//! Names of the maps and fields in a board's Yjs document.

// PROTOCOL: the maps of the document.
pub(crate) const OBJECTS: &str = "objects";
pub(crate) const DOCUMENTS: &str = "documents";
pub(crate) const USERS_IDS: &str = "usersIds";

// PROTOCOL: the room `<board>$comments`.
pub(crate) const COMMENTS_STORE: &str = "commentsStore";
pub(crate) const COMMENTS_ROOM: &str = "$comments";

// PROTOCOL: fields every object has.
pub(crate) const ID: &str = "id";
pub(crate) const TYPE: &str = "type";
pub(crate) const POSITION: &str = "position";
pub(crate) const X: &str = "x";
pub(crate) const Y: &str = "y";
pub(crate) const WIDTH: &str = "width";
pub(crate) const HEIGHT: &str = "height";
pub(crate) const Z_INDEX: &str = "zIndex";
pub(crate) const CREATED: &str = "created";
// PROTOCOL: `created.a` is an index into `usersIds`.
pub(crate) const AUTHOR_INDEX: &str = "a";
pub(crate) const PARENT_ID: &str = "parentId";

// PROTOCOL: a group keeps its members' places and is their `parentId`.
pub(crate) const IGNORE_Z_INDEX: &str = "ignoreZIndex";
// PROTOCOL: the table cell an object in a table sits in, copied onto its group.
pub(crate) const ADDITIONAL_POSITION_INFO: &str = "additionalPositionInfo";

// PROTOCOL: text and notes of an object.
pub(crate) const DOCUMENT_ID: &str = "documentId";
pub(crate) const TEXTS: &str = "texts";
pub(crate) const LABEL_TEXT: &str = "labelText";
pub(crate) const TITLE: &str = "title";
pub(crate) const EDITOR: &str = "editor";
pub(crate) const TEXT: &str = "text";
pub(crate) const CHILDREN: &str = "children";

// PROTOCOL: the look of an object.
pub(crate) const COLOR: &str = "color";
pub(crate) const COLOR_TOKEN: &str = "colorToken";
pub(crate) const FILL_COLOR: &str = "fillColor";
pub(crate) const TEXT_COLOR: &str = "textColor";
pub(crate) const FONT_SIZE: &str = "fontSize";
pub(crate) const TEXT_SCALE: &str = "textScale";
pub(crate) const SHOW_AUTHOR: &str = "showAuthor";
pub(crate) const FIXED_WIDTH: &str = "fixedWidth";
pub(crate) const ROTATION: &str = "rotation";
pub(crate) const HORIZONTAL_ALIGN: &str = "horizontalAlign";
pub(crate) const VERTICAL_ALIGN: &str = "verticalAlign";
pub(crate) const BACKGROUND_STYLE: &str = "backgroundStyle";
pub(crate) const OPACITY: &str = "opacity";
pub(crate) const BORDER_RADIUS: &str = "borderRadius";
pub(crate) const CROP_TRANSFORM: &str = "cropTransform";

// PROTOCOL: shapes and their outline.
pub(crate) const SHAPE_TYPE: &str = "shapeType";
pub(crate) const STROKE_COLOR: &str = "strokeColor";
pub(crate) const STROKE_STYLE: &str = "strokeStyle";
pub(crate) const STROKE_WIDTH: &str = "strokeWidth";
pub(crate) const FIXED_SIZE: &str = "fixedSize";
pub(crate) const BPMN_RESOURCES: &str = "bpmnResources";

// PROTOCOL: arrows, their ends and their heads.
pub(crate) const START: &str = "start";
pub(crate) const END: &str = "end";
pub(crate) const POINT: &str = "point";
pub(crate) const RELATIVE_POINT: &str = "relativePoint";
pub(crate) const CONTROL: &str = "control";
pub(crate) const OBJECT_ID: &str = "objectId";
pub(crate) const ARROW_TYPE: &str = "arrowType";
pub(crate) const ARROWHEAD_START: &str = "arrowheadStart";
pub(crate) const ARROWHEAD_END: &str = "arrowheadEnd";

// PROTOCOL: files and images.
pub(crate) const NAME: &str = "name";
pub(crate) const ORIGINAL_NAME: &str = "originalName";
pub(crate) const FILE_NAME: &str = "fileName";
pub(crate) const DISPLAY_FILE_NAME: &str = "displayFileName";

// PROTOCOL: drawings and stamps.
pub(crate) const DATA: &str = "data";
pub(crate) const URL: &str = "url";
pub(crate) const PATH: &str = "path";
pub(crate) const SCALE: &str = "scale";
pub(crate) const ALPHA: &str = "alpha";
pub(crate) const ALGORITHM: &str = "algorithm";

// PROTOCOL: table tracks `column-<id>` and `row-<id>`.
pub(crate) const COLUMN_PREFIX: &str = "column-";
pub(crate) const ROW_PREFIX: &str = "row-";
pub(crate) const COLUMN_ID: &str = "columnId";
pub(crate) const ROW_ID: &str = "rowId";
pub(crate) const INDEX: &str = "index";

// PROTOCOL: link cards and embeds.
pub(crate) const LINK: &str = "link";
pub(crate) const LINK_INFO: &str = "linkInfo";

// PROTOCOL: tags, `tagIds` and the board's definitions.
pub(crate) const TAG_IDS: &str = "tagIds";
pub(crate) const TAGS_STORE: &str = "task-card-tagsStore";
