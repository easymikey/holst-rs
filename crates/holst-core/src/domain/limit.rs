//! How many results a search keeps.

use std::num::NonZeroUsize;

/// How many results to keep: all of them, or the best few.
///
/// ```
/// use std::num::NonZeroUsize;
///
/// use holst_core::domain::limit::Limit;
///
/// assert_eq!(Limit::from(0), Limit::All);
/// assert_eq!(
///     Limit::from(3),
///     Limit::First(NonZeroUsize::MIN.saturating_add(2))
/// );
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Limit {
    #[default]
    All,
    First(NonZeroUsize),
}

impl Limit {
    /// Returns how many results to keep, `None` for all.
    #[must_use]
    pub const fn count(self) -> Option<usize> {
        match self {
            Self::All => None,
            Self::First(count) => Some(count.get()),
        }
    }
}

impl From<usize> for Limit {
    /// The user's number: `0` keeps all.
    fn from(count: usize) -> Self {
        NonZeroUsize::new(count).map_or(Self::All, Self::First)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use rstest::rstest;

    use super::Limit;

    #[rstest]
    #[case::zero_is_all(0, None)]
    #[case::one(1, Some(1))]
    #[case::largest(usize::MAX, Some(usize::MAX))]
    fn limit_from_the_users_number(#[case] count: usize, #[case] first: Option<usize>) {
        let expected = first
            .and_then(NonZeroUsize::new)
            .map_or(Limit::All, Limit::First);
        assert_eq!(Limit::from(count), expected);
    }
}
