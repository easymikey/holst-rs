//! What the fake holds when it starts.

use std::io;

use serde_json::Value;

use crate::{
    FixtureObject,
    State,
    access::{Access, Right},
    comment::{Comments, FixtureThread},
    files::{Files, FixtureBackup, FixturePage, PageOutcome, gzip},
    login::Sessions,
    templates::{FixtureTemplate, FixtureTemplateCategory, Templates},
    tree::{FixtureNode, Storage, Tree},
    workspace::{Detail, Directory, FixtureEvent, WorkspaceRole},
};

/// What the fake holds when it starts.
#[derive(Debug, Default)]
pub struct Fixture {
    workspaces: Vec<String>,
    nodes: Vec<FixtureNode>,
    categories: Vec<FixtureTemplateCategory>,
    own_templates: Vec<FixtureTemplate>,
    backups: Vec<FixtureBackup>,
    covers: Vec<(String, String)>,
    notes: Vec<(String, String)>,
    threads: Vec<(String, FixtureThread)>,
    sidebar: Vec<String>,
    pages: Vec<FixturePage>,
    own_roles: Vec<(String, Right)>,
    upload_limit: Option<usize>,
    rotating: bool,
    details: Vec<(String, Detail)>,
    events: Vec<FixtureEvent>,
    access_tokens: Vec<String>,
    other_clients: Vec<String>,
    failing: Vec<(String, u16)>,
}

impl Fixture {
    /// Adds a workspace after the ones already there; the server lists them in
    /// this order.
    #[must_use]
    pub fn workspace(self, name: &str) -> Self {
        Self {
            workspaces: self
                .workspaces
                .into_iter()
                .chain([name.to_owned()])
                .collect(),
            ..self
        }
    }

    /// Adds a workspace of the organization `organization`.
    #[must_use]
    pub fn workspace_in(self, name: &str, organization: &str) -> Self {
        self.workspace(name)
            .detail(name, Detail::Organization(organization.to_owned()))
    }

    /// Adds a workspace the user may join and does not belong to.
    #[must_use]
    pub fn joinable_workspace(self, name: &str) -> Self {
        self.workspace(name).detail(name, Detail::Joinable)
    }

    /// Makes the person called `name` a member of `workspace` with `role`.
    #[must_use]
    pub fn member(self, name: &str, workspace: &str, role: WorkspaceRole) -> Self {
        self.detail(workspace, Detail::Member(name.to_owned(), role))
    }

    /// Sets the role the signed-in user holds in `workspace`; it is the owner
    /// otherwise.
    #[must_use]
    pub fn user_role(self, workspace: &str, role: WorkspaceRole) -> Self {
        self.detail(workspace, Detail::UserIs(role))
    }

    /// Adds a verified domain to `workspace`.
    #[must_use]
    pub fn domain(self, workspace: &str, domain: &str) -> Self {
        self.detail(workspace, Detail::Domain(domain.to_owned()))
    }

    /// Turns single sign-on on in `workspace`.
    #[must_use]
    pub fn sso(self, workspace: &str) -> Self {
        self.detail(workspace, Detail::Sso)
    }

    /// Adds a request of the person called `name` to join `workspace`.
    #[must_use]
    pub fn join_request(self, workspace: &str, name: &str) -> Self {
        self.detail(workspace, Detail::JoinRequest(name.to_owned()))
    }

    /// Puts `workspace` on the plan called `plan` (Free, Team, Business or
    /// Enterprise); it is on Free otherwise.
    #[must_use]
    pub fn plan(self, workspace: &str, plan: &str) -> Self {
        self.detail(workspace, Detail::Plan(plan.to_owned()))
    }

    /// Gives `workspace` billing details naming `company`.
    #[must_use]
    pub fn billing(self, workspace: &str, company: &str) -> Self {
        self.detail(workspace, Detail::Billing(company.to_owned()))
    }

    /// Adds an event to the log; later events are newer.
    #[must_use]
    pub fn event(
        self,
        workspace: &str,
        user: &str,
        action: &str,
        target: &str,
    ) -> Self {
        Self {
            events: self
                .events
                .into_iter()
                .chain([FixtureEvent {
                    workspace: workspace.to_owned(),
                    user: user.to_owned(),
                    action: action.to_owned(),
                    target: target.to_owned(),
                }])
                .collect(),
            ..self
        }
    }

