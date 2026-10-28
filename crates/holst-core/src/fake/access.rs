//! Who can see and edit a board: the link settings and the roles.

use serde_json::{Map, Value, json};

use crate::fake::{
    FixtureError,
    fixture::unknown,
    http::{Parts, Request, Response, found, object_body, refused},
};

/// The right a person holds on a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Right {
    Viewer,
    Commentator,
    Editor,
    Full,
}

impl Right {
    const fn name(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Commentator => "commentator",
            Self::Editor => "editor",
            Self::Full => "owner",
        }
    }

    /// Tells whether the role may read and not change the board.
    pub(crate) const fn is_read_only(self) -> bool {
        matches!(self, Self::Viewer | Self::Commentator)
    }
}

/// The sharing fields of a board nobody changed the access of.
fn default_sharing() -> Map<String, Value> {
    [
        ("public", json!(false)),
        ("public_access", Value::Null),
        ("workspace_member_access", json!("co_owner")),
        ("who_can_copy_content", json!("viewer")),
        ("who_can_delete_content", json!("editor")),
        ("who_can_manage_collaboration_tools", json!("editor")),
        ("who_can_manage_lock_content", json!("editor")),
        ("who_can_manage_frame_hide_content", json!("editor")),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

/// The keys of a channel patch that tell who sees the board; the web client
/// does not send them to `sharing_settings`.
fn is_access_key(key: &str) -> bool {
    matches!(
        key,
        "public" | "public_access" | "workspace_member_access" | "share_as_template"
    )
}

#[derive(Debug, Default)]
pub(crate) struct Access {
    settings: Vec<(String, Map<String, Value>)>,
    /// Board id, person id and role.
    roles: Vec<(String, String, String)>,
    /// The id of the signed-in user.
    user: String,
}

/// The permits the web client gives a user it sets to `role`.
fn permits_of(role: &str) -> &'static [&'static str] {
    match role {
        "co_owner" => &[
            "read",
            "write",
            "delete",
            "set_permits",
            "create_channel",
            "read_comments",
            "write_comments",
        ],
        "editor" => &[
            "read",
            "write",
            "create_channel",
            "read_comments",
            "write_comments",
        ],
        "commentator" => &["read", "read_comments", "write_comments"],
        "viewer" => &["read", "read_comments"],
        _ => &[],
    }
}

impl Access {
    /// Fills the roles the user holds on the boards called as `own` says;
    /// `board_id` maps a board name to its id.
    ///
    /// # Errors
    /// [`FixtureError`] when a role names a board the fixture does not hold.
    pub(crate) fn fixture(
        own: &[(String, Right)],
        user: &str,
        board_id: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, FixtureError> {
        Ok(Self {
            settings: Vec::new(),
            roles: own
                .iter()
                .map(|(board, role)| {
                    board_id(board)
                        .map(|id| (id, user.to_owned(), role.name().to_owned()))
                        .ok_or_else(|| unknown("board", board))
                })
                .collect::<Result<_, FixtureError>>()?,
            user: user.to_owned(),
        })
    }

    /// Tells whether the user may change who has access to the board `board`:
    /// only an owner or co-owner may, and so may a user who holds no filled
    /// role.
    fn may_share(&self, board: &str) -> bool {
        self.role_of(board, &self.user)
            .is_none_or(|role| role == Right::Full.name() || role == "co_owner")
    }

    /// The link settings a board holds, by its id.
    pub(crate) fn settings_of(&self, board: &str) -> Map<String, Value> {
        self.settings
            .iter()
            .find(|(id, _)| id == board)
            .map(|(_, settings)| settings.clone())
            .unwrap_or_default()
    }

    /// Keeps copying out of `board` to the role `needs` and up, and gives the
    /// user the role `role` there.
    pub(crate) fn withhold_copying(&mut self, board: &str, needs: &str, role: &str) {
        self.withhold(board, "who_can_copy_content", needs, role);
    }

    /// Keeps removing other people's objects on `board` to the role `needs`
    /// and up, and gives the user the role `role` there.
    pub(crate) fn withhold_deleting(&mut self, board: &str, needs: &str, role: &str) {
        self.withhold(board, "who_can_delete_content", needs, role);
    }

    /// Keeps locking and unlocking on `board` to the role `needs` and up, and
    /// gives the user the role `role` there.
    pub(crate) fn withhold_locking(&mut self, board: &str, needs: &str, role: &str) {
        self.withhold(board, "who_can_manage_lock_content", needs, role);
    }

    fn withhold(&mut self, board: &str, setting: &str, needs: &str, role: &str) {
        self.patch(board, Map::from_iter([(setting.to_owned(), json!(needs))]));
        let user = self.user.clone();
        self.roles
            .retain(|(id, who, _)| !(id == board && *who == user));
        self.roles.push((board.to_owned(), user, role.to_owned()));
    }

    /// The role `user` holds on the board `board`.
    pub(crate) fn role_of(&self, board: &str, user: &str) -> Option<String> {
        self.roles
            .iter()
            .find(|(id, who, _)| id == board && who == user)
            .map(|(_, _, role)| role.clone())
    }

    /// Adds the link settings to the reply that shows a board, as the server's
    /// channel reply carries its access fields; a copy of a board made with
    /// `duplicate_permissions` gets the roles of the original, and a channel
    /// made to inherit gets those of its folder.
    pub(crate) fn shown_on(&mut self, request: &Request, reply: Response) -> Response {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("POST", ["", "gapi", "channel", "duplicate"]) => {
                self.copy_roles(request, &reply);
                reply
            }
            ("POST", ["", "gapi", "channel", "create"]) => {
                self.inherit_roles(request, &reply);
                reply
            }
            ("PATCH", ["", "gapi", "channel", id]) => {
                self.keep_access_keys(id, request, reply)
            }
            ("GET", ["", "gapi", "channel", id]) => {
                serde_json::from_str::<Map<String, Value>>(&reply.body)
                    .ok()
                    .filter(|_| reply.status == 200)
                    .map(|mut board| {
                        board.extend(default_sharing());
                        board.extend(self.settings_of(id));
                        let role = self.role_of(id, &self.user);
                        // No filled role is the top one, as in `may_share`.
                        let permits = match role.as_deref() {
                            Some("owner") | None => "co_owner",
                            Some(other) => other,
                        };
                        board.insert("permits".to_owned(), json!(permits_of(permits)));
                        if let Some(role) = role {
                            board.insert(
                                "channel_user".to_owned(),
                                json!({ "user_id": self.user, "role": role }),
                            );
                        }
                        found(&Value::Object(board))
                    })
                    .unwrap_or(reply)
            }
            _ => reply,
        }
    }

    /// Answers the access endpoints of the boards `exists` knows.
    pub(crate) fn respond(
        &mut self,
        request: &Request,
        exists: impl Fn(&str) -> bool,
    ) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("PATCH", ["", "gapi", "channel", id, "sharing_settings"]) => {
                Some(match (exists(id), object_body(request)) {
                    (true, _) if !self.may_share(id) => refused(403, "forbidden"),
                    // The web client sends these to the channel itself.
                    (true, Some(patch))
                        if patch.keys().any(|key| is_access_key(key)) =>
                    {
                        refused(400, "bad-request")
                    }
                    (true, Some(patch)) if !patch.is_empty() => {
                        self.patch(id, patch);
                        found(&json!({}))
                    }
                    (false, _) => refused(404, "not-found"),
                    (true, _) => refused(400, "bad-request"),
                })
            }
            ("POST", ["", "gapi", "channel_user", "set"]) => {
                Some(self.set_role(request, exists))
            }
            // Nobody holds a right through a folder.
            ("GET", ["", "gapi", "channel", id, "parent_total_permits"]) => {
                Some(if exists(id) {
                    found(&json!({ "permits": [] }))
                } else {
                    refused(404, "not-found")
                })
            }
            ("GET", ["", "gapi", "channel_user", "users"]) => Some(
                parts
                    .query("channel_id")
                    .filter(|id| exists(id))
                    .map_or_else(
                        || refused(404, "not-found"),
                        |board| found(&json!({ "result": self.holders(board) })),
                    ),
            ),
            _ => None,
        }
    }

    /// Who holds a role on the board, in the order they got it.
    fn holders(&self, board: &str) -> Vec<Value> {
        self.roles
            .iter()
            .filter(|(id, _, _)| id == board)
            .map(|(id, user, role)| {
                json!({
                    "channel_id": id,
                    "user_id": user,
                    "permits": permits_of(role),
                    "created_at": "2026-01-01T00:00:00Z",
                })
            })
            .collect()
    }

    /// A channel patch with access keys changes the board's link and member
    /// access, for whoever may share it; the other keys are the tree's.
    fn keep_access_keys(
        &mut self,
        id: &str,
        request: &Request,
        reply: Response,
    ) -> Response {
        let access: Map<String, Value> = object_body(request)
            .into_iter()
            .flatten()
            .filter(|(key, _)| is_access_key(key))
            .collect();
        match access {
            access if reply.status != 200 || access.is_empty() => reply,
            access if self.may_share(id) => {
                self.patch(id, access);
                reply
            }
            _ => refused(403, "forbidden"),
        }
    }

    fn copy_roles(&mut self, request: &Request, reply: &Response) {
        let asked = object_body(request).filter(|body| {
            body.get("duplicate_permissions").and_then(Value::as_bool) == Some(true)
        });
        let ids = asked.and_then(|body| {
            let source = body.get("source_channel_id")?.as_str()?.to_owned();
            let made: Value = serde_json::from_str(&reply.body).ok()?;
            let copy = made.pointer("/result/id")?.as_str()?.to_owned();
            (reply.status == 200).then_some((source, copy))
        });
        if let Some((source, copy)) = ids {
            self.carry_roles(&source, &copy);
        }
    }

    /// A channel made with `parent_channel_access: "inherit"` gets the roles
    /// of its folder.
    fn inherit_roles(&mut self, request: &Request, reply: &Response) {
        let ids = object_body(request)
            .filter(|body| {
                body.get("parent_channel_access").and_then(Value::as_str)
                    == Some("inherit")
            })
            .and_then(|body| {
                let folder = body.get("parent_id")?.as_str()?.to_owned();
                let made = body.get("id")?.as_str()?.to_owned();
                (reply.status == 200).then_some((folder, made))
            });
        if let Some((folder, made)) = ids {
            self.carry_roles(&folder, &made);
        }
    }

    fn carry_roles(&mut self, source: &str, copy: &str) {
        let carried: Vec<_> = self
            .roles
            .iter()
            .filter(|(id, _, _)| id == source)
            .map(|(_, user, role)| (copy.to_owned(), user.clone(), role.clone()))
            .collect();
        self.roles.extend(carried);
    }

    fn patch(&mut self, board: &str, patch: Map<String, Value>) {
        let mut merged = self.settings_of(board);
        // `in_trash` and `thumbnail_updated_at` ride along and are no link settings.
        merged.extend(patch.into_iter().filter(|(key, _)| {
            !matches!(key.as_str(), "in_trash" | "thumbnail_updated_at")
        }));
        self.settings.retain(|(id, _)| id != board);
        self.settings.push((board.to_owned(), merged));
    }

    fn set_role(
        &mut self,
        request: &Request,
        exists: impl Fn(&str) -> bool,
    ) -> Response {
        let text = |body: &Map<String, Value>, key: &str| {
            body.get(key).and_then(Value::as_str).map(str::to_owned)
        };
        match object_body(request) {
            Some(body) => match (text(&body, "channel_id"), text(&body, "user_id")) {
                (Some(board), Some(_)) if exists(&board) && !self.may_share(&board) => {
                    refused(403, "forbidden")
                }
                (Some(board), Some(user)) if exists(&board) => {
                    self.roles
                        .retain(|(id, who, _)| !(*id == board && *who == user));
                    self.roles.extend(
                        text(&body, "role")
                            .map(|role| (board.clone(), user.clone(), role)),
                    );
                    found(&json!({}))
                }
                (Some(_), Some(_)) => refused(404, "not-found"),
                _ => refused(400, "bad-request"),
            },
            None => refused(400, "bad-request"),
        }
    }
}
