//! Exporting a board as a picture, and saving it as a folder.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    cli,
    error::ScenarioError,
    holst::{Holst, On, SendMode},
    mcp,
    observation::{Observation, PictureFormat},
    surface::Surface,
    then::Then,
    whole::Look,
};

impl On<'_> {
    /// Exports the picture of the board with that name: a file on the CLI,
    /// the reply of the snapshot tool on MCP.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn export_picture(
        &self,
        board: &str,
        picture: PictureFormat,
    ) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                self.holst
                    .snap_to_file_with(
                        board,
                        &format!("{board}.{}", picture.extension()),
                        &Look::new(),
                    )
                    .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board",
                    Map::from_iter([
                        ("action".to_owned(), Value::from("snap")),
                        ("board".to_owned(), Value::from(board)),
                        ("format".to_owned(), Value::from(picture.extension())),
                    ]),
                    picture_of,
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

/// A picture reads as an image block, or as SVG text.
fn picture_of(reply: &mcp::Reply) -> Observation {
    Observation::Picture(
        reply
            .image_types
            .iter()
            .find_map(|mime| PictureFormat::named(mime))
            .or_else(|| PictureFormat::of(reply.text.as_bytes())),
    )
}

impl Holst {
    /// Exports the picture of the board into a new file of that name, on the
    /// CLI, whose `snap` takes the format from the extension, narrowed or
    /// drawn as `look` says.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn snap_to_file_with(
        &self,
        board: &str,
        file: &str,
        look: &Look<'_>,
    ) -> Result<Observation, ScenarioError> {
        let path = self.path("snapshots").join(file);
        let observation = cli::run(
            self.command("holst")?,
            ["board", "snap", board]
                .map(std::ffi::OsString::from)
                .into_iter()
                .chain([path.as_os_str().to_owned()])
                .chain(look.arguments().0.into_iter().map(std::ffi::OsString::from)),
            |_| {
                Ok(Observation::Picture(
                    std::fs::read(&path)
                        .ok()
                        .and_then(|bytes| PictureFormat::of(&bytes)),
                ))
            },
        )
        .await?;
        self.checked(observation)
    }
}

impl Then<'_> {
    /// Expects a picture in this format.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn is_a_picture(&self, picture: PictureFormat) {
        assert_eq!(self.observation, &Observation::Picture(Some(picture)));
    }

    /// Expects a saved board that lists `count` originals as not saved.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_not_saved(&self, count: usize) {
        assert!(
            matches!(self.observation, Observation::Saved { not_saved } if not_saved.len() == count),
            "expected {count} originals not saved, got {:?}",
            self.observation
        );
    }
}

/// Expects things of the folders `holst save` left, hidden ones too, with
/// every path inside each.
#[must_use]
pub fn saved_in(holst: &Holst) -> SavedThen {
    let saved = holst.path("saved");
    let folders = entries(&saved, "")
        .into_iter()
        .filter(|path| path.matches('/').count() == 1)
        .filter_map(|path| path.strip_suffix('/').map(str::to_owned))
        .map(|folder| {
            let inside = entries(&saved.join(&folder), "");
            (folder, inside)
        })
        .collect();
    SavedThen {
        root: saved,
        folders,
    }
}

#[derive(Debug)]
pub struct SavedThen {
    root: std::path::PathBuf,
    folders: Vec<(String, Vec<String>)>,
}

impl SavedThen {
    /// Expects one folder named `name-<short id>` holding these files.
    ///
    /// # Panics
    /// When there is none, or its files differ.
    pub fn is_one_folder_of(&self, name: &str, files: &[&str]) {
        let wanted: Vec<String> = files.iter().map(|file| (*file).to_owned()).collect();
        assert!(
            matches!(self.folders.as_slice(), [(folder, found)]
                if folder.starts_with(&format!("{name}-")) && *found == wanted),
            "expected one folder of {name}-* holding {wanted:?}, found {:?}",
            self.folders
        );
    }
}

impl SavedThen {
    /// Expects `count` folders named `name-<short id>`, `name-<short id>-2`
    /// and so on, none overwritten by the next.
    ///
    /// # Panics
    /// When the folders differ.
    pub fn is_folders_of(&self, name: &str, count: usize) {
        let first = self.folders.first().map(|(folder, _)| folder.as_str());
        assert!(
            self.folders.len() == count
                && first.is_some_and(|first| first.starts_with(&format!("{name}-")))
                && self
                    .folders
                    .iter()
                    .skip(1)
                    .zip(2..)
                    .all(|((folder, _), number)| {
                        first
                            .is_some_and(|first| *folder == format!("{first}-{number}"))
                    }),
            "expected {count} folders of {name}-*, found {:?}",
            self.folders
        );
    }

