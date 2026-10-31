//! A change that needs its confirmation first, and what it guards against.

use std::borrow::Cow;

use crate::error::hint::Hint;

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

/// A change without its confirmation: the change and the risk.
///
/// ```
/// use holst_core::error::confirmation::{ChangeRisk, Unconfirmed};
///
/// let err = Unconfirmed::new("deleting a template", ChangeRisk::Irreversible);
/// assert_eq!(err.to_string(), "deleting a template cannot be undone");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{change} {risk}")]
pub struct Unconfirmed {
    pub change: Cow<'static, str>,
    pub risk: ChangeRisk,
}

impl Unconfirmed {
    /// A refusal of `change`, which carries `risk`.
    #[must_use]
    pub fn new(change: impl Into<Cow<'static, str>>, risk: ChangeRisk) -> Self {
        Self {
            change: change.into(),
            risk,
        }
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
        hint::Hint,
    };

    #[test]
    fn every_refusal_hints_to_confirm() {
        let err = Unconfirmed::new("deleting permanently", ChangeRisk::Irreversible);
        assert_eq!(err.hint(), Some(Hint::ConfirmFirst));
    }

    #[test]
    fn each_risk_has_its_words() {
        let said: Vec<String> = ChangeRisk::iter()
            .map(|risk| Unconfirmed::new("granting a right", risk).to_string())
            .collect();
        let undone = said.iter().filter(|text| text.contains("cannot be undone"));
        assert_eq!(undone.count(), 1, "{said:?}");
        assert!(
            said.iter()
                .all(|text| text.starts_with("granting a right ")),
            "{said:?}"
        );
    }
}
