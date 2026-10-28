//! The state a scenario starts from.

use holst_core::fake::{
    ACCEPTED_COOKIES,
    ACCEPTED_TOKEN,
    Fake,
    Fixture,
    FixtureObject,
    FixtureThread,
    Right,
    WorkspaceRole,
};
use serde_json::json;

use crate::{directory::TestDirectory, error::ScenarioError, holst::Holst};

#[must_use]
pub fn given() -> Given {
    Given::default()
}

#[derive(Debug, Default)]
pub struct Given {
    login: SavedLogin,
    fixture: Fixture,
    /// Files to write, by path from the root of the run's directory.
    local_files: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Default, Clone, Copy)]
enum SavedLogin {
    #[default]
    Absent,
    Active,
    Revoked,
}

impl SavedLogin {
    fn session_file(self) -> Option<String> {
        let saved = |token: &str, cookies: &str| {
            json!({ "token": token, "cookies": cookies }).to_string()
        };
        match self {
            SavedLogin::Absent => None,
            SavedLogin::Active => Some(saved(ACCEPTED_TOKEN, ACCEPTED_COOKIES)),
            SavedLogin::Revoked => Some(saved("csrf-revoked", "sessionToken=revoked")),
        }
    }
}

impl Given {
    /// Saves a login the server accepts.
    #[must_use]
    pub fn saved_login(self) -> Self {
        Self {
            login: SavedLogin::Active,
            ..self
        }
    }

    /// Saves a login the server answers with 401.
    #[must_use]
    pub fn rejected_login(self) -> Self {
        Self {
            login: SavedLogin::Revoked,
            ..self
        }
    }

    #[must_use]
    pub fn workspace(self, name: &str) -> Self {
        Self {
            fixture: self.fixture.workspace(name),
            ..self
        }
    }

    #[must_use]
    pub fn workspace_in(self, name: &str, organization: &str) -> Self {
        Self {
            fixture: self.fixture.workspace_in(name, organization),
            ..self
        }
    }

    /// Adds a workspace the user may join and does not belong to.
    #[must_use]
    pub fn joinable_workspace(self, name: &str) -> Self {
        Self {
            fixture: self.fixture.joinable_workspace(name),
            ..self
        }
    }

    #[must_use]
    pub fn member(self, name: &str, workspace: &str, role: WorkspaceRole) -> Self {
        Self {
            fixture: self.fixture.member(name, workspace, role),
            ..self
        }
    }

    /// Makes the signed-in user a plain member of `workspace`, not its owner.
    #[must_use]
    pub fn as_workspace_member(self, workspace: &str) -> Self {
        Self {
            fixture: self.fixture.user_role(workspace, WorkspaceRole::Member),
            ..self
        }
    }

    #[must_use]
    pub fn domain(self, workspace: &str, domain: &str) -> Self {
        Self {
            fixture: self.fixture.domain(workspace, domain),
            ..self
        }
    }

    #[must_use]
    pub fn sso(self, workspace: &str) -> Self {
        Self {
            fixture: self.fixture.sso(workspace),
            ..self
        }
    }

    /// Puts `workspace` on the plan called `plan`.
    #[must_use]
    pub fn plan(self, workspace: &str, plan: &str) -> Self {
        Self {
            fixture: self.fixture.plan(workspace, plan),
            ..self
        }
    }

    /// Adds an event to the log of `workspace`; later events are newer.
    #[must_use]
    pub fn event(
        self,
        workspace: &str,
        user: &str,
        action: &str,
        target: &str,
    ) -> Self {
        Self {
            fixture: self.fixture.event(workspace, user, action, target),
            ..self
        }
    }

    /// Adds a personal access token of the signed-in user.
    #[must_use]
    pub fn access_token(self, name: &str) -> Self {
        Self {
            fixture: self.fixture.access_token(name),
            ..self
        }
    }

    #[must_use]
    pub fn folder(self, name: &str, workspace: &str) -> Self {
        Self {
            fixture: self.fixture.folder(name, workspace),
            ..self
        }
    }

    /// Adds a board to `workspace`, in the folder added before it, or at the
    /// root.
    #[must_use]
    pub fn board(self, name: &str, workspace: &str, folder: Option<&str>) -> Self {
        Self {
            fixture: self.fixture.board(name, workspace, folder),
            ..self
        }
    }

    /// Adds `count` boards called "Board 1", "Board 2" and on, at the root of
    /// `workspace`.
    #[must_use]
    pub fn boards(self, count: usize, workspace: &str) -> Self {
        Self {
            fixture: self.fixture.boards(count, workspace),
            ..self
        }
    }

    /// Adds a board of the person called `owner` that the user was invited
    /// to; `owner` becomes a member of `workspace` if not one yet.
    #[must_use]
    pub fn invited_board(self, name: &str, workspace: &str, owner: &str) -> Self {
        Self {
            fixture: self.fixture.invited_board(name, workspace, owner),
            ..self
        }
    }

    /// Adds a board to `workspace` on which the user holds `role`.
    #[must_use]
    pub fn board_as(self, name: &str, workspace: &str, role: Right) -> Self {
        Self {
            fixture: self.fixture.board_as(name, workspace, role),
            ..self
        }
    }

    /// Puts the boards called `boards` first in the sidebar order of the
    /// user, in this order.
    #[must_use]
    pub fn sidebar_order(self, boards: &[&str]) -> Self {
        Self {
            fixture: self.fixture.sidebar_order(boards),
            ..self
        }
    }

