//! Boards and folders: kinds, parents, create and duplicate bodies.

use std::str::FromStr;

use serde::Deserialize;
use serde_json::Value;
use strum::{Display, EnumIter, IntoStaticStr};

use crate::{
    domain::{
        id::{FolderId, WorkspaceId},
        time::Timestamp,
    },
    error::{Error, input::InputError},
};

/// A board or folder name as typed: trimmed, never blank.
///
/// ```
/// use holst_core::domain::node::NodeName;
///
/// let name: NodeName = "  Plans Q4 ".parse()?;
/// assert_eq!(name.as_str(), "Plans Q4");
/// assert!("  ".parse::<NodeName>().is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, derive_more::Display, derive_more::Into)]
#[into(String)]
pub struct NodeName(String);

impl NodeName {
    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for NodeName {
    type Err = Error;

    /// # Errors
    ///
    /// [`crate::error::input::InputError::BlankName`] for an empty or
    /// whitespace-only name.
    fn from_str(raw: &str) -> Result<Self, Error> {
        Some(raw.trim())
            .filter(|name| !name.is_empty())
            .map(|name| Self(name.to_owned()))
            .ok_or(Error::Input(InputError::BlankName))
    }
}

/// Id and time of a new object, both chosen by the client.
#[derive(Debug, Clone, PartialEq)]
pub struct CreationStamp {
    pub id: String,
    pub at: Timestamp,
}

impl CreationStamp {
    /// A stamp with id `id` at the instant `at`.
    ///
    /// ```
    /// use holst_core::domain::{node::CreationStamp, time::Timestamp};
    ///
    /// let stamp = CreationStamp::at("b1", Timestamp::from_millis(1_792_577_145_404));
    /// assert_eq!(stamp.at.iso8601(), "2026-10-21T10:05:45.404Z");
    /// ```
    #[must_use]
    pub fn at(id: impl Into<String>, at: Timestamp) -> Self {
        Self { id: id.into(), at }
    }
}

/// A tree node's kind; parsing is total: unknown is [`NodeKind::Other`].
///
/// ```
/// use holst_core::domain::node::NodeKind;
///
/// assert_eq!(NodeKind::from("folder"), NodeKind::Folder);
/// assert_eq!(NodeKind::from("sticker"), NodeKind::Other);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, IntoStaticStr, EnumIter)]
#[strum(serialize_all = "lowercase")]
pub enum NodeKind {
    Board,
    Folder,
    Other,
}

impl NodeKind {
    /// The kind as text: `board`, `folder`, `other`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

impl NodeKind {
    /// Parses a kind the API creates: `board` or `folder`, in any case.
    ///
    /// # Errors
    ///
    /// [`InputError::NotNodeKind`] for any other text.
    pub fn creatable(raw: &str) -> Result<Self, Error> {
        match NodeKind::from(raw.to_ascii_lowercase().as_str()) {
            kind @ (NodeKind::Board | NodeKind::Folder) => Ok(kind),
            NodeKind::Other => Err(Error::Input(InputError::NotNodeKind {
                raw: raw.to_owned(),
            })),
        }
    }
}

impl From<&str> for NodeKind {
    fn from(raw: &str) -> Self {
        match raw {
            "board" => NodeKind::Board,
            "folder" => NodeKind::Folder,
            _ => NodeKind::Other,
        }
    }
}

/// Where a board goes: the workspace root or a folder.
#[derive(Debug, Clone, PartialEq, Eq, IntoStaticStr)]
pub enum Parent {
    #[strum(serialize = "workspace")]
    Workspace(String),
    #[strum(serialize = "channel")]
    Folder(String),
}

/// The fields of a board or folder reply that lookups, moves and copies read.
///
/// ```
/// use holst_core::domain::node::{NodeBrief, Parent};
/// use serde_json::json;
///
/// let board = json!({ "name": "Plans", "workspace_id": "w", "parent_id": "f" });
/// let info = NodeBrief::of(&board)?;
/// assert_eq!((info.name(), info.workspace_id()), ("Plans", Some("w")));
/// assert_eq!(info.parent(), Some(Parent::Folder("f".into())));
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NodeBrief {
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    workspace_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    parent_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_type")]
    #[serde(rename = "parent_type")]
    parent_kind: Option<ParentKind>,
    #[serde(default)]
    in_trash: Option<Value>,
}

fn lenient<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Value::deserialize(deserializer).map(|value| value.as_str().map(str::to_owned))
}

fn lenient_type<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ParentKind>, D::Error> {
    Value::deserialize(deserializer).map(|value| {
        value.as_str().map(|raw| {
            ParentKind::deserialize(Value::from(raw)).unwrap_or(ParentKind::Unknown)
        })
    })
}

impl NodeBrief {
    /// The fields of a channel reply; a mistyped field counts as missing.
    ///
    /// # Errors
    ///
    /// [`crate::error::http::HttpError::Json`] when the reply is no object,
    /// e.g. a string.
    pub fn of(response: &Value) -> Result<Self, Error> {
        Ok(Self::deserialize(response)?)
    }

