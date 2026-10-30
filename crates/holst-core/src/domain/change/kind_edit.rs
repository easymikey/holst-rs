//! The edits only some kinds take, one variant and one file per kind.

mod reaction;
mod sticker;

use std::collections::HashMap;

use serde_json::Value;
use yrs::TransactionMut;

pub use self::{
    reaction::MOST_KINDS,
    sticker::{StickerEdit, TextSize},
};
use crate::{
    domain::{
        board::ObjectKind,
        change::{value::ChangedObject, write::WriteTransaction},
    },
    error::Result,
};

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
}

impl KindEdit {
    /// The word a report and a refusal use, e.g. `status`.
    #[must_use]
    pub fn field(&self) -> &'static str {
        match self {
            Self::Sticker(edit) => edit.field(),
            Self::Reaction(_) => reaction::FIELD,
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
        _fields: &Value,
    ) -> Result<Written> {
        match self {
            Self::Sticker(edit) => edit.write(transaction, scope, id),
            Self::Reaction(wanted) => reaction::write(wanted, transaction, scope, id),
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
        _states: &HashMap<String, Value>,
        _id: &str,
    ) -> Result<()> {
        match self {
            Self::Sticker(edit) => edit.check(kind),
            Self::Reaction(_) => reaction::check(kind),
        }
    }
}
