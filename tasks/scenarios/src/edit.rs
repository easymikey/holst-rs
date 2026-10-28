//! Calls of the `object` command on either surface, and the board behind them.

use serde_json::{Map, Value};

use crate::{
    cli,
    error::ScenarioError,
    holst::{Holst, On, SendMode},
    mcp,
    observation::Observation,
    read::{cli_objects, mcp_objects},
    surface::{Surface, Words, arguments},
};

/// Flags the command line repeats and the tool takes as an array.
const REPEATED: [&str; 3] = ["object", "tag", "label"];
/// Flags that hold a count, a number for the tool.
const COUNTS: [&str; 6] = ["offset", "limit", "padding", "radius", "check", "uncheck"];
/// Flags that hold a fraction, a number for the tool.
const FRACTIONS: [&str; 3] = ["opacity", "stroke_width", "rotate"];

/// How `--items` reaches the command line; the tool always gets an array.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ItemsFile {
    /// A file holding a JSON array.
    #[default]
    Array,
    /// A file holding one JSON value per line.
    Lines,
    /// Standard input, as `--items -`.
    Stdin,
}

/// What a user asks of `object`: the flags of the CLI, the fields of the tool.
#[derive(Debug, Clone)]
pub struct ObjectCall {
    verb: &'static str,
    kind: Option<&'static str>,
    /// The one positional argument after the board, by the field it fills.
    argument: Option<(&'static str, String)>,
    flags: Vec<(&'static str, String)>,
    switches: Vec<&'static str>,
    items: Option<Vec<Value>>,
    items_file: ItemsFile,
}

impl ObjectCall {
    const fn new(verb: &'static str, kind: Option<&'static str>) -> Self {
        Self {
            verb,
            kind,
            argument: None,
            flags: Vec::new(),
            switches: Vec::new(),
            items: None,
            items_file: ItemsFile::Array,
        }
    }

    /// Lists the objects of a board.
    #[must_use]
    pub const fn list() -> Self {
        Self::new("ls", None)
    }

    /// Adds objects of this kind: sticker, text, shape, frame, arrow, link,
    /// image or file.
    #[must_use]
    pub const fn add(kind: &'static str) -> Self {
        Self::new("add", Some(kind))
    }

    /// Changes objects.
    #[must_use]
    pub const fn edit() -> Self {
        Self::new("edit", None)
    }

    /// Copies objects.
    #[must_use]
    pub const fn copy() -> Self {
        Self::new("cp", None)
    }

    /// Puts objects into a new group.
    #[must_use]
    pub const fn group() -> Self {
        Self::new("group", None)
    }

    /// Dissolves groups.
    #[must_use]
    pub const fn ungroup() -> Self {
        Self::new("ungroup", None)
    }

    /// Locks objects.
    #[must_use]
    pub const fn lock() -> Self {
        Self::new("lock", None)
    }

    /// Unlocks objects.
    #[must_use]
    pub const fn unlock() -> Self {
        Self::new("unlock", None)
    }

    /// Aligns objects to a side or spreads them.
    #[must_use]
    pub const fn align() -> Self {
        Self::new("align", None)
    }

    /// Removes objects.
    #[must_use]
    pub const fn remove() -> Self {
        Self::new("rm", None)
    }

    /// Downloads the originals of image and file objects.
    #[must_use]
    pub const fn get() -> Self {
        Self::new("get", None)
    }

    /// Gives the text of the object to add.
    #[must_use]
    pub fn text(self, text: &str) -> Self {
        Self {
            argument: Some(("text", text.to_owned())),
            ..self
        }
    }

    /// Gives the address of the page the link to add points to.
    #[must_use]
    pub fn url(self, url: &str) -> Self {
        Self {
            argument: Some(("url", url.to_owned())),
            ..self
        }
    }

    /// Gives the local file the image or file to add is read from.
    #[must_use]
    pub fn path(self, path: &str) -> Self {
        Self {
            argument: Some(("path", path.to_owned())),
            ..self
        }
    }

    /// Gives the file to download to.
    #[must_use]
    pub fn file(self, file: &str) -> Self {
        Self {
            argument: Some(("file", file.to_owned())),
            ..self
        }
    }

    /// Sets the flag `name` (the tool's field of the same name); `object`,
    /// `tag` and `label` can be set again and again.
    #[must_use]
    pub fn with(self, name: &'static str, value: impl Into<String>) -> Self {
        Self {
            flags: self
                .flags
                .into_iter()
                .chain([(name, value.into())])
                .collect(),
            ..self
        }
    }

    /// Turns the flag `name` on.
    #[must_use]
    pub fn switched(self, name: &'static str) -> Self {
        Self {
            switches: self.switches.into_iter().chain([name]).collect(),
            ..self
        }
    }

    /// Gives many objects in one call.
    #[must_use]
    pub fn items(self, items: Vec<Value>) -> Self {
        Self {
            items: Some(items),
            ..self
        }
    }

    /// Chooses how `--items` reaches the command line.
    #[must_use]
    pub fn items_via(self, items_file: ItemsFile) -> Self {
        Self { items_file, ..self }
    }

    fn lists(&self) -> bool {
        self.verb == "ls"
    }

    /// Returns what goes on the standard input.
    fn input(&self) -> String {
        match (&self.items, self.items_file) {
            (Some(items), ItemsFile::Stdin) => Value::Array(items.clone()).to_string(),
            _ => String::new(),
        }
    }

    /// Returns the text of the items file, when the command line names one.
    fn items_text(&self) -> Option<String> {
        self.items.as_ref().and_then(|items| match self.items_file {
            ItemsFile::Array => Some(Value::Array(items.clone()).to_string()),
            ItemsFile::Lines => Some(
                items
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            ItemsFile::Stdin => None,
        })
    }

    /// Returns the arguments of `holst`, after `-o json`, and the fields of
    /// the `object` or `object_write` tool; `items_path` is where the items
    /// file is.
    fn arguments(
        &self,
        board: &str,
        mode: SendMode,
        items_path: Option<&str>,
    ) -> (Vec<String>, Map<String, Value>) {
        let words = Words {
            positional: self
                .kind
                .map(|kind| ("kind", kind))
                .into_iter()
                .chain([("board", board)])
                .chain(
                    self.argument
                        .as_ref()
                        .map(|(name, value)| (*name, value.as_str())),
                )
                .collect(),
            options: self
                .flags
                .iter()
                .map(|(name, value)| (*name, value.as_str()))
                .chain(
                    self.items
                        .is_some()
                        .then(|| ("items", items_path.unwrap_or("-"))),
                )
                .collect(),
            switches: self.switches.clone(),
            confirms: self.verb == "rm",
            writes: !self.lists(),
        };
        let (line, mut tool) = arguments(&words, mode);
        // the tool takes counts as numbers, repeated flags and items as arrays
        let values = |name: &str| -> Vec<&str> {
            self.flags
                .iter()
                .filter(|(flag, _)| *flag == name)
                .map(|(_, value)| value.as_str())
                .collect()
        };
        let counts = COUNTS.iter().filter_map(|name| {
            let count = values(name).last()?.parse::<u64>().ok()?;
            Some(((*name).to_owned(), Value::from(count)))
        });
        let fractions = FRACTIONS.iter().filter_map(|name| {
            let fraction = values(name).last()?.parse::<f64>().ok()?;
            Some(((*name).to_owned(), Value::from(fraction)))
        });
        let arrays = REPEATED.iter().filter_map(|name| {
            let found = values(name);
            (!found.is_empty()).then(|| ((*name).to_owned(), Value::from(found)))
        });
        tool.extend(counts.chain(fractions).chain(arrays));
        if let Some(items) = &self.items {
            tool.insert("items".to_owned(), Value::Array(items.clone()));
        }
        if self.lists() {
            // a listing shows all unless asked for a page
            tool.entry("limit").or_insert_with(|| Value::from(0));
        } else {
            tool.insert("action".to_owned(), Value::from(self.verb));
        }
        (
            ["object", self.verb]
                .map(str::to_owned)
                .into_iter()
                .chain(line)
                .collect(),
            tool,
        )
    }
}

impl Holst {
    /// Returns the id the fake gave the `number`th object of the fixture on
    /// `board`, counting from 1.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds no such board.
    pub fn fixture_object(
        &self,
        board: &str,
        number: u32,
    ) -> Result<String, ScenarioError> {
        self.fake
            .object_id(board, number)
            .ok_or_else(|| ScenarioError::NoBoard {
                name: board.to_owned(),
            })
    }

    /// Returns the first object of this `kind` (the type in the board's
    /// document, as `image`) that the fake holds on `board`, as the document
    /// keeps it.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds no such board,
    /// [`ScenarioError::NoObjectOfKind`] when it holds no such object on it.
    pub fn first_of_kind(
        &self,
        board: &str,
        kind: &str,
    ) -> Result<Value, ScenarioError> {
        self.board_id(board)?;
        self.fake
            .objects_of(board)
            .into_iter()
            .find(|object| object.get("type").and_then(Value::as_str) == Some(kind))
            .ok_or_else(|| ScenarioError::NoObjectOfKind {
                board: board.to_owned(),
                kind: kind.to_owned(),
            })
    }

    /// Adds objects to `board` as another client would, so what a surface
    /// cached of the board is stale.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds no such board.
    pub fn edit_board_behind_its_back(
        &self,
        board: &str,
        objects: Vec<crate::FixtureObject>,
    ) -> Result<(), ScenarioError> {
        self.fake
            .edit_board(board, objects)
            .map_err(|_| ScenarioError::NoBoard {
                name: board.to_owned(),
            })
    }

    /// Returns where a file called `name` is in the folder the binaries run
    /// from, whether it is there or not.
    #[must_use]
    pub fn work_place(&self, name: &str) -> String {
        self.work_file(name).to_string_lossy().into_owned()
    }

    /// Returns the stacking order (`zIndex`) of the `number`th object of the
    /// fixture on `board`, counting from 1; a higher one is above.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds no such board.
    pub fn stacking_of(
        &self,
        board: &str,
        number: u32,
    ) -> Result<Option<f64>, ScenarioError> {
        let id = self.fixture_object(board, number)?;
        Ok(self
            .fake
            .objects_of(board)
            .iter()
            .find(|object| {
                object.get("id").and_then(Value::as_str) == Some(id.as_str())
            })
            .and_then(|object| object.get("zIndex").and_then(Value::as_f64)))
    }
}

impl On<'_> {
    /// Asks `object` for something about `board`; a listing is read, the rest
    /// is `Done`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn object(
        &self,
        board: &str,
        call: ObjectCall,
    ) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                let items_path = call
                    .items_text()
                    .map(|text| {
                        let path = self.holst.work_file("items.json");
                        std::fs::write(&path, text).map(|()| path)
                    })
                    .transpose()?
                    .map(|path| path.to_string_lossy().into_owned());
                let output = cli::output(
                    self.holst.command("holst")?,
                    call.arguments(board, self.mode, items_path.as_deref()).0,
                    &call.input(),
                )
                .await?;
                match cli::failure(&output) {
                    Some(failure) => failure,
                    None if call.lists() => cli_objects(&output.stdout)?,
                    None => Observation::Done,
                }
            }
            Surface::Mcp => {
                let listing = call.lists();
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    if listing { "object" } else { "object_write" },
                    call.arguments(board, self.mode, None).1,
                    |reply| {
                        if listing {
                            mcp_objects(&reply.text)
                        } else {
                            Observation::Done
                        }
                    },
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    /// Adds a sticker with `text` to `board`, inside the frame named `frame`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn add_sticker(
        &self,
        board: &str,
        text: &str,
        frame: Option<&str>,
    ) -> Result<Observation, ScenarioError> {
        let call = ObjectCall::add("sticker").text(text);
        self.object(
            board,
            frame
                .into_iter()
                .fold(call, |call, frame| call.with("frame", frame)),
        )
        .await
    }

