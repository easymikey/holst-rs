//! Workspace, folder, board and object ids, only made by parsing.

use std::str::FromStr;

use derive_more::{AsRef, Display, Into};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result, input::InputError};

/// A board id — a canonical UUID in lowercase.
///
/// ```
/// use holst_core::domain::id::BoardId;
///
/// let id: BoardId = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b".parse()?;
/// assert_eq!(id.as_str(), "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b");
/// assert!("Plans".parse::<BoardId>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Display, AsRef, Into, Serialize, Deserialize,
)]
#[as_ref(str)]
#[into(String)]
#[serde(into = "String", try_from = "String")]
pub struct BoardId(String);

impl BoardId {
    /// The lowercase id from a UUID, a board or share link, or a `boardId=`.
    ///
    /// ```
    /// use holst_core::domain::id::BoardId;
    ///
    /// let link = "https://app.holst.so/board/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b?x=1";
    /// assert!(BoardId::find_in(link).is_some());
    /// assert!(BoardId::find_in("https://app.holst.so/dashboard").is_none());
    /// ```
    #[must_use]
    pub fn find_in(input: &str) -> Option<Self> {
        match input.trim() {
            id if is_uuid(id) => Some(id),
            link => {
                let end = memchr::memchr2(b'?', b'#', link.as_bytes());
                let path = end.and_then(|end| link.get(..end)).unwrap_or(link);
                in_path(path)
                    .or_else(|| query_value(link, "boardId").filter(|id| is_uuid(id)))
            }
        }
        .map(Self::lowercase)
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn lowercase(id: &str) -> Self {
        Self(id.to_ascii_lowercase())
    }
}

/// A workspace id — a UUID in lowercase: the server names every workspace by
/// one.
///
/// ```
/// use holst_core::domain::id::WorkspaceId;
///
/// let id: WorkspaceId = "AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B".parse()?;
/// assert_eq!(id.as_str(), "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b");
/// assert!("easy".parse::<WorkspaceId>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Display, AsRef, Into, Serialize, Deserialize,
)]
#[as_ref(str)]
#[into(String)]
#[serde(into = "String", try_from = "String")]
pub struct WorkspaceId(String);

impl WorkspaceId {
    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for WorkspaceId {
    type Err = Error;

    /// A bare canonical UUID, lowercased.
    ///
    /// # Errors
    ///
    /// [`InputError::NotWorkspaceId`] for anything else.
    fn from_str(raw: &str) -> Result<Self> {
        parse_uuid(raw, |raw| InputError::NotWorkspaceId { raw }).map(Self)
    }
}

impl TryFrom<String> for WorkspaceId {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self> {
        raw.parse()
    }
}

/// A folder id — a UUID in lowercase: folders and boards are channels, and the
/// server names every channel by one.
///
/// ```
/// use holst_core::domain::id::FolderId;
///
/// let id: FolderId = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b".parse()?;
/// assert_eq!(id.as_str(), "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b");
/// assert!("Plans".parse::<FolderId>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Display, AsRef, Into, Serialize, Deserialize,
)]
#[as_ref(str)]
#[into(String)]
#[serde(into = "String", try_from = "String")]
pub struct FolderId(String);

impl FolderId {
    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for FolderId {
    type Err = Error;

    /// A bare canonical UUID, lowercased.
    ///
    /// # Errors
    ///
    /// [`InputError::NotFolderId`] for anything else.
    fn from_str(raw: &str) -> Result<Self> {
        parse_uuid(raw, |raw| InputError::NotFolderId { raw }).map(Self)
    }
}

impl TryFrom<String> for FolderId {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self> {
        raw.parse()
    }
}

/// An object id: the key of the object in the board's `objects` map. The web
/// client makes UUIDs, but the board stores any key, so the rule is only a
/// non-empty id with no whitespace or control characters; the case is kept.
///
/// ```
/// use holst_core::domain::id::ObjectId;
///
/// let id: ObjectId = "00000000-0000-4000-8000-0000000000a5".parse()?;
/// assert_eq!(id.as_str(), "00000000-0000-4000-8000-0000000000a5");
/// assert!("two words".parse::<ObjectId>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Display, AsRef, Into, Serialize, Deserialize,
)]
#[as_ref(str)]
#[into(String)]
#[serde(into = "String", try_from = "String")]
pub struct ObjectId(String);

