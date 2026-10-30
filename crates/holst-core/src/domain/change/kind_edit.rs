//! The edits only some kinds take, one variant and one file per kind.

mod chance;
mod code;
mod flipcard;
mod kanban;
mod mind_map;
mod reaction;
mod sticker;
mod table;
mod task;

use std::collections::HashMap;

use serde_json::Value;
use yrs::TransactionMut;

pub(super) use self::table::next_index;
pub use self::{
    kanban::CardMove,
    reaction::MOST_KINDS,
    sticker::{StickerEdit, TextSize},
    table::{Span, TableEdit, Track},
    task::{Setting, TaskEdit},
};
use crate::{
    domain::{
        board::ObjectKind,
        change::{value::ChangedObject, write::WriteTransaction},
        kinds::code::{Language, Theme},
        rich_text::TextFormat,
    },
    error::{Error, Result, board_change::BoardChangeError},
};

/// The refusal of a field the kind does not have.
pub(super) fn not_changeable(kind: &ObjectKind, field: &'static str) -> Error {
    Error::BoardChange(BoardChangeError::NotChangeable {
        kind: kind.as_str().to_owned(),
        field,
    })
}

/// Refuses the field unless `kind` is `only`, the one kind that takes it.
pub(super) fn only(
    kind: &ObjectKind,
    only: &ObjectKind,
    field: &'static str,
) -> Result<()> {
    (kind == only)
        .then_some(())
        .ok_or_else(|| not_changeable(kind, field))
}

/// What the edit writes: fields of the object and the other objects it touched.
pub(super) struct Written {
    pub(super) fields: Vec<(&'static str, Value)>,
    pub(super) touched: Vec<ChangedObject>,
}

/// What an edit changes that only some kinds have; each variant is one kind's.
#[derive(Debug, Clone, PartialEq)]
pub enum KindEdit {
    Sticker(StickerEdit),
    /// The reactions the caller leaves on the object; none takes them away.
    Reaction(Vec<String>),
    /// A die takes a new value, a wheel a new winner, each with the spin time.
    Roll,
    /// The faces of a die; its value stays among them.
    Faces(u32),
    /// Hide (true) or show (false) the children of a mind-map node.
    Children(bool),
    /// A flip card turns over: the other side up, its width and height swapped.
    Flip,
    /// A flip card's new back, in the format its text is written in.
    Back(String, TextFormat),
    /// A code block's language and theme; a word left out stays.
    Code {
        language: Option<Language>,
        theme: Option<Theme>,
    },
    /// A table's cell text, or a row or column inserted or deleted.
    Table(TableEdit),
    /// A task card's status, days and description.
    Task(TaskEdit),
    /// A task card moved into a kanban cell or out of its kanban.
    Card(CardMove),
}

impl KindEdit {
    /// The word a report and a refusal use, e.g. `status`.
    #[must_use]
    pub fn field(&self) -> &'static str {
        match self {
            Self::Sticker(edit) => edit.field(),
            Self::Reaction(_) => reaction::FIELD,
            Self::Roll => chance::ROLL,
            Self::Faces(_) => chance::FACES,
            Self::Children(_) => mind_map::CHILDREN,
            Self::Flip => flipcard::FLIP,
            Self::Back(..) => flipcard::BACK,
            Self::Code { .. } => code::CODE,
            Self::Table(edit) => edit.field(),
            Self::Task(edit) => edit.field(),
            Self::Card(edit) => edit.field(),
        }
    }

    /// Writes the edit: the fields to set on the object and the other objects
    /// it touched.
    ///
    /// # Errors
    /// When the document cannot take the write.
    pub(super) fn write(
        &self,
        transaction: &mut TransactionMut<'_>,
        scope: &WriteTransaction<'_>,
        id: &str,
        fields: &Value,
    ) -> Result<Written> {
        match self {
            Self::Sticker(edit) => edit.write(transaction, scope, id),
            Self::Reaction(wanted) => reaction::write(wanted, transaction, scope, id),
            Self::Roll => chance::roll(scope, id, fields),
            Self::Faces(faces) => Ok(chance::faces(*faces, fields)),
            Self::Children(hidden) => Ok(mind_map::write(*hidden)),
            Self::Flip => Ok(flipcard::flip(fields)),
            Self::Back(text, format) => {
                Ok(flipcard::back(transaction, scope, fields, (text, *format)))
            }
            Self::Code { language, theme } => Ok(code::write(*language, *theme)),
            Self::Table(edit) => table::write(edit, transaction, scope, id, fields),
            Self::Task(edit) => task::write(edit, transaction, scope, fields),
            Self::Card(edit) => kanban::write(edit, transaction, scope, (id, fields)),
        }
    }

    /// Refuses a kind that does not take this edit
    /// ([`crate::error::board_change::BoardChangeError::NotChangeable`]).
    ///
    /// # Errors
    /// When the kind does not take the edit.
    pub(super) fn check(
        &self,
        kind: &ObjectKind,
        states: &HashMap<String, Value>,
        id: &str,
    ) -> Result<()> {
        match self {
            Self::Sticker(edit) => edit.check(kind),
            Self::Reaction(_) => reaction::check(kind),
            Self::Roll => chance::check_roll(kind, states.get(id)),
            Self::Faces(_) => chance::check_faces(kind),
            Self::Children(_) => mind_map::check(kind),
            Self::Flip => flipcard::check(kind, flipcard::FLIP),
            Self::Back(..) => flipcard::check(kind, flipcard::BACK),
            Self::Code { .. } => code::check(kind),
            Self::Table(edit) => table::check(edit, kind, states, id),
            Self::Task(edit) => task::check(edit, kind, states, id),
            Self::Card(edit) => kanban::check(edit, kind, states, id),
        }
    }
}