    /// The name; empty when the reply has none.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or_default()
    }

    /// The workspace it lives in.
    #[must_use]
    pub fn workspace_id(&self) -> Option<&str> {
        self.workspace_id.as_deref()
    }

    /// It is in the trash: `in_trash` is there and not `null` or `false`.
    #[must_use]
    pub fn in_trash(&self) -> bool {
        crate::domain::tree::is_trashed(self.in_trash.as_ref())
    }

    /// Where it sits: by `parent_kind`; untyped, `workspace_id` is the root.
    #[must_use]
    pub fn parent(&self) -> Option<Parent> {
        let id = self.parent_id.clone()?;
        Some(match self.parent_kind {
            Some(ParentKind::Workspace) => Parent::Workspace(id),
            Some(ParentKind::Folder) => Parent::Folder(id),
            _ if self.workspace_id.as_ref() == Some(&id) => Parent::Workspace(id),
            _ => Parent::Folder(id),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ParentKind {
    Workspace,
    #[serde(rename = "channel", alias = "folder")]
    Folder,
    #[serde(other)]
    Unknown,
}

/// What to create: a board or a folder, its name and place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNode {
    pub name: NodeName,
    /// Board or folder; the request refuses any other.
    pub kind: NodeKind,
    pub workspace: WorkspaceId,
    pub parent: NewParent,
}

/// Where a created or moved node goes: the workspace root or a folder that
/// exists.
///
/// ```
/// use holst_core::domain::{
///     id::WorkspaceId,
///     node::{NewParent, Parent},
/// };
///
/// let workspace: WorkspaceId = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b".parse()?;
/// let root = NewParent::Root.parent_in(&workspace);
/// assert_eq!(root, Parent::Workspace(workspace.to_string()));
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewParent {
    Root,
    Folder(FolderId),
}

impl NewParent {
    /// The [`Parent`] of the wire in `workspace`.
    #[must_use]
    pub fn parent_in(&self, workspace: &WorkspaceId) -> Parent {
        match self {
            NewParent::Root => Parent::Workspace(workspace.to_string()),
            NewParent::Folder(folder) => Parent::Folder(folder.to_string()),
        }
    }
}

impl Parent {
    /// Id of the workspace or the folder.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Parent::Workspace(id) | Parent::Folder(id) => id,
        }
    }

    /// `parent_type` in the API: a folder is a `channel`.
    #[must_use]
    pub fn api_type(&self) -> &'static str {
        self.into()
    }

    /// `newParentId` of a move as the web client sends it: null for the root.
    ///
    /// ```
    /// use holst_core::domain::node::Parent;
    ///
    /// assert_eq!(Parent::Workspace("w".into()).new_parent_id(), None);
    /// assert_eq!(Parent::Folder("f".into()).new_parent_id(), Some("f"));
    /// ```
    #[must_use]
    pub fn new_parent_id(&self) -> Option<&str> {
        match self {
            Parent::Workspace(_) => None,
            Parent::Folder(id) => Some(id),
        }
    }
}

