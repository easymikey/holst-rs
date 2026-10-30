//! A change that needs its confirmation first, and what it guards against.

use std::borrow::Cow;

use crate::error::hint::{Hint, Surface};

/// What a confirmation guards against; the refusal names it after the action.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display, strum::EnumIter,
)]
pub enum ChangeRisk {
    #[display("cannot be undone")]
    Irreversible,
    #[display("changes who can open or edit it")]
    Sharing,
    #[display("changes it for everyone in the workspace")]
    Everyone,
    #[display("takes everything inside along")]
    Contents,
}

/// A change without its confirmation: the change, the risk, how to confirm.
///
/// ```
/// use holst_core::error::{
///     confirmation::{ChangeRisk, Unconfirmed},
///     hint::Surface,
/// };
///
/// let err = Unconfirmed::new("deleting a template", ChangeRisk::Irreversible);
/// assert_eq!(err.to_string(), "deleting a template cannot be undone");
/// assert_eq!(
///     err.on(Surface::Cli),
///     "deleting a template cannot be undone; confirm with --yes"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{change} {risk}")]
pub struct Unconfirmed {
    pub change: Cow<'static, str>,
    pub risk: ChangeRisk,
}

impl Unconfirmed {
    /// A refusal whose flag word a shell adds with [`Unconfirmed::on`].
    #[must_use]
    pub fn new(change: impl Into<Cow<'static, str>>, risk: ChangeRisk) -> Self {
        Self {
            change: change.into(),
            risk,
        }
    }

    /// The refusal worded the way `surface` confirms.
    #[must_use]
    pub fn on(&self, surface: Surface) -> String {
        format!(
            "{} {}; confirm with {}",
            self.change,
            self.risk,
            surface.confirmation()
        )
    }

    /// The change and its risk as one phrase, for a report that holds it.
    #[must_use]
    pub fn needs(&self) -> String {
        self.to_string()
    }

    /// Advice for every unconfirmed action.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        Some(Hint::ConfirmFirst)
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use crate::error::{
        confirmation::{ChangeRisk, Unconfirmed},
        hint::{Hint, Surface},
    };

    #[test]
    fn every_refusal_hints_to_confirm() {
        let err = Unconfirmed::new("deleting permanently", ChangeRisk::Irreversible);
        assert_eq!(err.hint(), Some(Hint::ConfirmFirst));
    }

    #[test]
    fn a_held_change_needs_its_change_and_risk() {
        let err = Unconfirmed::new("deleting permanently", ChangeRisk::Irreversible);
        assert_eq!(err.needs(), "deleting permanently cannot be undone");
    }

    #[test]
    fn the_shell_chooses_the_wording() {
        let err = Unconfirmed::new("deleting permanently", ChangeRisk::Irreversible);
        assert_eq!(
            err.on(Surface::Mcp),
            "deleting permanently cannot be undone; confirm with confirm: true"
        );
        assert_eq!(
            err.on(Surface::Cli),
            "deleting permanently cannot be undone; confirm with --yes"
        );
    }

    #[test]
    fn each_risk_has_its_words() {
        let said: Vec<String> = ChangeRisk::iter()
            .map(|risk| Unconfirmed::new("granting a right", risk).on(Surface::Mcp))
            .collect();
        let undone = said.iter().filter(|text| text.contains("cannot be undone"));
        assert_eq!(undone.count(), 1, "{said:?}");
        assert!(
            said.iter()
                .all(|text| text.ends_with("; confirm with confirm: true")),
            "{said:?}"
        );
    }
}