    /// Expects the one folder `name-<short id>` to hold `objects.json` with
    /// exactly the objects of these texts, a frame's own among them.
    ///
    /// # Panics
    /// When there is not one folder, or its objects differ.
    pub fn holds_objects_of(&self, name: &str, texts: &[&str]) {
        let folder = self.folders.first().map(|(folder, _)| folder.as_str());
        assert!(
            self.folders.len() == 1
                && folder.is_some_and(|folder| folder.starts_with(&format!("{name}-"))),
            "expected one folder of {name}-*, found {:?}",
            self.folders
        );
        let folder = folder.unwrap_or_default();
        let json: Value =
            std::fs::read_to_string(self.root.join(folder).join("objects.json"))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
        let mut held: Vec<&str> = json
            .get("objects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|object| object.get("text").and_then(Value::as_str))
            .collect();
        held.sort_unstable();
        let mut expected = texts.to_vec();
        expected.sort_unstable();
        assert_eq!(held, expected, "the objects of {folder}");
    }
}

/// Lists what lies in `directory` and below it, as paths under `prefix`.
pub(crate) fn entries(directory: &Path, prefix: &str) -> Vec<String> {
    let mut names: Vec<(String, bool)> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path().is_dir(),
            )
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .flat_map(|(name, is_directory)| {
            let nested = is_directory.then(|| {
                let inside = format!("{prefix}{name}/");
                [inside.clone()]
                    .into_iter()
                    .chain(entries(&directory.join(&name), &inside))
                    .collect::<Vec<_>>()
            });
            nested.unwrap_or_else(|| vec![format!("{prefix}{name}")])
        })
        .collect()
}

/// How an export is written.
#[derive(Debug, Clone, Copy, Default)]
pub struct Export {
    uncompressed: bool,
    object_list: bool,
}

impl Export {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            uncompressed: false,
            object_list: false,
        }
    }

    /// Writes the file without gzip.
    #[must_use]
    pub const fn uncompressed(self) -> Self {
        Self {
            uncompressed: true,
            ..self
        }
    }

    /// Also writes the objects beside the file, one per line.
    #[must_use]
    pub const fn object_list(self) -> Self {
        Self {
            object_list: true,
            ..self
        }
    }
}

/// Where an import goes and what it is called; a new board unless `into`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Import<'a> {
    workspace: Option<&'a str>,
    folder: Option<&'a str>,
    name: Option<&'a str>,
    into: Option<&'a str>,
    merge: bool,
}

impl<'a> Import<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            workspace: None,
            folder: None,
            name: None,
            into: None,
            merge: false,
        }
    }

    /// Makes the board in this workspace.
    #[must_use]
    pub const fn workspace(self, workspace: &'a str) -> Self {
        Self {
            workspace: Some(workspace),
            ..self
        }
    }

    /// Makes the board in this folder.
    #[must_use]
    pub const fn folder(self, folder: &'a str) -> Self {
        Self {
            folder: Some(folder),
            ..self
        }
    }

    /// Calls the new board this.
    #[must_use]
    pub const fn name(self, name: &'a str) -> Self {
        Self {
            name: Some(name),
            ..self
        }
    }

    /// Writes into this existing board.
    #[must_use]
    pub const fn into(self, board: &'a str) -> Self {
        Self {
            into: Some(board),
            ..self
        }
    }

    /// Keeps what the board holds.
    #[must_use]
    pub const fn merge(self) -> Self {
        Self {
            merge: true,
            ..self
        }
    }
}

