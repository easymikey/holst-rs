//! Files of a board: the originals of images and file objects.

use crate::error::hint::Hint;

/// A file that cannot be named or fetched.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum StoredFileError {
    #[error(
        "the {kind} {object} has no file to download: only images and files have one"
    )]
    NoFile { object: String, kind: String },

    #[error(
        "\"{reference}\" names no object: give the link of the image or file, with ?objectId="
    )]
    NoObject { reference: String },

    #[error(
        "{address} is not in the public storage {storage}: files come from there only"
    )]
    ForeignStorage { address: String, storage: String },
}

impl StoredFileError {
    /// What to do next, decided for every variant: the message says it all.
    #[must_use]
    pub const fn hint(&self) -> Option<Hint> {
        match self {
            StoredFileError::NoFile { .. }
            | StoredFileError::NoObject { .. }
            | StoredFileError::ForeignStorage { .. } => None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use crate::error::{
        hint::{Hint, decided::assert_every_variant},
        stored_file::{StoredFileError, StoredFileErrorDiscriminants},
    };

    pub(in crate::error) fn samples() -> Vec<(StoredFileError, Option<Hint>)> {
        vec![
            (
                StoredFileError::NoFile {
                    object: "o1".into(),
                    kind: "sticker".into(),
                },
                None,
            ),
            (
                StoredFileError::NoObject {
                    reference: "b1".into(),
                },
                None,
            ),
            (
                StoredFileError::ForeignStorage {
                    address: "https://evil.test/a.png".into(),
                    storage: "https://storage-edge.app.holst.so/storage/".into(),
                },
                None,
            ),
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_every_variant::<_, StoredFileErrorDiscriminants>(
            samples(),
            StoredFileError::hint,
        );
    }
}
