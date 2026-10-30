//! The plan of a board write, decided before anything is sent; pure.

use yrs::Doc;

use crate::{
    domain::{
        board::EditRight,
        change::{
            receipt::{BoardStatus, ChangeReport, Delivery, UpdateReceipt},
            value::{BoardChange, ChangeSources, ChangeUpdate, RemoveRight},
        },
        rich_text::kept_note,
    },
    effect::confirmation::ReleaseCheck,
    error::{Error, Result, board_change::BoardChangeError},
};

/// An update the server confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedUpdate {
    pub id: u64,
}

/// A change built on a loaded board and not yet sent.
#[derive(Debug, Clone, PartialEq)]
pub struct WritePlan {
    pub change: ChangeUpdate,
    pub update_id: u64,
    pub placement: BoardStatus,
    pub release: Release,
}

/// Whether a built change goes out or stays held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    Released,
    Held,
}

/// What a write plan is built from.
pub struct LoadedChange<'a> {
    pub document: &'a Doc,
    pub right: EditRight,
    pub placement: BoardStatus,
    pub change: &'a BoardChange,
    pub removal: RemoveRight,
    pub account_id: &'a str,
    pub context: &'a ChangeSources<'a>,
    pub release_check: ReleaseCheck,
}

// `ChangeSources` has no `Debug`: it is left out.
impl std::fmt::Debug for LoadedChange<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanInput")
            .field("right", &self.right)
            .field("placement", &self.placement)
            .field("change", &self.change)
            .field("user", &self.account_id)
            .finish_non_exhaustive()
    }
}

/// Builds the operation of `input` on its doc into the plan of a write; the
/// update id is drawn from `context` after the build.
///
/// # Errors
/// [`crate::error::board_change::BoardChangeError::ReadOnly`] unless the
/// board is editable, or the errors of [`crate::domain::change::build::build`].
pub fn plan(input: &LoadedChange<'_>) -> Result<WritePlan> {
    let built = if matches!(input.right, EditRight::Editable) {
        crate::domain::change::build::build_with(
            input.document,
            input.change,
            input.account_id,
            input.context,
            input.removal,
        )
    } else {
        Err(Error::BoardChange(BoardChangeError::ReadOnly))
    }?;
    let update_id =
        crate::domain::change::build::update_id(&input.context.ids.new_id());
    let release = if (input.release_check)(&built) {
        Release::Released
    } else {
        Release::Held
    };
    Ok(WritePlan {
        change: built,
        update_id,
        placement: input.placement,
        release,
    })
}

impl WritePlan {
    /// The write this plan became: confirmed by `accepted`, or never sent.
    #[must_use]
    pub fn written(self, accepted: Option<AcceptedUpdate>) -> WrittenChange {
        WrittenChange {
            change: self.change,
            update_id: self.update_id,
            accepted,
            placement: self.placement,
        }
    }
}

/// A change built on the board and, unless dry, confirmed by the server.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenChange {
    pub change: ChangeUpdate,
    pub update_id: u64,
    pub accepted: Option<AcceptedUpdate>,
    pub placement: BoardStatus,
}

impl WrittenChange {
    /// The receipt of the update: built only, or confirmed.
    #[must_use]
    pub const fn receipt(&self) -> UpdateReceipt {
        UpdateReceipt {
            bytes: self.change.update.len(),
            id: self.update_id,
            delivery: match self.accepted {
                Some(_) => Delivery::Accepted,
                None => Delivery::Built,
            },
        }
    }

    /// The outcome on `board`, each object with the link `link` gives it.
    #[must_use]
    pub fn report(&self, board: &str, link: impl Fn(&str) -> String) -> ChangeReport {
        ChangeReport {
            board: board.to_owned(),
            receipt: self.receipt(),
            objects: self
                .change
                .objects
                .iter()
                .map(|touched| touched.with_link(&link))
                .collect(),
            warnings: kept_note(&self.change.kept).into_iter().collect(),
            board_status: self.placement,
        }
    }
}