impl On<'_> {
    /// Saves the board, or the part `look` names, as a folder.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn save_board(
        &self,
        board: &str,
        look: &Look<'_>,
    ) -> Result<Observation, ScenarioError> {
        let saved = self.holst.path("saved");
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(
                    self.holst.command("holst")?,
                    ["board".to_owned(), "save".to_owned(), board.to_owned()]
                        .into_iter()
                        .chain([
                            "--dir".to_owned(),
                            saved.to_string_lossy().into_owned(),
                        ])
                        .chain(look.arguments().0),
                    |stdout| {
                        #[derive(Deserialize)]
                        struct Saved {
                            not_saved: Vec<String>,
                        }
                        Ok(Observation::Saved {
                            not_saved: serde_json::from_slice::<Saved>(stdout)?
                                .not_saved,
                        })
                    },
                )
                .await?
            }
            Surface::Mcp => {
                let (_, mut arguments) = look.arguments();
                arguments.insert("action".to_owned(), Value::from("save"));
                arguments.insert("board".to_owned(), Value::from(board));
                arguments
                    .insert("dir".to_owned(), Value::from(saved.to_string_lossy()));
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board_write",
                    arguments,
                    |reply| Observation::Saved {
                        not_saved: reply
                            .text
                            .lines()
                            .filter(|line| line.starts_with("not saved:"))
                            .map(str::to_owned)
                            .collect(),
                    },
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    /// Exports the board to a new file called `file` in the folder the
    /// binaries run from.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn export_board(
        &self,
        board: &str,
        file: &str,
        how: Export,
    ) -> Result<Observation, ScenarioError> {
        let path = self.holst.work_file(file);
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(
                    self.holst.command("holst")?,
                    [
                        "board".as_ref(),
                        "export".as_ref(),
                        board.as_ref(),
                        path.as_os_str(),
                    ]
                    .into_iter()
                    .map(std::ffi::OsString::from)
                    .chain(how.uncompressed.then(|| "--uncompressed".into()))
                    .chain(how.object_list.then(|| "--object-list".into())),
                    |_| Ok(Observation::Done),
                )
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board_write",
                    Map::from_iter([
                        ("action".to_owned(), Value::from("export")),
                        ("board".to_owned(), Value::from(board)),
                        ("file".to_owned(), Value::from(path.to_string_lossy())),
                        ("uncompressed".to_owned(), Value::from(how.uncompressed)),
                        ("object_list".to_owned(), Value::from(how.object_list)),
                    ]),
                    |_| Observation::Done,
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    /// Imports the board file called `file`, in the folder the binaries run
    /// from; a write into a board needs the confirmation.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn import_board(
        &self,
        file: &str,
        how: Import<'_>,
    ) -> Result<Observation, ScenarioError> {
        let path = self.holst.work_file(file);
        let confirmed = how.into.is_some() && self.mode == SendMode::Confirmed;
        let dry = self.mode == SendMode::DryRun;
        let observation = match self.surface {
            Surface::Cli => {
                let line = [
                    ("-w", how.workspace),
                    ("--folder", how.folder),
                    ("--name", how.name),
                    ("--into", how.into),
                ]
                .into_iter()
                .filter_map(|(flag, value)| {
                    value.map(|value| [flag.to_owned(), value.to_owned()])
                })
                .flatten()
                .chain(how.merge.then(|| "--merge".to_owned()))
                .chain(confirmed.then(|| "-y".to_owned()))
                .chain(dry.then(|| "-n".to_owned()));
                cli::run(
                    self.holst.command("holst")?,
                    [
                        "board".to_owned(),
                        "import".to_owned(),
                        path.to_string_lossy().into_owned(),
                    ]
                    .into_iter()
                    .chain(line),
                    |stdout| Ok(self.observed(&String::from_utf8_lossy(stdout))),
                )
                .await?
            }
            Surface::Mcp => {
                let arguments = [
                    ("workspace", how.workspace),
                    ("folder", how.folder),
                    ("name", how.name),
                    ("into", how.into),
                ]
                .into_iter()
                .filter_map(|(name, value)| {
                    value.map(|value| (name.to_owned(), Value::from(value)))
                })
                .chain([
                    ("action".to_owned(), Value::from("import")),
                    ("file".to_owned(), Value::from(path.to_string_lossy())),
                    ("merge".to_owned(), Value::from(how.merge)),
                    ("dry_run".to_owned(), Value::from(dry)),
                    ("confirm".to_owned(), Value::from(confirmed)),
                ])
                .collect();
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board_write",
                    arguments,
                    |reply| self.observed(&reply.text),
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

/// Expects things of the file `name` a board was exported to.
#[must_use]
pub fn exported_file(holst: &Holst, name: &str) -> FileThen {
    FileThen {
        bytes: std::fs::read(holst.work_file(name)).ok(),
    }
}

#[derive(Debug)]
pub struct FileThen {
    bytes: Option<Vec<u8>>,
}

impl FileThen {
    /// Expects a file that is gzip, or is not.
    ///
    /// # Panics
    /// When it is not there, or the other.
    pub fn is_gzip(&self, gzip: bool) {
        assert_eq!(
            self.bytes
                .as_deref()
                .map(|bytes| bytes.starts_with(&[0x1f, 0x8b])),
            Some(gzip),
            "expected a file that is gzip: {gzip}"
        );
    }

    /// Expects the lines of the file to be JSON with these `(kind, text)`
    /// pairs of objects, in this order, each with an id.
    ///
    /// # Panics
    /// When the file differs.
    pub fn lists_objects(&self, expected: &[(&str, &str)]) {
        let text = String::from_utf8_lossy(self.bytes.as_deref().unwrap_or_default())
            .into_owned();
        let listed: Vec<Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert!(
            listed.iter().all(|object| object.get("id").is_some()),
            "an object without an id: {text}"
        );
        let kinds: Vec<(String, String)> = listed
            .iter()
            .map(|object| {
                let field = |name: &str| {
                    object
                        .get(name)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                (field("kind"), field("text"))
            })
            .collect();
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(kind, words)| ((*kind).to_owned(), (*words).to_owned()))
            .collect();
        assert_eq!(kinds, expected, "{text}");
    }
}