    /// Adds a personal access token of the signed-in user.
    #[must_use]
    pub fn access_token(self, name: &str) -> Self {
        Self {
            access_tokens: self
                .access_tokens
                .into_iter()
                .chain([name.to_owned()])
                .collect(),
            ..self
        }
    }

    /// Adds a session of the signed-in user that a client other than the
    /// caller holds.
    #[must_use]
    pub fn other_session(self, client: &str) -> Self {
        Self {
            other_clients: self
                .other_clients
                .into_iter()
                .chain([client.to_owned()])
                .collect(),
            ..self
        }
    }

    /// Makes the API path `path` (as `current_user`) answer with `status`.
    #[must_use]
    pub fn failing(self, path: &str, status: u16) -> Self {
        Self {
            failing: self
                .failing
                .into_iter()
                .chain([(format!("/gapi/{path}"), status)])
                .collect(),
            ..self
        }
    }

    /// Adds a folder at the root of the workspace named `workspace`.
    #[must_use]
    pub fn folder(self, name: &str, workspace: &str) -> Self {
        self.node(name, workspace, "folder", None, Vec::new(), false)
    }

    /// Adds a board to the workspace named `workspace`, in the folder named
    /// `folder` that was added before it, or at the root.
    #[must_use]
    pub fn board(self, name: &str, workspace: &str, folder: Option<&str>) -> Self {
        self.node(name, workspace, "board", folder, Vec::new(), false)
    }

    /// Adds `count` boards called "Board 1", "Board 2" and on, at the root of
    /// `workspace`.
    #[must_use]
    pub fn boards(self, count: usize, workspace: &str) -> Self {
        (1..=count).fold(self, |fixture, number| {
            fixture.board(&format!("Board {number}"), workspace, None)
        })
    }

    /// Adds a board to `workspace` on which the user holds `role`; the user
    /// may change it unless the role is a viewer's or a commentator's.
    #[must_use]
    pub fn board_as(self, name: &str, workspace: &str, role: Right) -> Self {
        Self {
            own_roles: self
                .own_roles
                .iter()
                .cloned()
                .chain([(name.to_owned(), role)])
                .collect(),
            ..self
        }
        .node(
            name,
            workspace,
            "board",
            None,
            Vec::new(),
            role.is_read_only(),
        )
    }

    /// Adds a board of the person called `owner`, whom the user was invited to
    /// join; the user may leave it.
    #[must_use]
    pub fn invited_board(self, name: &str, workspace: &str, owner: &str) -> Self {
        self.owned_by(name, workspace, owner, true)
    }

    /// Adds a board of the person called `owner` that the user reaches without
    /// an invitation, as an admin of `workspace`.
    #[must_use]
    pub fn board_owned_by(self, name: &str, workspace: &str, owner: &str) -> Self {
        self.owned_by(name, workspace, owner, false)
    }

    /// Adds a board with `objects` on it to the workspace of that name.
    #[must_use]
    pub fn board_with_objects(
        self,
        name: &str,
        workspace: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        self.node(name, workspace, "board", None, objects, false)
    }

    /// Adds a board stored in `schema_version` of the board format on the
    /// server `provider` ("hud" or "hocuspocus"); the program reads only
    /// version 2 on hud.
    #[must_use]
    pub fn board_stored_as(
        self,
        name: &str,
        workspace: &str,
        schema_version: u32,
        provider: &'static str,
    ) -> Self {
        self.pushed(FixtureNode {
            workspace: workspace.to_owned(),
            name: name.to_owned(),
            kind: "board",
            folder: None,
            objects: Vec::new(),
            read_only: false,
            owner: None,
            invited: false,
            storage: Storage {
                schema_version,
                provider,
            },
        })
    }

    /// Adds a board the user may read and not change.
    #[must_use]
    pub fn read_only_board_with_objects(
        self,
        name: &str,
        workspace: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        self.node(name, workspace, "board", None, objects, true)
    }

    /// Adds a category of ready-made templates with these templates in it.
    #[must_use]
    pub fn template_category(self, name: &str, slug: &str, templates: &[&str]) -> Self {
        Self {
            categories: self
                .categories
                .into_iter()
                .chain([FixtureTemplateCategory {
                    name: name.to_owned(),
                    slug: slug.to_owned(),
                    templates: templates
                        .iter()
                        .map(|template| (*template).to_owned())
                        .collect(),
                }])
                .collect(),
            ..self
        }
    }

    /// Adds a template of the signed-in user's own.
    #[must_use]
    pub fn personal_template(self, name: &str) -> Self {
        self.template(name, None, Vec::new())
    }