/// The name and place of a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyTarget {
    pub name: String,
    pub parent: Parent,
}

/// Whether a deletion goes to the trash or is final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMode {
    Trash,
    Permanent,
}

impl DeleteMode {
    /// Whether the board is gone for good.
    #[must_use]
    pub const fn is_permanent(self) -> bool {
        matches!(self, Self::Permanent)
    }
}

/// Whether a copy of a board gets the permissions of the source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RightsCopy {
    #[default]
    Without,
    With,
}

impl RightsCopy {
    /// Whether the rights are copied.
    #[must_use]
    pub const fn is_copied(self) -> bool {
        matches!(self, Self::With)
    }
}

/// A board to copy, where the copy goes, and what it takes along.
#[derive(Debug, Clone, Copy)]
pub struct NodeCopy<'a> {
    pub source_id: &'a str,
    pub target: &'a CopyTarget,
    pub permissions: RightsCopy,
}

/// Where a copy goes: next to `source` unless `parent` is given.
///
/// ```
/// use holst_core::domain::node::{NodeBrief, Parent, copy_target};
/// use serde_json::json;
///
/// let source = NodeBrief::of(&json!({ "name": "Plans", "parent_id": "f" }))?;
/// let copy = copy_target(&source, None, None).map(|copy| copy.name);
/// assert_eq!(copy.as_deref(), Some("Plans-copy"));
///
/// # Ok::<(), holst_core::Error>(())
/// ```
#[must_use]
pub fn copy_target(
    source: &NodeBrief,
    name: Option<&str>,
    parent: Option<Parent>,
) -> Option<CopyTarget> {
    parent.or_else(|| source.parent()).map(|parent| CopyTarget {
        name: copy_name(source.name(), name),
        parent,
    })
}

