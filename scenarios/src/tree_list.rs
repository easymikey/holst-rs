//! The tree of boards and folders as each surface lists it.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    cli,
    error::ScenarioError,
    holst::{On, SendMode},
    mcp,
    observation::{ListedTree, Observation},
    surface::{Surface, Words, arguments},
    then::Then,
};

static NO_PAGE: ListedTree = ListedTree {
    rows: Vec::new(),
    cursor: None,
};
const TRASH_MARK: &str = " [in trash]";

/// A listing: what narrows it. `view` is spelled as on MCP.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ls<'a> {
    workspace: Option<&'a str>,
    folder: Option<&'a str>,
    view: Option<&'a str>,
    limit: Option<u32>,
    cursor: Option<&'a str>,
}

impl<'a> Ls<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            workspace: None,
            folder: None,
            view: None,
            limit: None,
            cursor: None,
        }
    }

    /// Looks only in this workspace.
    #[must_use]
    pub const fn workspace(self, workspace: &'a str) -> Self {
        Self {
            workspace: Some(workspace),
            ..self
        }
    }

    /// Lists only this folder.
    #[must_use]
    pub const fn folder(self, folder: &'a str) -> Self {
        Self {
            folder: Some(folder),
            ..self
        }
    }

    /// Lists `tree`, `trash`, `both`, `favorites`, `recent` or `sidebar`.
    #[must_use]
    pub const fn view(self, view: &'a str) -> Self {
        Self {
            view: Some(view),
            ..self
        }
    }

    /// Shows at most this many nodes; `0` is all.
    #[must_use]
    pub const fn limit(self, limit: u32) -> Self {
        Self {
            limit: Some(limit),
            ..self
        }
    }

    /// Continues after this cursor.
    #[must_use]
    pub const fn cursor(self, cursor: &'a str) -> Self {
        Self {
            cursor: Some(cursor),
            ..self
        }
    }

    /// Returns the command line of `holst` and the fields of the tool.
    fn arguments(&self) -> (Vec<String>, Map<String, Value>) {
        let limit = self.limit.map(|number| number.to_string());
        let words = Words {
            options: [
                ("workspace", self.workspace),
                ("folder", self.folder),
                ("view", self.view),
                ("limit", limit.as_deref()),
                ("cursor", self.cursor),
            ]
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?)))
            .collect(),
            ..Words::default()
        };
        let (line, mut tool) = arguments(&words, SendMode::Send);
        tool.extend(
            self.limit
                .map(|number| ("limit".to_owned(), Value::from(number))),
        );
        (
            ["board", "ls"]
                .map(str::to_owned)
                .into_iter()
                .chain(line)
                .collect(),
            tool,
        )
    }
}

impl On<'_> {
    /// Lists boards and folders.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_tree(&self, ls: Ls<'_>) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(self.holst.command("holst")?, ls.arguments().0, |stdout| {
                    Ok(Observation::Tree(rows_of_json(stdout)?))
                })
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board",
                    ls.arguments()
                        .1
                        .into_iter()
                        .chain([("action".to_owned(), Value::from("ls"))])
                        .collect(),
                    |reply| Observation::Tree(rows_of_text(&reply.text)),
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    /// Renames the board that a listing of the workspace showed under
    /// `shown`, naming it by the handle the listing gave it. Only MCP hands
    /// handles out.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn rename_listed_board(
        &self,
        shown: &str,
        workspace: &str,
        name: &str,
    ) -> Result<Observation, ScenarioError> {
        let observation = mcp::call_then(
            self.holst.command("holst-mcp")?,
            "board",
            Map::from_iter([
                ("action".to_owned(), Value::from("ls")),
                ("workspace".to_owned(), Value::from(workspace)),
            ]),
            |reply| {
                let handle = reply
                    .text
                    .lines()
                    .find(|line| line.trim_start().starts_with(shown))
                    .and_then(|line| line.rsplit_once(' '))
                    .map(|(_, handle)| handle.to_owned());
                handle.map(|handle| {
                    (
                        "board_write",
                        [
                            ("action", "rename"),
                            ("board", handle.as_str()),
                            ("name", name),
                            ("workspace", workspace),
                        ]
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), Value::from(value)))
                        .collect(),
                    )
                })
            },
            |_| Observation::Done,
        )
        .await?;
        self.holst.checked(observation)
    }
}

