//! A stateful fake Holst server on 127.0.0.1.

mod access;
mod board;
mod comment;
mod files;
mod fixture;
mod helpers;
mod http;
mod login;
mod server;
mod socket;
mod templates;
mod tree;
mod workspace;

use std::{
    io,
    net::SocketAddr,
    sync::{Arc, Mutex, PoisonError, mpsc},
    thread,
    time::Duration,
};

use serde_json::Value;
use tokio::{
    net::{TcpListener, TcpStream},
    runtime,
    time::timeout,
};

use crate::fake::{
    access::Access,
    comment::Comments,
    files::Files,
    login::Sessions,
    server::accept,
    templates::Templates,
    tree::Tree,
    workspace::Directory,
};
pub use crate::fake::{
    access::Right,
    board::FixtureObject,
    comment::{FixtureThread, ThreadAnchor, ThreadMessage},
    files::{Upload, png},
    fixture::{Fixture, FixtureError},
    helpers::{assert_none, candidates},
    login::LOGIN_CODE,
    tree::ListedNode,
    workspace::WorkspaceRole,
};

pub const ACCEPTED_TOKEN: &str = "csrf-accepted";
pub const ACCEPTED_COOKIES: &str = "sessionToken=accepted";

#[derive(Debug)]
struct State {
    workspaces: Directory,
    tree: Tree,
    catalog: Templates,
    access: Access,
    comments: Comments,
    files: Files,
    sessions: Sessions,
    failing: Vec<(String, u16)>,
    unanswered: Vec<String>,
    requests: Vec<String>,
    updates: usize,
}

type Shared = Arc<Mutex<State>>;

/// How long one probe waits for the listener, and how many probes it makes.
const PROBE: Duration = Duration::from_millis(250);
const PROBES: usize = 120;

/// Returns once a probe connects to the listener, trying anew when one hangs.
///
/// A loaded host sometimes holds back every new loopback connect for several
/// seconds and the client's connect then runs into its own timeout; the
/// probes take that wait, so a client of the fake connects at once.
async fn reachable(address: SocketAddr) {
    // imperative: each probe waits for the socket
    for _ in 0..PROBES {
        let connected = timeout(PROBE, TcpStream::connect(address)).await;
        if connected.is_ok_and(|stream| stream.is_ok()) {
            return;
        }
    }
}

/// A running fake server; it stops with the runtime that started it.
#[derive(Debug)]
pub struct Fake {
    base_url: String,
    state: Shared,
}

impl Fake {
    /// Starts the server on a free port of 127.0.0.1.
    ///
    /// # Errors
    /// The I/O error of binding the port; `Other` (a [`FixtureError`]) when the
    /// fixture names a workspace, folder or board it does not hold.
    pub async fn start(fixture: Fixture) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let base_url = format!("http://{address}");
        let state = Shared::new(Mutex::new(State::fixture(fixture)?));
        tokio::spawn(accept(listener, Arc::clone(&state)));
        reachable(address).await;
        Ok(Self { base_url, state })
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Starts the server on a thread of its own for a caller that has no
    /// runtime; the thread lives until the process ends.
    ///
    /// # Errors
    /// The I/O error of building the runtime or binding the port.
    pub fn start_blocking(fixture: Fixture) -> io::Result<Self> {
        let (started, receiver) = mpsc::channel();
        drop(thread::spawn(move || {
            let outcome = runtime::Builder::new_current_thread()
                .enable_io()
                .build()
                .map(|runtime| {
                    runtime.block_on(async {
                        let fake = Self::start(fixture).await;
                        // The receiver is gone only when the caller gave up.
                        let running = fake.is_ok();
                        drop(started.send(fake));
                        if running {
                            std::future::pending::<()>().await;
                        }
                    });
                });
            if let Err(error) = outcome {
                drop(started.send(Err(error)));
            }
        }));
        receiver.recv().map_err(io::Error::other)?
    }

    /// Returns the folders and boards of the workspace named `workspace`, in
    /// the order they came.
    #[must_use]
    pub fn tree_of(&self, workspace: &str) -> Vec<ListedNode> {
        self.workspace_id(workspace)
            .map(|id| self.locked().tree.nodes(&id, false))
            .unwrap_or_default()
    }

    /// Returns the folders and boards in the trash of the workspace named
    /// `workspace`.
    #[must_use]
    pub fn trash_of(&self, workspace: &str) -> Vec<ListedNode> {
        self.workspace_id(workspace)
            .map(|id| self.locked().tree.nodes(&id, true))
            .unwrap_or_default()
    }

    /// Returns the names of the favorites in the workspace named `workspace`.
    #[must_use]
    pub fn favorites_of(&self, workspace: &str) -> Vec<String> {
        self.workspace_id(workspace)
            .map(|id| self.locked().tree.favorites(&id))
            .unwrap_or_default()
    }