    /// Adds a category of ready-made templates with these templates in it.
    #[must_use]
    pub fn template_category(self, name: &str, slug: &str, templates: &[&str]) -> Self {
        Self {
            fixture: self.fixture.template_category(name, slug, templates),
            ..self
        }
    }

    /// Adds a template of the signed-in user's own.
    #[must_use]
    pub fn personal_template(self, name: &str) -> Self {
        Self {
            fixture: self.fixture.personal_template(name),
            ..self
        }
    }

    /// Adds a template of the signed-in user's own with `objects` in it.
    #[must_use]
    pub fn template_with_objects(
        self,
        name: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        Self {
            fixture: self.fixture.template_with_objects(name, objects),
            ..self
        }
    }

    /// Adds a template that every member of `workspace` may use.
    #[must_use]
    pub fn workspace_template(self, name: &str, workspace: &str) -> Self {
        Self {
            fixture: self.fixture.workspace_template(name, workspace),
            ..self
        }
    }

    /// Adds a snapshot of a board; downloading `key` gives `content`.
    #[must_use]
    pub fn backup(self, board: &str, key: &str, content: &str) -> Self {
        Self {
            fixture: self.fixture.backup(board, key, content),
            ..self
        }
    }

    /// Adds a snapshot taken at `date` (ISO 8601) by `provider`; the server
    /// lists the newest first.
    #[must_use]
    pub fn backup_made(
        self,
        board: &str,
        key: &str,
        content: &str,
        made: (&str, &str),
    ) -> Self {
        Self {
            fixture: self.fixture.backup_made(board, key, content, made),
            ..self
        }
    }

    /// Adds a snapshot like [`Given::backup_made`] that the server sends
    /// gzip-compressed.
    #[must_use]
    pub fn gzipped_backup(
        self,
        board: &str,
        key: &str,
        content: &str,
        made: (&str, &str),
    ) -> Self {
        Self {
            fixture: self.fixture.gzipped_backup(board, key, content, made),
            ..self
        }
    }

    /// Adds a snapshot of a board that holds `objects`, which a restore can
    /// write back.
    #[must_use]
    pub fn backup_of_objects(
        self,
        board: &str,
        key: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        Self {
            fixture: self.fixture.backup_of_objects(board, key, objects),
            ..self
        }
    }

    /// Gives a board a cover: the file stored as `thumbnail`, fixed on it.
    #[must_use]
    pub fn cover(self, board: &str, thumbnail: &str) -> Self {
        Self {
            fixture: self.fixture.cover(board, thumbnail),
            ..self
        }
    }

    #[must_use]
    pub fn note(self, board: &str, text: &str) -> Self {
        Self {
            fixture: self.fixture.note(board, text),
            ..self
        }
    }

    #[must_use]
    pub fn thread(self, board: &str, thread: FixtureThread) -> Self {
        Self {
            fixture: self.fixture.thread(board, thread),
            ..self
        }
    }

    #[must_use]
    pub fn board_with_objects(
        self,
        name: &str,
        workspace: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        Self {
            fixture: self.fixture.board_with_objects(name, workspace, objects),
            ..self
        }
    }

    /// Puts an empty board into the workspace of that name, stored in
    /// `schema_version` of the board format on the server `provider`.
    #[must_use]
    pub fn board_stored_as(
        self,
        name: &str,
        workspace: &str,
        schema_version: u32,
        provider: &'static str,
    ) -> Self {
        Self {
            fixture: self.fixture.board_stored_as(
                name,
                workspace,
                schema_version,
                provider,
            ),
            ..self
        }
    }

    /// Puts a board with `objects` that the user may read and not change.
    #[must_use]
    pub fn read_only_board_with_objects(
        self,
        name: &str,
        workspace: &str,
        objects: Vec<FixtureObject>,
    ) -> Self {
        Self {
            fixture: self
                .fixture
                .read_only_board_with_objects(name, workspace, objects),
            ..self
        }
    }

    /// Makes the preview of the page at `url` carry `title`.
    #[must_use]
    pub fn link_preview(self, url: &str, title: &str) -> Self {
        Self {
            fixture: self.fixture.link_preview(url, title),
            ..self
        }
    }

    /// Makes the preview of the page at `url` fail.
    #[must_use]
    pub fn failing_link_preview(self, url: &str) -> Self {
        Self {
            fixture: self.fixture.failing_link_preview(url),
            ..self
        }
    }

    /// Puts a file called `name` with `bytes` into the folder the binaries run
    /// from.
    #[must_use]
    pub fn local_file(self, name: &str, bytes: &[u8]) -> Self {
        self.kept_file(&format!("home/{name}"), bytes)
    }

    /// Puts a file with `bytes` at `path` from the root of the run's folders
    /// (`home`, `holst`, `cache`), where holst keeps its own files.
    #[must_use]
    pub fn kept_file(self, path: &str, bytes: &[u8]) -> Self {
        Self {
            local_files: self
                .local_files
                .into_iter()
                .chain([(path.to_owned(), bytes.to_vec())])
                .collect(),
            ..self
        }
    }

    /// Starts the fake server and writes the saved login for the binaries.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn start(self) -> Result<Holst, ScenarioError> {
        let directory = TestDirectory::create()?;
        self.login
            .session_file()
            .map(|text| std::fs::write(directory.join("holst").join("session"), text))
            .transpose()?;
        self.local_files.iter().try_for_each(|(path, bytes)| {
            let file = directory.join(path);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(file, bytes)
        })?;
        let fake = Fake::start(self.fixture).await?;
        Ok(Holst::new(fake, directory))
    }
}
