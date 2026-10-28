//! Checks of user values side by side: every failure, not only the first.

use crate::error::{Error, Result, input::InputError};

/// Results of checks that do not depend on each other, combined into one:
/// both values, or one error naming every failure.
///
/// ```
/// use holst_core::Error;
/// use holst_core::error::input::InputError;
/// use holst_core::error::check_all::both;
/// use holst_core::domain::geometry::Finite;
/// use holst_core::domain::geometry::Positive;
///
/// let two = both(Finite::new(f64::NAN), Positive::new(0.0));
/// assert!(matches!(
///     two,
///     Err(Error::Input(InputError::Several { ref errors })) if errors.len() == 2
/// ));
/// let one = both(Finite::new(1.0), Positive::new(0.0));
/// assert!(matches!(one, Err(Error::Input(InputError::NotPositive { .. }))));
/// ```
///
/// # Errors
/// The only failure as it is, or [`InputError::Several`] for more than one.
pub fn both<First, Second>(
    first: Result<First>,
    second: Result<Second>,
) -> Result<(First, Second)> {
    match (first, second) {
        (Ok(first), Ok(second)) => Ok((first, second)),
        (first, second) => Err(Error::several(
            [first.err(), second.err()].into_iter().flatten(),
        )),
    }
}

impl Error {
    /// Returns one error for `errors`: the only one as it is, else
    /// [`InputError::Several`] with nested lists flattened.
    ///
    /// ```
    /// use holst_core::{Error, error::input::InputError};
    ///
    /// let one = Error::several([Error::from(InputError::FullUrl)]);
    /// assert_eq!(
    ///     one.to_string(),
    ///     Error::from(InputError::FullUrl).to_string()
    /// );
    /// ```
    #[must_use]
    pub fn several(errors: impl IntoIterator<Item = Error>) -> Error {
        let problems: Vec<Error> =
            errors.into_iter().flat_map(Error::into_errors).collect();
        match <[Error; 1]>::try_from(problems) {
            Ok([only]) => only,
            Err(problems) => Error::Input(InputError::Several { errors: problems }),
        }
    }

    fn into_errors(self) -> Vec<Error> {
        match self {
            Error::Input(InputError::Several { errors: problems }) => problems,
            Error::Login(_)
            | Error::Http(_)
            | Error::Input(_)
            | Error::LocalData(_)
            | Error::Lookup(_)
            | Error::BoardSocket(_)
            | Error::BoardChange(_)
            | Error::Picture(_)
            | Error::StoredFile(_) => vec![self],
        }
    }
}
