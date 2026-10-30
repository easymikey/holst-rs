use holst_macros::{get, patch, post};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::node::{
        CreationStamp,
        DeleteMode,
        NewNode,
        NewParent,
        NodeCopy,
        NodeKind,
        Parent,
    },
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

/// `PATCH channel/{id}` renaming; the web client adds two nulls to every patch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    pub in_trash: (),
    pub name: String,
    pub thumbnail_updated_at: (),
}

impl Rename {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }
}

/// `PATCH channel/{id}` fixing an uploaded picture as the cover.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetCover {
    pub in_trash: (),
    pub thumbnail: String,
    pub thumbnail_color: String,
    pub thumbnail_fixed: bool,
    pub thumbnail_updated_at: (),
}

impl SetCover {
    #[must_use]
    pub fn new(thumbnail: &str) -> Self {
        Self {
            thumbnail: thumbnail.to_owned(),
            thumbnail_color: "#ffffff".to_owned(),
            thumbnail_fixed: true,
            ..Self::default()
        }
    }
}

/// `PATCH channel/{id}` giving up the chosen cover (`Default` is the body).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetCover {
    pub in_trash: (),
    pub thumbnail_color: (),
    pub thumbnail_fixed: bool,
    pub thumbnail_updated_at: (),
}

/// `PATCH channel/{id}` storing the automatic preview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPreview {
    pub in_trash: (),
    pub thumbnail: String,
    pub thumbnail_color: (),
    pub thumbnail_updated_at: (),
}

impl SetPreview {
    #[must_use]
    pub fn new(thumbnail: &str) -> Self {
        Self {
            thumbnail: thumbnail.to_owned(),
            ..Self::default()
        }
    }
}

/// `channel/{id}/delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delete {
    pub permanently: bool,
}

impl From<DeleteMode> for Delete {
    fn from(mode: DeleteMode) -> Self {
        Self {
            permanently: mode.is_permanent(),
        }
    }
}

/// `channel/restore/{id}`: an empty object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "a unit struct would be sent as null, not {}"
)]
pub struct Restore {}

/// `channel/{id}/move`: into a folder or the root (`null`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoveTo {
    pub new_index: f64,
    pub new_parent_id: Option<String>,
}

impl MoveTo {
    #[must_use]
    pub fn new(parent: &Parent, new_index: f64) -> Self {
        Self {
            new_index,
            new_parent_id: parent.new_parent_id().map(str::to_owned),
        }
    }
}

/// `channel/create`; fields in key order, as the wire has them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateChannel {
    pub author_id: String,
    pub created_at: String,
    pub id: String,
    pub in_trash: (),
    pub name: String,
    pub order_index: f64,
    pub parent_channel_access: Option<String>,
    pub parent_id: String,
    pub parent_type: String,
    pub permits: Vec<String>,
    pub public: bool,
    pub public_access: (),
    pub schema_version: u32,
    pub thumbnail: (),
    pub thumbnail_color: (),
    pub thumbnail_fixed: bool,
    pub thumbnail_updated_at: (),
    #[serde(rename = "type")]
    pub kind: String,
    pub update_author_id: String,
    pub updated_at: String,
    pub workspace_id: String,
    pub workspace_member_access: Option<String>,
}

impl CreateChannel {
    /// The body of `channel/create` by `author_id`, its id and time from
    /// `stamp`. In a folder the channel inherits the folder's access; at the
    /// root the workspace members get `root_access`, none for private, as the
    /// web client sends.
    ///
    /// # Errors
    ///
    /// The error of [`NodeKind::creatable`] when the kind is neither board nor
    /// folder.
    pub fn new(
        new: &NewNode,
        author_id: &str,
        stamp: &CreationStamp,
        root_access: Option<&str>,
    ) -> Result<Self> {
        let kind = NodeKind::creatable(new.kind.as_str())?;
        let parent = new.parent.parent_in(&new.workspace);
        let now = stamp.at.iso8601();
        let (member_access, parent_access) = match new.parent {
            NewParent::Root => (root_access.map(str::to_owned), None),
            NewParent::Folder(_) => (None, Some("inherit".to_owned())),
        };
        Ok(Self {
            author_id: author_id.to_owned(),
            created_at: now.clone(),
            id: stamp.id.clone(),
            in_trash: (),
            name: new.name.as_str().to_owned(),
            order_index: stamp.at.sidebar_order(),
            parent_channel_access: parent_access,
            parent_id: parent.id().to_owned(),
            parent_type: parent.api_type().to_owned(),
            permits: vec!["read".to_owned(), "read_comments".to_owned()],
            public: false,
            public_access: (),
            schema_version: 2,
            thumbnail: (),
            thumbnail_color: (),
            thumbnail_fixed: false,
            thumbnail_updated_at: (),
            kind: kind.as_str().to_owned(),
            update_author_id: author_id.to_owned(),
            updated_at: now,
            workspace_id: new.workspace.as_str().to_owned(),
            workspace_member_access: member_access,
        })
    }
}

