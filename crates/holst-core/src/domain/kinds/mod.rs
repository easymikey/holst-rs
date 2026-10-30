//! What a listing shows of each kind beyond the common fields.

pub mod code;
pub mod dice;
pub mod document;
pub mod flipcard;
pub mod icon;
pub mod kanban;
pub mod mind_map;
pub mod reaction_stamp;
pub mod stamp;
pub mod sticker_stack;
pub mod table;
pub mod task;
pub mod wheel;

use yrs::Doc;

use crate::domain::board::{Object, ObjectKind};

/// What a listing shows of an object beside the fields every object has.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detail {
    /// The text shown in place of `Object::text` (a code block, a stamp).
    pub text: Option<String>,
    /// Words of the kind, in order: `rows=2`, `status=to-do`.
    pub words: Vec<(&'static str, serde_json::Value)>,
}

/// The detail of the object's kind; no detail for a kind that adds none.
#[must_use]
pub fn detail(document: &Doc, object: &Object) -> Detail {
    match object.kind {
        ObjectKind::Table => table::detail(object),
        ObjectKind::Dice => dice::detail(object),
        ObjectKind::Wheel => wheel::detail(object),
        ObjectKind::StickerStack => sticker_stack::detail(object),
        ObjectKind::Flipcard => flipcard::detail(document, object),
        ObjectKind::Stamp => stamp::detail(object),
        ObjectKind::Other(_) if mind_map::is_node(object) => mind_map::detail(object),
        ObjectKind::Other(_) if reaction_stamp::is_stamp(object) => {
            reaction_stamp::detail(object)
        }
        ObjectKind::Other(_) if icon::is_icon(object) => icon::detail(object),
        ObjectKind::Other(_) if kanban::is_kanban(object) => {
            kanban::detail(document, object)
        }
        ObjectKind::Document => document::detail(object),
        ObjectKind::Code => code::detail(document, object),
        ObjectKind::Task => task::detail(document, object),
        ObjectKind::Sticker
        | ObjectKind::Text
        | ObjectKind::Shape
        | ObjectKind::Frame
        | ObjectKind::Arrow
        | ObjectKind::Image
        | ObjectKind::File
        | ObjectKind::Group
        | ObjectKind::TableCell
        | ObjectKind::Drawing
        | ObjectKind::Link
        | ObjectKind::Other(_) => Detail::default(),
    }
}
