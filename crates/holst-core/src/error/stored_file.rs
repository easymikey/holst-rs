//! Files of a board: the originals of images and file objects.

use holst_macros::Hint;

use crate::domain::board::ObjectKind;

/// A file that cannot be named or fetched.
#[derive(Debug, thiserror::Error, Hint)]
#[cfg_attr(test, derive(strum::EnumDiscriminants))]
#[cfg_attr(test, strum_discriminants(derive(strum::EnumIter, PartialOrd, Ord)))]
pub enum StoredFileError {
    #[error(
        "the {} {object} has no file to download: only images and files have one",
        kind.short_name()
    )]
    NoFile { object: String, kind: ObjectKind },

    #[error(
        "\"{reference}\" names no object: give the link of the image or file, with ?objectId="
    )]
    NoObject { reference: String },

    #[error(
        "{address} is not in the public storage {storage}: files come from there only"
    )]
    ForeignStorage { address: String, storage: String },
}

#[cfg(test)]
pub(super) mod tests {
    use crate::{
        domain::board::ObjectKind,
        error::{
            hint::decided::assert_all_listed,
            stored_file::{StoredFileError, StoredFileErrorDiscriminants},
        },
    };

    pub(in crate::error) fn samples() -> Vec<StoredFileError> {
        vec![
            StoredFileError::NoFile {
                object: "o1".into(),
                kind: ObjectKind::Sticker,
            },
            StoredFileError::NoObject {
                reference: "b1".into(),
            },
            StoredFileError::ForeignStorage {
                address: "https://evil.test/a.png".into(),
                storage: "https://storage-edge.app.holst.so/storage/".into(),
            },
        ]
    }

    #[test]
    fn every_variant_has_a_hint_decision() {
        assert_all_listed::<_, StoredFileErrorDiscriminants>(&samples());
    }
}
