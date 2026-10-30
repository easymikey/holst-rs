//! The tree changes a scenario makes, and how each surface is asked for them.

use serde::Deserialize;
use serde_json::Value;

use crate::{
    cli,
    error::ScenarioError,
    holst::On,
    mcp,
    observation::Observation,
    surface::{Surface, Words, arguments},
};

/// Where a node is moved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewParent<'a> {
    Root,
    Folder(&'a str),
}

/// What a copy of a board is given besides the original's content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyAs<'a> {
    pub name: Option<&'a str>,
    pub folder: Option<&'a str>,
    /// Whether the copy keeps the rights of the original.
    pub rights: bool,
}

/// A change of one board, named by `node`.
fn of_node<'a>(node: &'a str, workspace: &'a str) -> Words<'a> {
    Words {
        positional: vec![("board", node)],
        options: vec![("workspace", workspace)],
        ..Words::default()
    }
}

impl On<'_> {
    /// Creates a folder at the root of `workspace`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn create_folder(
        &self,
        name: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "create",
            Words {
                positional: vec![("name", name)],
                options: vec![("workspace", workspace), ("kind", "folder")],
                ..Words::default()
            },
        )
        .await
    }

    /// Creates a board in `workspace`, in a folder or at the root.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn create_board(
        &self,
        name: &str,
        workspace: &str,
        folder: Option<&str>,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "create",
            Words {
                positional: vec![("name", name)],
                options: [("workspace", workspace)]
                    .into_iter()
                    .chain(folder.map(|folder| ("folder", folder)))
                    .collect(),
                ..Words::default()
            },
        )
        .await
    }

    /// Creates a board without saying where, which a login with one workspace
    /// allows.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn create_board_anywhere(
        &self,
        name: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "create",
            Words {
                positional: vec![("name", name)],
                ..Words::default()
            },
        )
        .await
    }

    /// Moves the board or folder named `node` of `workspace`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn move_node(
        &self,
        node: &str,
        workspace: &str,
        to: NewParent<'_>,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "mv",
            Words {
                positional: vec![
                    ("board", node),
                    (
                        "folder",
                        match to {
                            NewParent::Root => "/",
                            NewParent::Folder(folder) => folder,
                        },
                    ),
                ],
                options: vec![("workspace", workspace)],
                ..Words::default()
            },
        )
        .await
    }

    /// Copies the board named `node` of `workspace` next to the original.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn duplicate_board(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.copy_board(node, workspace, CopyAs::default()).await
    }

    /// Copies the board named `node` of `workspace` as `copy` says.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn copy_board(
        &self,
        node: &str,
        workspace: &str,
        copy: CopyAs<'_>,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "cp",
            Words {
                options: [("workspace", workspace)]
                    .into_iter()
                    .chain(copy.name.map(|name| ("name", name)))
                    .chain(copy.folder.map(|folder| ("folder", folder)))
                    .collect(),
                switches: copy.rights.then_some("rights").into_iter().collect(),
                ..of_node(node, workspace)
            },
        )
        .await
    }

    /// Moves the board or folder named `node` of `workspace` to the trash.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn delete_node(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change("rm", of_node(node, workspace)).await
    }

    /// Deletes the board or folder named `node` of `workspace` for good.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn delete_permanently(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "rm",
            Words {
                switches: vec!["permanently"],
                ..of_node(node, workspace)
            },
        )
        .await
    }

    /// Brings the board or folder named `node` back from the trash of
    /// `workspace`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn restore_node(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change("restore", of_node(node, workspace)).await
    }

    /// Adds the board named `node` of `workspace` to the favorites.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn favorite_board(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change("favorite", of_node(node, workspace)).await
    }

    /// Adds the board named `node` of `workspace` to the favorites at `index`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn favorite_board_at(
        &self,
        node: &str,
        workspace: &str,
        index: i64,
    ) -> Result<Observation, ScenarioError> {
        let index = index.to_string();
        self.change(
            "favorite",
            Words {
                options: vec![("workspace", workspace), ("index", &index)],
                ..of_node(node, workspace)
            },
        )
        .await
    }

    /// Takes the board named `node` of `workspace` out of the favorites.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn unfavorite_board(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change("unfavorite", of_node(node, workspace)).await
    }

    /// Marks the board named `node` of `workspace` as recently opened.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn touch_board(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change("touch", of_node(node, workspace)).await
    }

    /// Leaves the board named `node` of `workspace`, which the user was
    /// invited to.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn leave_board(
        &self,
        node: &str,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change("leave", of_node(node, workspace)).await
    }

    /// Renames the board or folder named `node` of `workspace`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn rename_node(
        &self,
        node: &str,
        workspace: &str,
        name: &str,
    ) -> Result<Observation, ScenarioError> {
        self.change(
            "rename",
            Words {
                positional: vec![("board", node), ("name", name)],
                options: vec![("workspace", workspace)],
                ..Words::default()
            },
        )
        .await
    }

    pub(crate) async fn change(
        &self,
        action: &'static str,
        words: Words<'_>,
    ) -> Result<Observation, ScenarioError> {
        let words = Words {
            confirms: matches!(action, "rm" | "mv" | "tag"),
            writes: true,
            ..words
        };
        let (flags, mut tool) = arguments(&words, self.mode);
        // the tool takes the index as a number
        let index = words
            .options
            .iter()
            .find(|(name, _)| *name == "index")
            .and_then(|(_, index)| index.parse::<i64>().ok());
        tool.extend(index.map(|index| ("index".to_owned(), Value::from(index))));
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(
                    self.holst.command("holst")?,
                    ["board", action]
                        .map(str::to_owned)
                        .into_iter()
                        .chain(flags),
                    |stdout| Ok(shown_on_cli(stdout)),
                )
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board_write",
                    tool.into_iter()
                        .chain([("action".to_owned(), Value::from(action))])
                        .collect(),
                    shown_on_mcp,
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

/// A preview, or a change held for its confirmation, carries the request it
/// would send; a change that went through carries the reply, which scenarios
/// read from the tree instead.
fn shown_on_cli(stdout: &[u8]) -> Observation {
    #[derive(Deserialize)]
    struct Printed {
        outcome: Outcome,
    }
    #[derive(Deserialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    enum Outcome {
        Sent,
        DryRun { request: Request },
        Held { request: Request },
    }
    #[derive(Deserialize)]
    struct Request {
        path: String,
        body: Value,
    }
    match serde_json::from_slice::<Printed>(stdout).map(|printed| printed.outcome) {
        Ok(Outcome::DryRun { request } | Outcome::Held { request }) => {
            Observation::DryRun {
                path: request.path,
                body: request.body,
            }
        }
        Ok(Outcome::Sent) | Err(_) => Observation::Done,
    }
}

