//! A running fake Holst and the surfaces that reach it.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, OnceLock},
};

use holst_core::fake::Fake;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    cli,
    directory::TestDirectory,
    error::{BuildError, ScenarioError},
    mcp,
    observation::Observation,
    session::LISTING_SEPARATOR,
    surface::Surface,
};

/// How a step sends: carried out, only shown, or confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendMode {
    Send,
    DryRun,
    Confirmed,
}

#[derive(Debug)]
pub struct Holst {
    pub(crate) fake: Fake,
    directory: TestDirectory,
}

/// A surface of a running Holst: what the user does there.
#[derive(Debug)]
pub struct On<'holst> {
    pub(crate) holst: &'holst Holst,
    pub(crate) surface: Surface,
    pub(crate) mode: SendMode,
}

impl Holst {
    pub(crate) const fn new(fake: Fake, directory: TestDirectory) -> Self {
        Self { fake, directory }
    }

    #[must_use]
    pub const fn on(&self, surface: Surface) -> On<'_> {
        On {
            holst: self,
            surface,
            mode: SendMode::Send,
        }
    }

    pub(crate) const fn fake(&self) -> &Fake {
        &self.fake
    }

    /// Returns the path of a file in the folder the binaries run from.
    pub(crate) fn work_file(&self, name: &str) -> PathBuf {
        self.directory.join("home").join(name)
    }

    /// How many page counts the fake was asked for.
    #[must_use]
    pub fn page_counts_asked(&self) -> usize {
        self.fake
            .requests()
            .iter()
            .filter(|request| request.ends_with("/page_count"))
            .count()
    }

    /// The page runs the fake was asked to convert: file, first page, last.
    #[must_use]
    pub fn page_runs(&self) -> Vec<(String, u32, u32)> {
        self.fake.page_runs()
    }

    /// Returns the id the fake gave the board with that name.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds none.
    pub fn board_id(&self, name: &str) -> Result<String, ScenarioError> {
        self.fake.id_of(name).ok_or_else(|| ScenarioError::NoBoard {
            name: name.to_owned(),
        })
    }

    /// Returns how `how` refers to the board with that name.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds none.
    pub fn reference_to(
        &self,
        how: BoardReference,
        name: &str,
    ) -> Result<String, ScenarioError> {
        match how {
            BoardReference::Part => Ok(name.chars().skip(2).take(4).collect()),
            BoardReference::Id => self.board_id(name),
            BoardReference::Link => self.board_link(name),
        }
    }

    /// Returns the link that opens the board with that name.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds none.
    pub fn board_link(&self, name: &str) -> Result<String, ScenarioError> {
        Ok(format!(
            "{}/board/{}",
            self.fake.base_url(),
            self.board_id(name)?
        ))
    }

    /// Returns the names of the boards the account opened, the last first.
    #[must_use]
    pub fn recent_boards(&self) -> Vec<String> {
        self.fake.recent_of()
    }

    pub(crate) fn path(&self, part: &str) -> PathBuf {
        self.directory.join(part)
    }

    pub(crate) fn command(&self, binary: &str) -> Result<Command, ScenarioError> {
        // imperative: Command is a builder that takes its settings by &mut
        let mut command = Command::new(binary_path(binary)?);
        command
            .env_clear()
            .envs(std::env::vars_os().filter(|(name, _)| {
                ["PATH", "SystemRoot", "LLVM_PROFILE_FILE"]
                    .iter()
                    .any(|kept| name.eq_ignore_ascii_case(kept))
            }))
            .env("HOME", self.directory.join("home"))
            .env("USERPROFILE", self.directory.join("home"))
            .env("HOLST_HOME", self.directory.join("holst"))
            .env("HOLST_CACHE_DIR", self.directory.join("cache"))
            .env("HOLST_BASE_URL", self.fake.base_url())
            .env("NO_COLOR", "1");
        Ok(command)
    }

    pub(crate) fn checked(
        &self,
        observation: Observation,
    ) -> Result<Observation, ScenarioError> {
        let requests = self.fake.unanswered();
        if requests.is_empty() {
            Ok(observation)
        } else {
            Err(ScenarioError::Unanswered { requests })
        }
    }
}

