//! Sharing and roles on either surface.

use serde_json::{Map, Value};

use crate::{
    cli,
    error::ScenarioError,
    holst::{Holst, On},
    mcp,
    observation::Observation,
    surface::{Surface, Words, arguments},
    then::Then,
};

/// A change of who may use a board.
#[derive(Debug, Clone, Copy)]
pub enum RightsChange<'a> {
    /// Gives `user` the right `right`, as the user typed it.
    Grant {
        user: &'a str,
        right: &'a str,
    },
    Revoke {
        user: &'a str,
    },
    /// Makes `user` the owner, and with `with_boards` of what is inside too.
    Transfer {
        user: &'a str,
        with_boards: bool,
    },
    /// Sets the sharing settings, each as a flag name and its value.
    Share {
        settings: &'a [(&'a str, &'a str)],
    },
}

impl<'a> RightsChange<'a> {
    /// Returns the action and what it takes besides the board and workspace.
    fn words(self, board: &'a str, workspace: &'a str) -> (&'static str, Words<'a>) {
        let (action, user, options, switches) = match self {
            RightsChange::Grant { user, right } => {
                ("grant", Some(user), vec![("right", right)], vec![])
            }
            RightsChange::Revoke { user } => ("revoke", Some(user), vec![], vec![]),
            RightsChange::Transfer { user, with_boards } => (
                "transfer",
                Some(user),
                vec![],
                with_boards.then_some("with_boards").into_iter().collect(),
            ),
            RightsChange::Share { settings } => {
                ("share", None, settings.to_vec(), vec![])
            }
        };
        (
            action,
            Words {
                positional: [("board", board)]
                    .into_iter()
                    .chain(user.map(|user| ("user", user)))
                    .collect(),
                options: options
                    .into_iter()
                    .chain([("workspace", workspace)])
                    .collect(),
                switches,
                confirms: true,
                writes: true,
            },
        )
    }
}

impl On<'_> {
    /// Changes who may use the board `board` of `workspace`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn change_rights(
        &self,
        board: &str,
        workspace: &str,
        change: RightsChange<'_>,
    ) -> Result<Observation, ScenarioError> {
        let (action, words) = change.words(board, workspace);
        let (flags, tool) = arguments(&words, self.mode);
        let observation = match self.surface {
            Surface::Cli => {
                let line = ["board", action]
                    .map(str::to_owned)
                    .into_iter()
                    .chain(flags);
                cli::run(self.holst.command("holst")?, line, |stdout| {
                    let text = String::from_utf8_lossy(stdout);
                    Ok(self.told(text.contains("\"not_held\""), &text))
                })
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board_write",
                    tool.into_iter()
                        .chain([("action".to_owned(), Value::from(action))])
                        .collect(),
                    |reply| {
                        self.told(reply.text.contains("nothing was sent"), &reply.text)
                    },
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    fn told(&self, not_held: bool, text: &str) -> Observation {
        if not_held {
            Observation::NotHeld
        } else {
            self.observed(text)
        }
    }
}

impl Then<'_> {
    /// Expects to be told that the user held no right, with nothing sent.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn told_no_right_was_held(&self) {
        assert_eq!(self.observation, &Observation::NotHeld);
    }
}

/// Expects things of the link settings of a board.
#[must_use]
pub fn sharing_of(holst: &Holst, board: &str) -> SharingThen {
    SharingThen {
        settings: holst.fake().sharing_of(board),
    }
}

#[derive(Debug)]
pub struct SharingThen {
    settings: Map<String, Value>,
}

impl SharingThen {
    /// Expects the board to hold exactly these settings.
    ///
    /// # Panics
    /// When it holds others, or these differ.
    pub fn holds_only(&self, expected: &Value) {
        assert_eq!(&Value::Object(self.settings.clone()), expected);
    }

    /// Expects the board to have no link settings.
    ///
    /// # Panics
    /// When it has some.
    pub fn is_untouched(&self) {
        assert!(self.settings.is_empty(), "{:?}", self.settings);
    }
}

/// Keeps copying out of the board called `board` to the role `needs` and up,
/// and gives the user the role `role` there.
pub fn withhold_copying(holst: &Holst, board: &str, needs: &str, role: &str) {
    holst
        .fake()
        .withhold(board, "who_can_copy_content", needs, role);
}

/// Keeps locking and unlocking on the board called `board` to the role `needs`
/// and up, and gives the user the role `role` there.
pub fn withhold_locking(holst: &Holst, board: &str, needs: &str, role: &str) {
    holst
        .fake()
        .withhold(board, "who_can_manage_lock_content", needs, role);
}

/// Keeps hiding and showing frame content on the board called `board` to the
/// role `needs` and up, and gives the user the role `role` there.
pub fn withhold_hiding(holst: &Holst, board: &str, needs: &str, role: &str) {
    holst
        .fake()
        .withhold(board, "who_can_manage_frame_hide_content", needs, role);
}
/// Keeps removing other people's objects on the board called `board` to the
/// role `needs` and up, and gives the user the role `role` there.
pub fn withhold_deleting(holst: &Holst, board: &str, needs: &str, role: &str) {
    holst
        .fake()
        .withhold(board, "who_can_delete_content", needs, role);
}

/// Expects the role `user` holds on the board called `board`, if any.
///
/// # Panics
/// When the role differs.
pub fn role_on(holst: &Holst, board: &str, user: &str, role: Option<&str>) {
    assert_eq!(
        holst.fake().role_of(board, user).as_deref(),
        role,
        "the right of {user} on {board}"
    );
}

/// Expects the person with the id `user` to own the board called `board`.
///
/// # Panics
/// When another person owns it.
pub fn owned_by(holst: &Holst, board: &str, user: &str) {
    assert_eq!(
        holst.fake().owner_of(board).as_deref(),
        Some(user),
        "the owner of {board}"
    );
}

/// Expects the person with the id `user` not to own the board called `board`.
///
/// # Panics
/// When they own it.
pub fn not_owned_by(holst: &Holst, board: &str, user: &str) {
    assert_ne!(
        holst.fake().owner_of(board).as_deref(),
        Some(user),
        "the owner of {board}"
    );
}

/// Returns the id of the node called `name`, to name a folder by what the
/// board lookup accepts.
///
/// # Panics
/// When the fixture holds no such node.
#[must_use]
pub fn id_of(holst: &Holst, name: &str) -> String {
    let id = holst.fake().id_of(name);
    assert!(id.is_some(), "the fixture holds no {name}");
    id.unwrap_or_default()
}