/// A preview reads a sentence with `would`, the request line (`METHOD path`),
/// and its JSON body when it has one.
fn shown_on_mcp(reply: &mcp::Reply) -> Observation {
    let mut lines = reply.text.lines();
    lines
        .next()
        .filter(|sentence| sentence.starts_with("would "))
        .and_then(|_| lines.next())
        .and_then(|line| line.split_once(' '))
        .map_or(Observation::Done, |(_, path)| Observation::DryRun {
            path: path.to_owned(),
            body: lines
                .next()
                .and_then(|body| serde_json::from_str(body).ok())
                .unwrap_or(Value::Null),
        })
}

/// A command of the board tree.
#[derive(Debug, Clone, Copy)]
pub enum TreeCommand {
    List,
    Create,
    Rename,
    Remove,
    Move,
    Copy,
    Favorite,
}

impl On<'_> {
    /// Runs a tree command on the board `board` in `Team` (`Create` makes it;
    /// `Rename` renames it to `Plan 2`, `Move` moves it into `Work`).
    ///
    /// # Errors
    /// Whatever the surface fails with before it answers.
    pub async fn run_tree_command(
        &self,
        command: TreeCommand,
        board: &str,
    ) -> Result<Observation, ScenarioError> {
        match command {
            TreeCommand::List => self.list_tree(crate::tree_list::Ls::new()).await,
            TreeCommand::Create => self.create_board(board, "Team", None).await,
            TreeCommand::Rename => self.rename_node(board, "Team", "Plan 2").await,
            TreeCommand::Remove => self.delete_node(board, "Team").await,
            TreeCommand::Move => {
                self.move_node(board, "Team", NewParent::Folder("Work"))
                    .await
            }
            TreeCommand::Copy => self.duplicate_board(board, "Team").await,
            TreeCommand::Favorite => self.favorite_board(board, "Team").await,
        }
    }
}