impl ObjectId {
    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ObjectId {
    type Err = Error;

    /// The text itself when it is a non-empty run of visible characters.
    ///
    /// # Errors
    ///
    /// [`InputError::NotObjectId`] for an empty text, or one with whitespace or
    /// control characters.
    fn from_str(raw: &str) -> Result<Self> {
        if !raw.is_empty() && raw.chars().all(|c| !c.is_whitespace() && !c.is_control())
        {
            Ok(Self(raw.to_owned()))
        } else {
            Err(Error::Input(InputError::NotObjectId {
                raw: raw.to_owned(),
            }))
        }
    }
}

impl TryFrom<String> for ObjectId {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self> {
        raw.parse()
    }
}

impl TryFrom<String> for BoardId {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self> {
        raw.parse()
    }
}

fn parse_uuid(raw: &str, refusal: impl FnOnce(String) -> InputError) -> Result<String> {
    if is_uuid(raw) {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(Error::Input(refusal(raw.to_owned())))
    }
}

const BOARD_SEGMENTS: [&str; 2] = ["/board/", "/share/b/"];

/// The workspace id of a `…/workspace/{id}` link, lowercase.
///
/// ```
/// use holst_core::domain::id::workspace_in;
///
/// let link =
///     "https://app.holst.so/workspace/AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B/boards";
/// assert_eq!(
///     workspace_in(link).as_ref().map(|id| id.as_str()),
///     Some("ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b")
/// );
/// assert!(workspace_in("easy").is_none());
/// ```
#[must_use]
pub fn workspace_in(link: &str) -> Option<WorkspaceId> {
    let link = link.trim();
    let path = link.split(['?', '#']).next().unwrap_or(link);
    memchr::memmem::find_iter(path.as_bytes(), b"/workspace/")
        .filter_map(|at| path.get(at + "/workspace/".len()..))
        .find_map(segment_uuid)
        .map(|id| WorkspaceId(id.to_ascii_lowercase()))
}

fn in_path(path: &str) -> Option<&str> {
    // Slash by slash: a name has none, and a `memmem` finder costs more to build.
    BOARD_SEGMENTS.iter().find_map(|segment| {
        memchr::memchr_iter(b'/', path.as_bytes())
            .filter_map(|at| path.get(at..)?.strip_prefix(segment))
            .find_map(segment_uuid)
    })
}

fn query_value<'a>(link: &'a str, key: &str) -> Option<&'a str> {
    let query = link.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
}

/// Returns the `objectId=` of a link, as written, if it has a usable one.
///
/// ```
/// use holst_core::domain::id::object_in;
///
/// let link = "https://app.holst.so/share/b/00000000-0000-4000-8000-0000000000a3\
///             ?objectId=00000000-0000-4000-8000-0000000000A4";
/// assert_eq!(
///     object_in(link).as_ref().map(|id| id.as_str()),
///     Some("00000000-0000-4000-8000-0000000000A4")
/// );
/// assert_eq!(object_in("https://app.holst.so/board/b1"), None);
/// ```
#[must_use]
pub fn object_in(link: &str) -> Option<ObjectId> {
    query_value(link.trim(), "objectId").and_then(|id| id.parse().ok())
}

fn segment_uuid(rest: &str) -> Option<&str> {
    rest.get(..36).filter(|id| {
        matches!(rest.as_bytes().get(36), None | Some(b'/')) && is_uuid(id)
    })
}

impl FromStr for BoardId {
    type Err = Error;

    /// A bare canonical UUID, lowercased; links and names are rejected.
    ///
    /// # Errors
    ///
    /// [`crate::error::input::InputError::NotBoardId`] for anything else.
    fn from_str(raw: &str) -> Result<Self> {
        parse_uuid(raw, |raw| InputError::NotBoardId { raw }).map(Self)
    }
}