fn rows_of_json(stdout: &[u8]) -> Result<ListedTree, ScenarioError> {
    #[derive(Deserialize)]
    struct Printed {
        nodes: Vec<Node>,
        cursor: Option<String>,
    }
    #[derive(Deserialize)]
    struct Node {
        name: String,
        kind: String,
        depth: usize,
        in_trash: bool,
    }
    let printed: Printed = serde_json::from_slice(stdout)?;
    Ok(ListedTree {
        rows: printed
            .nodes
            .into_iter()
            .map(|node| {
                format!(
                    "{:indent$}{}{}{}",
                    "",
                    node.name,
                    if node.kind == "folder" { "/" } else { "" },
                    if node.in_trash { TRASH_MARK } else { "" },
                    indent = node.depth * 2,
                )
            })
            .collect(),
        cursor: printed.cursor,
    })
}

/// One line per node, `indent name[/] @N[ [in trash]]`, a heading per
/// workspace when there are several, and `… N of M; next: cursor C` at the end.
fn rows_of_text(text: &str) -> ListedTree {
    ListedTree {
        rows: text
            .lines()
            .filter_map(|line| {
                let (body, trash) = line
                    .strip_suffix(TRASH_MARK)
                    .map_or((line, ""), |body| (body, TRASH_MARK));
                body.rsplit_once(" @")
                    .map(|(name, _handle)| format!("{name}{trash}"))
            })
            .collect(),
        cursor: text
            .lines()
            .find_map(|line| line.split_once("; next: cursor "))
            .map(|(_, cursor)| cursor.to_owned()),
    }
}

impl Then<'_> {
    fn tree(&self) -> &ListedTree {
        let page = self.observation.tree();
        assert!(
            page.is_some(),
            "expected a listing, got {:?}",
            self.observation
        );
        page.unwrap_or(&NO_PAGE)
    }

    /// Expects exactly these rows, in this order.
    ///
    /// # Panics
    /// When the listing differs.
    pub fn lists_tree(&self, rows: &[&str]) {
        assert_eq!(self.tree().rows, rows, "the rows of the listing");
    }

    /// Expects these rows among the listed ones; a listing over several
    /// workspaces has a heading row for each of them on MCP.
    ///
    /// # Panics
    /// When one is missing.
    pub fn lists_among(&self, rows: &[&str]) {
        let shown = &self.tree().rows;
        assert!(
            rows.iter().all(|row| shown.iter().any(|seen| seen == row)),
            "expected the rows {rows:?} among {shown:?}"
        );
    }

    /// Expects none of these rows.
    ///
    /// # Panics
    /// When one is there.
    pub fn lists_none_of(&self, rows: &[&str]) {
        let shown = &self.tree().rows;
        assert!(
            rows.iter().all(|row| shown.iter().all(|seen| seen != row)),
            "expected none of the rows {rows:?} in {shown:?}"
        );
    }

    /// Returns the cursor of the next page.
    ///
    /// # Panics
    /// When there is none.
    #[must_use]
    pub fn next_cursor(&self) -> String {
        let cursor = self.tree().cursor.clone();
        assert!(cursor.is_some(), "expected a cursor in {:?}", self.tree());
        cursor.unwrap_or_default()
    }

    /// Expects the listing to end here.
    ///
    /// # Panics
    /// When a cursor follows.
    pub fn ends_the_listing(&self) {
        assert_eq!(self.tree().cursor, None, "the cursor of the listing");
    }
}