    /// Adds a template of the signed-in user's own with `objects` in it.
    #[must_use]
    pub fn template_with_objects(
        self,
        name: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        self.template(name, None, objects)
    }

    /// Adds a template that every member of `workspace` may use.
    #[must_use]
    pub fn workspace_template(self, name: &str, workspace: &str) -> Self {
        self.template(name, Some(workspace), Vec::new())
    }

    /// Adds a snapshot of the board called `board`; downloading `key` gives
    /// `content`. It is dated 2026-10-01 and held by the `hud` provider.
    #[must_use]
    pub fn backup(self, board: &str, key: &str, content: &str) -> Self {
        self.backup_made(board, key, content, ("2026-10-01T00:00:00Z", "hud"))
    }

    /// Adds a snapshot taken at `date` (ISO 8601) by `provider`; the list shows
    /// the newest first, and a download names the provider too.
    #[must_use]
    pub fn backup_made(
        self,
        board: &str,
        key: &str,
        content: &str,
        (date, provider): (&str, &str),
    ) -> Self {
        self.snapshot(board, key, content.as_bytes().to_vec(), (date, provider))
    }

    /// Adds a snapshot like [`Fixture::backup_made`] whose body the server
    /// sends gzip-compressed.
    #[must_use]
    pub fn gzipped_backup(
        self,
        board: &str,
        key: &str,
        content: &str,
        (date, provider): (&str, &str),
    ) -> Self {
        self.snapshot(board, key, gzip(content.as_bytes()), (date, provider))
    }

    /// Adds a snapshot of the board called `board` that holds `objects`, as a
    /// restore needs; it is dated 2026-10-01 and held by the `hud` provider.
    #[must_use]
    pub fn backup_of_objects(
        self,
        board: &str,
        key: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        Self {
            backups: self
                .backups
                .into_iter()
                .chain([FixtureBackup {
                    board: board.to_owned(),
                    key: key.to_owned(),
                    date: "2026-10-01T00:00:00Z".to_owned(),
                    provider: "hud".to_owned(),
                    content: Vec::new(),
                    objects: Some(objects),
                }])
                .collect(),
            ..self
        }
    }

    /// Gives the board called `board` a cover: the file stored as `thumbnail`
    /// fixed on it.
    #[must_use]
    pub fn cover(self, board: &str, thumbnail: &str) -> Self {
        Self {
            covers: self
                .covers
                .into_iter()
                .chain([(board.to_owned(), thumbnail.to_owned())])
                .collect(),
            ..self
        }
    }

    /// Gives the board called `board` a note holding `text`.
    #[must_use]
    pub fn note(self, board: &str, text: &str) -> Self {
        Self {
            notes: self
                .notes
                .into_iter()
                .chain([(board.to_owned(), text.to_owned())])
                .collect(),
            ..self
        }
    }

    /// Adds `thread` to the board called `board`, after the threads already
    /// there.
    #[must_use]
    pub fn thread(self, board: &str, thread: FixtureThread) -> Self {
        Self {
            threads: self
                .threads
                .into_iter()
                .chain([(board.to_owned(), thread)])
                .collect(),
            ..self
        }
    }

    /// Puts the boards called `boards` first in the sidebar order of the user,
    /// in this order.
    #[must_use]
    pub fn sidebar_order(self, boards: &[&str]) -> Self {
        Self {
            sidebar: boards.iter().map(|name| (*name).to_owned()).collect(),
            ..self
        }
    }

    /// Makes the preview of the page at `url` carry `title`.
    #[must_use]
    pub fn link_preview(self, url: &str, title: &str) -> Self {
        self.page(url, PageOutcome::Title(title.to_owned()))
    }

    /// Makes the preview of the page at `url` fail.
    #[must_use]
    pub fn failing_link_preview(self, url: &str) -> Self {
        self.page(url, PageOutcome::Fails)
    }

    /// Makes the server refuse an uploaded file of more than `bytes` bytes.
    #[must_use]
    pub fn upload_limit(self, bytes: usize) -> Self {
        Self {
            upload_limit: Some(bytes),
            ..self
        }
    }

    /// Makes every signed-in reply hand out a fresh session cookie and end the
    /// one it was sent with, as the live server may on any request.
    #[must_use]
    pub fn rotating_cookie(self) -> Self {
        Self {
            rotating: true,
            ..self
        }
    }