/// A canonical 8-4-4-4-12 UUID, either case; one pass, no allocation.
///
/// ```
/// use holst_core::domain::id::is_uuid;
///
/// assert!(is_uuid("ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b"));
/// assert!(!is_uuid("ae1f0a2bc3d44f198a7b1c2d3e4f5a6b"));
/// ```
#[must_use]
pub fn is_uuid(value: &str) -> bool {
    value.len() == UUID_LEN
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

pub const UUID_LEN: usize = 36;

/// Byte ranges of the UUIDs in `text` with no hex digit on either side, in
/// order; the scan jumps from hyphen to hyphen, a UUID starting 8 bytes before
/// one.
///
/// ```
/// use holst_core::domain::id::uuid_spans;
///
/// let text = "board ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b is gone";
/// assert_eq!(uuid_spans(text).collect::<Vec<_>>(), [(6, 42)]);
/// assert_eq!(
///     uuid_spans("fae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b").count(),
///     0
/// );
/// ```
pub fn uuid_spans(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    const FIRST_HYPHEN: usize = 8;
    let bytes = text.as_bytes();
    let hex = move |at: usize| bytes.get(at).is_some_and(u8::is_ascii_hexdigit);
    memchr::memchr_iter(b'-', bytes)
        .filter_map(|hyphen| hyphen.checked_sub(FIRST_HYPHEN))
        .filter(move |&start| !start.checked_sub(1).is_some_and(hex))
        .filter_map(move |start| {
            let end = start.saturating_add(UUID_LEN);
            text.get(start..end)
                .filter(|id| is_uuid(id) && !hex(end))
                .map(|_| (start, end))
        })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::domain::id::{
        BoardId,
        FolderId,
        ObjectId,
        WorkspaceId,
        is_uuid,
        object_in,
        workspace_in,
    };

    const ID: &str = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b";

    fn found(input: &str) -> Option<String> {
        BoardId::find_in(input).map(String::from)
    }

    #[rstest]
    #[case::plain_uuid(ID, Some(ID))]
    #[case::padded_uuid("  ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b  ", Some(ID))]
    #[case::board_url(
        "https://app.holst.so/board/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b",
        Some(ID)
    )]
    #[case::query_and_fragment(
        "https://app.holst.so/board/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b?invite=xyz#frame-2",
        Some(ID)
    )]
    #[case::workspace_segment(
        "https://app.holst.so/workspace/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b/boards",
        None
    )]
    #[case::uuid_in_name("Retro ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b", None)]
    #[case::no_uuid("https://app.holst.so/dashboard", None)]
    #[case::empty("", None)]
    #[case::trailing_slash(
        "https://app.holst.so/board/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b/",
        Some(ID)
    )]
    #[case::upper_case_is_lowered("AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B", Some(ID))]
    #[case::upper_case_link(
        "https://app.holst.so/board/AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B",
        Some(ID)
    )]
    #[case::share_link(
        "https://app.holst.so/share/b/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b?objectId=o1",
        Some(ID)
    )]
    #[case::desktop_link(
        "https://app.holst.so/open-in-desktop?objectId=o1&boardId=ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b",
        Some(ID)
    )]
    #[case::board_id_in_fragment_only(
        "https://app.holst.so/x#boardId=ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b",
        None
    )]
    #[case::board_id_not_a_uuid(
        "https://app.holst.so/open-in-desktop?boardId=b1",
        None
    )]
    #[case::other_parameter_ending_in_board_id(
        "https://app.holst.so/x?parentBoardId=ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b",
        None
    )]
    fn board_id_is_found_in_links(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(found(input).as_deref(), expected);
    }

    #[derive(Debug, PartialEq)]
    enum Verdict {
        Uuid,
        NotUuid,
    }

    #[rstest]
    #[case::canonical(ID, Verdict::Uuid)]
    #[case::no_hyphens("ae1f0a2bc3d44f198a7b1c2d3e4f5a6b", Verdict::NotUuid)]
    #[case::not_hex("zzzzzzzz-c3d4-4f19-8a7b-1c2d3e4f5a6b", Verdict::NotUuid)]
    // Same length and hyphen count, but the hyphens are misplaced.
    #[case::hyphens_misplaced("ae1f0a2bf-e64-4f19-8a7b-1c2d3e4f5a6b", Verdict::NotUuid)]
    #[case::same_length_no_hyphens(
        "ae1f0a2bc3d44f198a7b1c2d3e4f5a6babcd",
        Verdict::NotUuid
    )]
    #[case::multibyte_same_length(
        "аe1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6",
        Verdict::NotUuid
    )]
    fn uuid_check(#[case] value: &str, #[case] expected: Verdict) {
        let verdict = is_uuid(value).then_some(Verdict::Uuid);
        assert_eq!(verdict.unwrap_or(Verdict::NotUuid), expected);
    }

    #[test]
    fn display_and_as_ref_show_the_uuid() {
        let shown = ID
            .parse::<BoardId>()
            .map(|id| (id.to_string(), AsRef::<str>::as_ref(&id).to_owned()));
        assert_eq!(shown.ok(), Some((ID.to_owned(), ID.to_owned())));
        assert_eq!(
            "x".parse::<BoardId>().err().map(|e| e.to_string()),
            Some(
                "\"x\" is not a board id: expected a UUID like \
                 00000000-0000-0000-0000-000000000000"
                    .to_owned()
            )
        );
    }

    #[derive(Debug, PartialEq)]
    enum Parsed {
        Id(String),
        Refused,
    }

    fn parsed<T: std::str::FromStr + Into<String>>(raw: &str) -> Parsed {
        raw.parse::<T>()
            .ok()
            .map_or(Parsed::Refused, |id| Parsed::Id(id.into()))
    }

    #[rstest]
    #[case::name("easy", Parsed::Refused)]
    #[case::link(
        "https://app.holst.so/workspace/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b",
        Parsed::Refused
    )]
    #[case::padded(" ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b", Parsed::Refused)]
    #[case::empty("", Parsed::Refused)]
    fn uuid_ids_parse_only_bare_uuids(#[case] raw: &str, #[case] expected: Parsed) {
        assert_eq!(parsed::<WorkspaceId>(raw), expected);
        assert_eq!(parsed::<FolderId>(raw), expected);
    }

    #[rstest]
    #[case::uuid(ID, Parsed::Id(ID.to_owned()))]
    #[case::case_is_kept("Obj-A", Parsed::Id("Obj-A".to_owned()))]
    #[case::short("o1", Parsed::Id("o1".to_owned()))]
    #[case::empty("", Parsed::Refused)]
    #[case::space("two words", Parsed::Refused)]
    #[case::padded(" o1", Parsed::Refused)]
    #[case::newline("o1\n", Parsed::Refused)]
    #[case::control("o\u{7}1", Parsed::Refused)]
    fn object_id_parses_visible_text(#[case] raw: &str, #[case] expected: Parsed) {
        assert_eq!(parsed::<ObjectId>(raw), expected);
    }

    #[rstest]
    #[case::workspace("x", "is not a workspace id")]
    #[case::folder("x", "is not a folder id")]
    #[case::object("", "is not an object id")]
    fn refusals_name_the_kind(#[case] raw: &str, #[case] words: &str) {
        let messages = [
            raw.parse::<WorkspaceId>().err().map(|e| e.to_string()),
            raw.parse::<FolderId>().err().map(|e| e.to_string()),
            raw.parse::<ObjectId>().err().map(|e| e.to_string()),
        ];
        assert!(
            messages
                .iter()
                .flatten()
                .any(|message| message.contains(words)),
            "{messages:?}"
        );
    }

    #[test]
    fn ids_refuse_bad_json_strings() {
        assert!(serde_json::from_str::<WorkspaceId>("\"easy\"").is_err());
        assert!(serde_json::from_str::<BoardId>("7").is_err());
        assert!(serde_json::from_str::<ObjectId>("\"\"").is_err());
    }

    #[rstest]
    #[case::any_visible_key(
        "https://app.holst.so/share/b/x?objectId=Obj-A",
        Some("Obj-A")
    )]
    #[case::second_parameter(
        "https://app.holst.so/open?boardId=x&objectId=o1#top",
        Some("o1")
    )]
    #[case::other_parameter_ending_the_same(
        "https://app.holst.so/x?parentObjectId=X",
        None
    )]
    #[case::empty("https://app.holst.so/x?objectId=", None)]
    fn object_in_reads_the_whole_pair(
        #[case] link: &str,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(object_in(link).as_ref().map(ObjectId::as_str), expected);
    }

    #[test]
    fn links_give_typed_ids() {
        let link = "https://app.holst.so/workspace/AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B/boards?objectId=B5D77851-99C0-46C3-ABC2-BFB7FEE160DF";
        assert_eq!(workspace_in(link), ID.parse::<WorkspaceId>().ok());
        assert_eq!(
            object_in(link),
            "B5D77851-99C0-46C3-ABC2-BFB7FEE160DF"
                .parse::<ObjectId>()
                .ok()
        );
    }
}