    /// Returns the id of the first folder or board called `name`.
    #[must_use]
    pub fn id_of(&self, name: &str) -> Option<String> {
        self.locked().tree.id_of(name)
    }

    /// Returns the id of the `number`th object filled on the board named
    /// `board`, counting from 1.
    #[must_use]
    pub fn object_id(&self, board: &str, number: u32) -> Option<String> {
        self.id_of(board).map(|id| board::object_id(&id, number))
    }

    /// Returns how many updates of boards the socket received.
    #[must_use]
    pub fn updates_received(&self) -> usize {
        self.locked().updates
    }

    /// Returns the names of the signed-in user's own templates.
    #[must_use]
    pub fn personal_templates(&self) -> Vec<String> {
        self.locked().catalog.personal_names()
    }

    /// Returns the names of the templates shared with the workspace named
    /// `workspace`.
    #[must_use]
    pub fn workspace_templates(&self, workspace: &str) -> Vec<String> {
        self.workspace_id(workspace)
            .map(|id| self.locked().catalog.workspace_names(&id))
            .unwrap_or_default()
    }

    /// Returns the id of the own template called `name`.
    #[must_use]
    pub fn template_id(&self, name: &str) -> Option<String> {
        self.locked().catalog.personal_id(name)
    }

    /// Returns the fields of the own or workspace template called `name`.
    #[must_use]
    pub fn template_of(&self, name: &str) -> Option<Value> {
        self.locked().catalog.json_of(name)
    }

    /// Returns the link settings of the board called `board`.
    #[must_use]
    pub fn sharing_of(&self, board: &str) -> serde_json::Map<String, Value> {
        let state = self.locked();
        state
            .tree
            .id_of(board)
            .map(|id| state.access.settings_of(&id))
            .unwrap_or_default()
    }

    /// Keeps the board setting `setting` (such as `who_can_copy_content`) on
    /// the board called `board` to the role `needs` and up, and gives the user
    /// the role `role` there.
    pub fn withhold(&self, board: &str, setting: &str, needs: &str, role: &str) {
        let mut state = self.locked();
        if let Some(id) = state.tree.id_of(board) {
            state.access.withhold(&id, setting, needs, role);
        }
    }

    /// Returns the role `user` holds on the board called `board`.
    #[must_use]
    pub fn role_of(&self, board: &str, user: &str) -> Option<String> {
        let state = self.locked();
        state
            .tree
            .id_of(board)
            .and_then(|id| state.access.role_of(&id, user))
    }

    /// Returns the id of the person who owns the board called `board`.
    #[must_use]
    pub fn owner_of(&self, board: &str) -> Option<String> {
        let state = self.locked();
        state
            .tree
            .id_of(board)
            .and_then(|id| state.tree.owner_of(&id))
    }

    /// Returns the comment threads of the board called `board`, in the order
    /// they came.
    #[must_use]
    pub fn threads_of(&self, board: &str) -> Vec<FixtureThread> {
        let state = self.locked();
        state
            .tree
            .id_of(board)
            .map(|id| state.comments.threads_of(&id))
            .unwrap_or_default()
    }

    /// Returns when the signed-in user last read the thread `thread` of the
    /// board called `board`, as the server holds the time.
    #[must_use]
    pub fn thread_read_at(&self, board: &str, thread: &str) -> Option<String> {
        let state = self.locked();
        state
            .tree
            .id_of(board)
            .and_then(|id| state.comments.read_at(&id, thread))
    }

    /// Returns the files received, in order.
    #[must_use]
    pub fn uploads(&self) -> Vec<Upload> {
        self.locked().files.uploads()
    }

    /// Returns the page runs the page service was asked to convert, in order:
    /// the stored file name, the first page and the last.
    #[must_use]
    pub fn page_runs(&self) -> Vec<(String, u32, u32)> {
        self.locked().files.page_runs()
    }

    /// Returns the pictures uploaded outside any board, such as the thumbnail
    /// of a template.
    #[must_use]
    pub fn thumbnails(&self) -> Vec<Upload> {
        self.uploads()
            .into_iter()
            .filter(|upload| upload.board.is_none())
            .collect()
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn workspace_id(&self, name: &str) -> Option<String> {
        self.locked().workspaces.id_of(name)
    }

    /// Makes the API path `path` (as `current_user`) answer with `status` from
    /// now on.
    pub fn fail(&self, path: &str, status: u16) {
        let failed = format!("/gapi/{path}");
        let mut state = self.locked();
        state.failing.retain(|(known, _)| *known != failed);
        state.failing.push((failed, status));
    }

    /// Returns the requests no endpoint of the fake answers, as `METHOD path`.
    #[must_use]
    pub fn unanswered(&self) -> Vec<String> {
        self.locked().unanswered.clone()
    }

    /// Returns every request the fake got, in order: `METHOD path` without the
    /// query, `WS open`, `WS RequestDocument <room>` or `WS Update <room>`.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        self.locked().requests.clone()
    }
}