    /// Moves the object with this id to the corner `x`, `y`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn move_object(
        &self,
        board: &str,
        object: &str,
        (x, y): (i32, i32),
    ) -> Result<Observation, ScenarioError> {
        self.object(
            board,
            ObjectCall::edit()
                .with("object", object)
                .with("at", format!("{x},{y}")),
        )
        .await
    }

    /// Replaces the text of the object with this id.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn change_object_text(
        &self,
        board: &str,
        object: &str,
        text: &str,
    ) -> Result<Observation, ScenarioError> {
        self.object(
            board,
            ObjectCall::edit().with("object", object).with("text", text),
        )
        .await
    }

    /// Removes the object with this id from the board.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn remove_object(
        &self,
        board: &str,
        object: &str,
    ) -> Result<Observation, ScenarioError> {
        self.object(board, ObjectCall::remove().with("object", object))
            .await
    }
}

/// Expects of what the fake received; what a board holds is read through
/// `object`.
#[must_use]
pub fn board_of(holst: &Holst) -> BoardThen {
    BoardThen {
        updates: holst.fake.updates_received(),
    }
}

#[derive(Debug)]
pub struct BoardThen {
    updates: usize,
}

impl BoardThen {
    /// Expects that the fake got no update of any board.
    ///
    /// # Panics
    /// When it got one.
    pub fn got_no_update(&self) {
        assert_eq!(
            self.updates, 0,
            "expected no update, the fake got {}",
            self.updates
        );
    }

    /// Expects that the fake got exactly one update of boards.
    ///
    /// # Panics
    /// When it got none or more.
    pub fn got_one_update(&self) {
        assert_eq!(
            self.updates, 1,
            "expected one update, the fake got {}",
            self.updates
        );
    }
}
