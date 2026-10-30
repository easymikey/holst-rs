//! Helpers the tests of several crates share.

use crate::error::lookup::{Candidate, Candidates};

/// Candidates that carry only the given names.
#[must_use]
pub fn candidates(names: &[&str]) -> Candidates {
    names
        .iter()
        .map(|name| Candidate {
            name: (*name).to_owned(),
            ..Candidate::default()
        })
        .collect()
}

/// Fails the test, naming the `rule`, when `offenders` is not empty.
/// Panics in the same case.
pub fn assert_none(rule: &str, offenders: &[String]) {
    assert!(
        offenders.is_empty(),
        "{rule}: {} offenders\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}
