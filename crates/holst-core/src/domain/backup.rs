//! Board snapshots: the default provider and the snapshot check.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::response::OtherFields,
    error::{Error, Result, http::HttpError},
};

pub const DEFAULT_PROVIDER: &str = "hud";

/// Where the server keeps every backup that the list does not put in `hud`.
const OTHER_PROVIDER: &str = "hocuspocus";

const JSON_HEAD: usize = 300;

/// One snapshot of a board in the server's list; `key` downloads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedBackup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(flatten)]
    pub other: OtherFields,
}

impl ListedBackup {
    /// The backup as a line: `key · provider · date`, what the server sent.
    #[must_use]
    pub fn line(&self) -> String {
        [
            self.key.as_deref(),
            self.other.text("server_provider_type"),
            self.other.text("date"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }

    /// The provider to download the backup from: `hud` for a backup the list
    /// puts there (or does not place), `hocuspocus` for any other.
    #[must_use]
    pub fn download_provider(&self) -> &'static str {
        match self.other.text("server_provider_type") {
            Some(DEFAULT_PROVIDER) | None => DEFAULT_PROVIDER,
            Some(_) => OTHER_PROVIDER,
        }
    }
}

/// A board's snapshots, as the list endpoint answers: an object holding
/// `result`, the list. Any other reply is kept as it came.
///
/// ```
/// use holst_core::domain::backup::Backups;
///
/// let listed: Backups =
///     serde_json::from_str(r#"{"result":[{"key":"k1","date":"d"}]}"#)?;
/// assert_eq!(listed.as_slice()[0].key.as_deref(), Some("k1"));
///
/// let other: Backups = serde_json::from_str(r#"{"items":[{"key":7}]}"#)?;
/// assert!(other.as_slice().is_empty());
/// assert_eq!(serde_json::to_string(&other)?, r#"{"items":[{"key":7}]}"#);
///
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Backups {
    Listed { result: Vec<ListedBackup> },
    Unknown(Value),
}

impl Backups {
    /// The snapshots, newest first as the server sends them; none when the
    /// reply has a shape that is not known.
    #[must_use]
    pub fn as_slice(&self) -> &[ListedBackup] {
        match self {
            Self::Listed { result } => result,
            Self::Unknown(_) => &[],
        }
    }
}

/// The bytes of a board snapshot, never JSON (most likely a link to one).
///
/// ```
/// use holst_core::domain::backup::Backup;
///
/// assert!(Backup::try_from(vec![0x1f, 0x8b, 0, 1]).is_ok());
/// assert!(Backup::try_from(br#"{"url":"https://x"}"#.to_vec()).is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, derive_more::Into)]
#[into(Vec<u8>)]
pub struct Backup(Vec<u8>);

impl Backup {
    /// The snapshot bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for Backup {
    type Error = Error;

    /// # Errors
    ///
    /// [`crate::error::http::HttpError::JsonInsteadOfBackup`] when the bytes
    /// are JSON.
    fn try_from(bytes: Vec<u8>) -> Result<Self> {
        let looks_like_json = bytes
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .is_some_and(|b| matches!(b, b'{' | b'[' | b'"'));
        match looks_like_json.then(|| serde_json::from_slice::<Value>(&bytes)) {
            Some(Ok(value)) => {
                let text = value.to_string();
                let head = text
                    .get(..text.floor_char_boundary(JSON_HEAD))
                    .unwrap_or_default();
                Err(Error::Http(HttpError::JsonInsteadOfBackup {
                    head: head.to_owned(),
                }))
            }
            _ => Ok(Self(bytes)),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::{
        domain::backup::{Backup, Error, JSON_HEAD},
        error::http::HttpError,
    };

    #[derive(Debug, PartialEq, Eq)]
    enum Saving {
        Saved,
        NotSaved,
    }

    #[rstest]
    #[case::json_object(br#"{"url":"https://x/y"}"#, Saving::NotSaved)]
    #[case::json_string(b"  \"https://x\"", Saving::NotSaved)]
    #[case::json_array(b"[1]", Saving::NotSaved)]
    #[case::gzip(&[0x1f, 0x8b, 0, 1], Saving::Saved)]
    #[case::broken_json(b"{not json", Saving::Saved)]
    #[case::empty(b"", Saving::Saved)]
    fn backup_json_is_not_saved_as_snapshot(
        #[case] bytes: &[u8],
        #[case] expected: Saving,
    ) {
        let saving = Backup::try_from(bytes.to_vec())
            .map_or(Saving::NotSaved, |_| Saving::Saved);
        assert_eq!(saving, expected);
    }

    #[test]
    fn snapshot_keeps_bytes() {
        let bytes = vec![0x1f, 0x8b, 0, 1];
        assert_eq!(
            Backup::try_from(bytes.clone()).map(Vec::from).ok(),
            Some(bytes)
        );
    }

    #[test]
    fn json_head_is_cut_on_a_char_boundary() {
        let long = format!("\"{}\"", "я".repeat(400)).into_bytes();
        let expected = format!("\"{}", "я".repeat((JSON_HEAD - 1) / 2));
        let cut = Backup::try_from(long)
            .err()
            .map(|error| format!("{error:?}"));
        assert_eq!(
            cut,
            Some(format!(
                "{:?}",
                Error::Http(HttpError::JsonInsteadOfBackup { head: expected })
            ))
        );
    }
}