/// `channel/duplicate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Duplicate {
    pub channel_id: String,
    pub duplicate_permissions: bool,
    pub name: String,
    pub order_index: f64,
    pub parent_id: String,
    pub parent_type: String,
    pub source_channel_id: String,
}

impl Duplicate {
    /// The copy of `duplication`; its id and sidebar place come from `stamp`.
    #[must_use]
    pub fn new(duplication: &NodeCopy<'_>, stamp: &CreationStamp) -> Self {
        let parent = &duplication.target.parent;
        Self {
            channel_id: stamp.id.clone(),
            duplicate_permissions: duplication.permissions.is_copied(),
            name: duplication.target.name.clone(),
            order_index: stamp.at.sidebar_order(),
            parent_id: parent.id().to_owned(),
            parent_type: parent.api_type().to_owned(),
            source_channel_id: duplication.source_id.to_owned(),
        }
    }
}

/// A board's metadata and permissions.
#[get("channel/{channel_id}", class = Read)]
pub async fn get(client: &Client, channel_id: &str) -> Result<Value>;

/// Who has access.
#[get("channel_user/users", class = Read, query(channel_id = channel_id))]
pub async fn users(client: &Client, channel_id: &str) -> Result<Value>;

/// Permissions inherited from the folder.
#[get("channel/{channel_id}/parent_total_permits", class = Read)]
pub async fn parent_total_permits(client: &Client, channel_id: &str) -> Result<Value>;

/// The order in the sidebar.
#[get("channel_user_order_index/{workspace_id}/list", class = Read)]
pub async fn order_index_list(client: &Client, workspace_id: &str) -> Result<Value>;

/// Rename a board or a folder.
#[patch(
    "channel/{channel_id}",
    class = Admin,
    request_only
)]
pub async fn rename(
    client: &Client,
    channel_id: &str,
    #[body] body: &Rename,
) -> Result<Value>;

/// Set the cover to an uploaded picture: the user's choice, not the preview.
#[patch(
    "channel/{channel_id}",
    class = Admin
)]
pub async fn set_cover(
    client: &Client,
    channel_id: &str,
    #[body] body: &SetCover,
) -> Result<Value>;

/// Give up the chosen cover: the board shows its automatic preview again.
#[patch(
    "channel/{channel_id}",
    class = Admin
)]
pub async fn reset_cover(
    client: &Client,
    channel_id: &str,
    #[body] body: &ResetCover,
) -> Result<Value>;

/// Store a freshly drawn automatic preview after a reset.
#[patch(
    "channel/{channel_id}",
    class = Admin
)]
pub async fn set_preview(
    client: &Client,
    channel_id: &str,
    #[body] body: &SetPreview,
) -> Result<Value>;

/// Delete: to the trash, or for good.
#[post(
    "channel/{channel_id}/delete",
    class = Write,
    request_only
)]
pub async fn delete(
    client: &Client,
    channel_id: &str,
    #[body] body: &Delete,
) -> Result<Value>;

/// Bring back from the trash. Not verified live.
#[post("channel/restore/{channel_id}", class = Write, request_only)]
pub async fn restore(
    client: &Client,
    channel_id: &str,
    #[body] body: &Restore,
) -> Result<Value>;

/// Move into a folder or to the root, to the sidebar place `new_index`.
#[post(
        "channel/{channel_id}/move",
        class = Write,
        request_only
    )]
pub async fn move_to(
    client: &Client,
    channel_id: &str,
    #[body] body: &MoveTo,
) -> Result<Value>;

/// Leave a board you were invited to. Not verified live.
#[post("channel/{channel_id}/leave", class = Write, request_only)]
pub async fn leave(client: &Client, channel_id: &str) -> Result<Value>;

/// Create a board or folder.
#[post("channel/create", class = Write, request_only)]
pub async fn create(client: &Client, #[body] body: &CreateChannel) -> Result<Value>;

/// A copy of a board.
#[post("channel/duplicate", class = Write, request_only)]
pub async fn duplicate(client: &Client, #[body] body: &Duplicate) -> Result<Value>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[
    GET,
    USERS,
    PARENT_TOTAL_PERMITS,
    ORDER_INDEX_LIST,
    RENAME,
    SET_COVER,
    RESET_COVER,
    SET_PREVIEW,
    DELETE,
    RESTORE,
    MOVE_TO,
    LEAVE,
    CREATE,
    DUPLICATE,
];