    fn template(
        self,
        name: &str,
        workspace: Option<&str>,
        objects: Vec<FixtureObject>,
    ) -> Self {
        Self {
            own_templates: self
                .own_templates
                .into_iter()
                .chain([FixtureTemplate {
                    name: name.to_owned(),
                    workspace: workspace.map(str::to_owned),
                    objects,
                }])
                .collect(),
            ..self
        }
    }

    fn snapshot(
        self,
        board: &str,
        key: &str,
        content: Vec<u8>,
        (date, provider): (&str, &str),
    ) -> Self {
        Self {
            backups: self
                .backups
                .into_iter()
                .chain([FixtureBackup {
                    board: board.to_owned(),
                    key: key.to_owned(),
                    date: date.to_owned(),
                    provider: provider.to_owned(),
                    content,
                    objects: None,
                }])
                .collect(),
            ..self
        }
    }

    fn page(self, url: &str, outcome: PageOutcome) -> Self {
        Self {
            pages: self
                .pages
                .into_iter()
                .chain([FixturePage {
                    url: url.to_owned(),
                    outcome,
                }])
                .collect(),
            ..self
        }
    }

    fn detail(self, workspace: &str, detail: Detail) -> Self {
        Self {
            details: self
                .details
                .into_iter()
                .chain([(workspace.to_owned(), detail)])
                .collect(),
            ..self
        }
    }

    fn node(
        self,
        name: &str,
        workspace: &str,
        kind: &'static str,
        folder: Option<&str>,
        objects: Vec<FixtureObject>,
        read_only: bool,
    ) -> Self {
        self.pushed(FixtureNode {
            workspace: workspace.to_owned(),
            name: name.to_owned(),
            kind,
            folder: folder.map(str::to_owned),
            objects,
            read_only,
            owner: None,
            invited: false,
            storage: Storage::default(),
        })
    }

    /// Adds a board of `owner`, who becomes a member of `workspace` when not
    /// one already.
    fn owned_by(self, name: &str, workspace: &str, owner: &str, invited: bool) -> Self {
        let known = self.details.iter().any(|(known, detail)| {
            known == workspace
                && matches!(detail, Detail::Member(who, _) if who == owner)
        });
        let fixture = if known {
            self
        } else {
            self.member(owner, workspace, WorkspaceRole::Member)
        };
        fixture.pushed(FixtureNode {
            workspace: workspace.to_owned(),
            name: name.to_owned(),
            kind: "board",
            folder: None,
            objects: Vec::new(),
            read_only: false,
            owner: Some(owner.to_owned()),
            invited,
            storage: Storage::default(),
        })
    }

    fn pushed(self, node: FixtureNode) -> Self {
        Self {
            nodes: self.nodes.into_iter().chain([node]).collect(),
            ..self
        }
    }
}

impl State {
    pub(crate) fn fixture(fixture: Fixture) -> Result<Self, FixtureError> {
        let workspaces =
            Directory::fixture(&fixture.workspaces, &fixture.details, &fixture.events)?;
        let named = workspaces.named();
        let user_id = workspaces
            .current_user()
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let people = workspaces.people();
        let mut tree = Tree::fixture(&fixture.nodes, &named, &user_id, &people)?;
        tree.settle(&fixture.covers, &fixture.notes, &fixture.sidebar)?;
        Ok(Self {
            catalog: Templates::fixture(
                &fixture.categories,
                &fixture.own_templates,
                &named,
            )?,
            access: Access::fixture(&fixture.own_roles, &user_id, |name| {
                tree.id_of(name)
            })?,
            comments: Comments::fixture(&fixture.threads, &user_id, |name| {
                tree.id_of(name)
            })?,
            files: Files::fixture(
                &fixture.backups,
                (&fixture.pages, fixture.upload_limit),
                |name| tree.id_of(name),
            )?,
            tree,
            workspaces,
            sessions: Sessions::fixture(
                fixture.rotating,
                fixture.access_tokens,
                fixture.other_clients,
            ),
            failing: fixture.failing,
            unanswered: Vec::new(),
            updates: 0,
        })
    }
}

/// A fixture that names something it does not hold.
#[derive(Debug, thiserror::Error)]
#[error("the fixture names the {what} {name:?}, which it does not hold")]
pub struct FixtureError {
    what: &'static str,
    name: String,
}

impl From<FixtureError> for io::Error {
    fn from(error: FixtureError) -> Self {
        Self::other(error)
    }
}

pub(crate) fn unknown(what: &'static str, name: &str) -> FixtureError {
    FixtureError {
        what,
        name: name.to_owned(),
    }
}