impl On<'_> {
    /// Makes the changes that follow only show what they would do.
    #[must_use]
    pub const fn dry_running(self) -> Self {
        Self {
            mode: SendMode::DryRun,
            ..self
        }
    }

    /// Makes the changes that follow carry along what they take with them,
    /// such as the boards of a folder.
    #[must_use]
    pub const fn confirming(self) -> Self {
        Self {
            mode: SendMode::Confirmed,
            ..self
        }
    }

    /// Lists the workspaces of the saved login, as this surface shows them.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_workspaces(&self) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(
                    self.holst.command("holst")?,
                    ["workspace", "ls"],
                    workspaces,
                )
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "workspace",
                    Map::from_iter([("action".to_owned(), Value::from("ls"))]),
                    listing,
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

#[derive(Deserialize)]
struct Workspace {
    name: String,
}

fn workspaces(stdout: &[u8]) -> Result<Observation, ScenarioError> {
    Ok(Observation::Workspaces(
        serde_json::from_slice::<Vec<Workspace>>(stdout)?
            .into_iter()
            .map(|workspace| workspace.name)
            .collect(),
    ))
}

fn listing(reply: &mcp::Reply) -> Observation {
    Observation::Workspaces(
        reply
            .text
            .lines()
            .filter_map(|line| line.split(LISTING_SEPARATOR).next())
            .filter_map(|entry| entry.rsplit_once(' '))
            .map(|(name, _alias)| name.to_owned())
            .collect(),
    )
}

/// Finds a binary next to the test executable's profile directory
/// (`target/<profile>/`), after building the binaries once per test run so a
/// stale one is never tested.
fn binary_path(name: &str) -> Result<PathBuf, ScenarioError> {
    static BUILT: OnceLock<Result<(), BuildError>> = OnceLock::new();
    let profile = std::env::current_exe()?
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    BUILT.get_or_init(|| build(&profile)).clone()?;
    let path = profile.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(ScenarioError::MissingBinary { path })
    }
}

/// Builds `holst` and `holst-mcp` in the profile the tests run in; cargo
/// leaves up-to-date binaries alone.
fn build(profile: &Path) -> Result<(), BuildError> {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or("cargo".into()));
    command.args(["build", "--quiet", "-p", "holst-cli", "-p", "holst-mcp"]);
    let flags: &[&str] = match profile.file_name().and_then(|name| name.to_str()) {
        Some("release") => &["--release"],
        Some(custom) if custom != "debug" => &["--profile", custom],
        Some(_) | None => &[],
    };
    let output = command
        .args(flags)
        .output()
        .map_err(|error| BuildError::Spawn(Arc::new(error)))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BuildError::Failed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

impl On<'_> {
    pub(crate) fn admin_arguments(&self, arguments: Value) -> Value {
        with(
            arguments,
            [
                ("confirm", Value::from(self.mode == SendMode::Confirmed)),
                ("dry_run", Value::from(self.mode == SendMode::DryRun)),
            ],
        )
    }

    pub(crate) async fn reading(
        &self,
        line: &[&str],
        tool: &'static str,
        arguments: Value,
    ) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(self.holst.command("holst")?, line, |stdout| {
                    Ok(Observation::Shown(
                        String::from_utf8_lossy(stdout).into_owned(),
                    ))
                })
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    tool,
                    fields(arguments),
                    |reply| Observation::Shown(reply.text.clone()),
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    pub(crate) async fn changing(
        &self,
        line: &[&str],
        tool: &'static str,
        arguments: Value,
    ) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(self.holst.command("holst")?, line, |stdout| {
                    Ok(self.observed(&String::from_utf8_lossy(stdout)))
                })
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    tool,
                    fields(arguments),
                    |reply| self.observed(&reply.text),
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    pub(crate) fn observed(&self, text: &str) -> Observation {
        match self.mode {
            SendMode::Send | SendMode::Confirmed => Observation::Done,
            SendMode::DryRun => Observation::Shown(text.to_owned()),
        }
    }
}

/// Returns the fields of `arguments`.
pub(crate) fn fields(arguments: Value) -> Map<String, Value> {
    match arguments {
        Value::Object(map) => map,
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_) => Map::new(),
    }
}

/// Returns `arguments` with the fields in `set` added.
pub(crate) fn with<const N: usize>(arguments: Value, set: [(&str, Value); N]) -> Value {
    Value::Object(
        fields(arguments)
            .into_iter()
            .chain(set.into_iter().map(|(key, field)| (key.to_owned(), field)))
            .collect(),
    )
}

/// How a command refers to a board.
#[derive(Debug, Clone, Copy)]
pub enum BoardReference {
    /// A part of the name.
    Part,
    Id,
    Link,
}