/// The name of a copy: the requested one, or `<name>-copy` by default.
///
/// ```
/// use holst_core::domain::node::copy_name;
///
/// assert_eq!(copy_name("Plans", None), "Plans-copy");
/// assert_eq!(copy_name("Plans", Some(" Mine ")), "Mine");
/// ```
#[must_use]
pub fn copy_name(source_name: &str, requested: Option<&str>) -> String {
    match (requested.map(str::trim), source_name) {
        (Some(name), _) if !name.is_empty() => name.to_owned(),
        (_, "") => "copy".to_owned(),
        (_, source) => format!("{source}-copy"),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};
    use strum::IntoEnumIterator as _;

    use crate::domain::node::{NodeBrief, NodeKind, Parent, copy_name, copy_target};

    #[rstest]
    #[case::unknown("sticker", None)]
    #[case::other("other", None)]
    #[case::empty("", None)]
    fn creatable_kind_parses_ignoring_case(
        #[case] raw: &str,
        #[case] expected: Option<NodeKind>,
    ) {
        assert_eq!(NodeKind::creatable(raw).ok(), expected);
    }

    #[test]
    fn unknown_kind_names_the_choices() {
        assert_eq!(
            NodeKind::creatable("sticker")
                .err()
                .map(|err| err.to_string()),
            Some("unknown kind \"sticker\": expected board or folder".to_owned())
        );
    }

    #[rstest]
    #[case::folder(json!({ "parent_id": "f", "parent_type": "channel" }), Some(Parent::Folder("f".into())))]
    #[case::root(json!({ "parent_id": "w", "parent_type": "workspace" }), Some(Parent::Workspace("w".into())))]
    #[case::untyped_root(json!({ "parent_id": "w", "workspace_id": "w" }), Some(Parent::Workspace("w".into())))]
    #[case::unknown_type(json!({ "parent_id": "f", "workspace_id": "w", "parent_type": "space" }), Some(Parent::Folder("f".into())))]
    #[case::no_parent(json!({ "workspace_id": "w" }), None)]
    #[case::parent_not_a_string(json!({ "parent_id": 7 }), None)]
    fn parent_of_channel(#[case] channel: Value, #[case] expected: Option<Parent>) {
        assert_eq!(
            NodeBrief::of(&channel)
                .ok()
                .and_then(|summary| summary.parent()),
            expected
        );
    }

    #[rstest]
    #[case::full(
        json!({ "name": "Plans", "workspace_id": "w", "in_trash": "2026-10-01", "extra": [1] }),
        Some(("Plans", Some("w"), true))
    )]
    #[case::empty(json!({}), Some(("", None, false)))]
    #[case::nulls(json!({ "name": null, "in_trash": null }), Some(("", None, false)))]
    #[case::not_trashed(json!({ "in_trash": false }), Some(("", None, false)))]
    #[case::mistyped(json!({ "workspace_id": 7 }), Some(("", None, false)))]
    #[case::mistyped_name_and_parent(
        json!({ "name": 7, "workspace_id": "w", "parent_id": [1], "parent_type": 3 }),
        Some(("", Some("w"), false))
    )]
    #[case::not_an_object(json!("Plans"), None)]
    fn channel_info_reads_the_reply(
        #[case] channel: Value,
        #[case] expected: Option<(&str, Option<&str>, bool)>,
    ) {
        let info = NodeBrief::of(&channel).ok();
        let got = info
            .as_ref()
            .map(|info| (info.name(), info.workspace_id(), info.in_trash()));
        assert_eq!(got, expected);
    }

    #[rstest]
    #[case::next_to_source(
        json!({ "name": "Plans", "parent_id": "f", "parent_type": "channel" }),
        None,
        None,
        Some(("Plans-copy", Parent::Folder("f".into())))
    )]
    #[case::named_into_a_folder(
        json!({ "name": "Plans", "parent_id": "w", "workspace_id": "w" }),
        Some(" Mine "),
        Some(Parent::Folder("g".into())),
        Some(("Mine", Parent::Folder("g".into())))
    )]
    #[case::untyped_root(
        json!({ "name": "Plans", "parent_id": "w", "workspace_id": "w" }),
        None,
        None,
        Some(("Plans-copy", Parent::Workspace("w".into())))
    )]
    #[case::no_parent(json!({ "name": "Plans" }), None, None, None)]
    fn duplicate_target_cases(
        #[case] source: Value,
        #[case] name: Option<&str>,
        #[case] parent: Option<Parent>,
        #[case] expected: Option<(&str, Parent)>,
    ) {
        let source = NodeBrief::of(&source).unwrap_or_default();
        let copy = copy_target(&source, name, parent);
        let got = copy
            .as_ref()
            .map(|copy| (copy.name.as_str(), copy.parent.clone()));
        assert_eq!(got, expected);
    }

    #[rstest]
    #[case::default_suffix("Plans Q4", None, "Plans Q4-copy")]
    #[case::requested_trimmed("Plans Q4", Some(" My name "), "My name")]
    #[case::blank_requested("Plans Q4", Some("  "), "Plans Q4-copy")]
    #[case::nameless_source("", None, "copy")]
    fn copy_name_cases(
        #[case] source: &str,
        #[case] requested: Option<&str>,
        #[case] expected: &str,
    ) {
        assert_eq!(copy_name(source, requested), expected);
    }

    #[test]
    fn every_kind_round_trips_through_text() {
        assert!(
            NodeKind::iter()
                .filter(|kind| *kind != NodeKind::Other)
                .all(|known| NodeKind::from(known.as_str()) == known)
        );
        assert_eq!(NodeKind::from(NodeKind::Other.as_str()), NodeKind::Other);
    }

    #[rstest]
    #[case::board("board", NodeKind::Board)]
    #[case::folder("folder", NodeKind::Folder)]
    #[case::unknown("sticker", NodeKind::Other)]
    #[case::case_matters("Board", NodeKind::Other)]
    fn node_kind_is_total(#[case] raw: &str, #[case] expected: NodeKind) {
        assert_eq!(NodeKind::from(raw), expected);
    }
}
