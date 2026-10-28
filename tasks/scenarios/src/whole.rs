//! Showing and drawing a board or a part of it, on either surface.

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU32, Ordering},
};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    cli,
    error::ScenarioError,
    holst::{Holst, On, SendMode},
    mcp,
    observation::{ErrorExit, Observation, PictureFormat},
    surface::{Surface, Words, arguments},
    then::Then,
};

/// What narrows a read of a board and how a picture is drawn, as the flags say.
#[derive(Debug, Clone, Default)]
pub struct Look<'a> {
    with: Option<&'a str>,
    note_format: Option<&'a str>,
    frame: Option<&'a str>,
    rectangle: Option<&'a str>,
    objects: Vec<&'a str>,
    padding: Option<f64>,
    refresh: bool,
    offline: bool,
    scale: Option<f64>,
    max_side: Option<u32>,
    embed: bool,
}

impl<'a> Look<'a> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds these parts of the board, comma separated.
    #[must_use]
    pub fn with(self, parts: &'a str) -> Self {
        Self {
            with: Some(parts),
            ..self
        }
    }

    /// Shows the note, with `--with note`, as `format`: plain or markdown.
    #[must_use]
    pub fn note_format(self, format: &'a str) -> Self {
        Self {
            note_format: Some(format),
            ..self
        }
    }

    /// Takes the part of the board that this frame is.
    #[must_use]
    pub fn frame(self, frame: &'a str) -> Self {
        Self {
            frame: Some(frame),
            ..self
        }
    }

    /// Takes the part inside `x,y,width,height`.
    #[must_use]
    pub fn rectangle(self, rectangle: &'a str) -> Self {
        Self {
            rectangle: Some(rectangle),
            ..self
        }
    }

    /// Takes the part that holds this object too.
    #[must_use]
    pub fn object(self, object: &'a str) -> Self {
        Self {
            objects: self.objects.into_iter().chain([object]).collect(),
            ..self
        }
    }

    /// Pads the part by this much, in board units.
    #[must_use]
    pub fn padding(self, padding: f64) -> Self {
        Self {
            padding: Some(padding),
            ..self
        }
    }

    /// Reads the board again instead of the copy.
    #[must_use]
    pub fn refresh(self) -> Self {
        Self {
            refresh: true,
            ..self
        }
    }

    /// Reads the copy and asks for nothing, which only the CLI can say.
    #[must_use]
    pub fn offline(self) -> Self {
        Self {
            offline: true,
            ..self
        }
    }

    /// Draws this many pixels to a board unit.
    #[must_use]
    pub fn scale(self, scale: f64) -> Self {
        Self {
            scale: Some(scale),
            ..self
        }
    }

    /// Draws no side longer than this.
    #[must_use]
    pub fn max_side(self, pixels: u32) -> Self {
        Self {
            max_side: Some(pixels),
            ..self
        }
    }

    /// Puts the images into an SVG, which only the CLI can say.
    #[must_use]
    pub fn embed(self) -> Self {
        Self {
            embed: true,
            ..self
        }
    }

    /// Returns the flags of `holst` and the fields of the tool.
    pub(crate) fn arguments(&self) -> (Vec<String>, Map<String, Value>) {
        let padding = self.padding.map(|padding| padding.to_string());
        let scale = self.scale.map(|scale| scale.to_string());
        let max_side = self.max_side.map(|side| side.to_string());
        let words = Words {
            options: [
                ("with", self.with),
                ("note_format", self.note_format),
                ("frame", self.frame),
                ("rectangle", self.rectangle),
                ("padding", padding.as_deref()),
                ("scale", scale.as_deref()),
                ("max-side", max_side.as_deref()),
            ]
            .into_iter()
            .chain(self.objects.iter().map(|object| ("object", Some(*object))))
            .filter_map(|(name, value)| Some((name, value?)))
            .collect(),
            switches: self.refresh.then_some("refresh").into_iter().collect(),
            ..Words::default()
        };
        let (line, mut tool) = arguments(&words, SendMode::Send);
        let line = line
            .into_iter()
            .chain(self.offline.then(|| "--offline".to_owned()))
            .chain(self.embed.then(|| "--embed".to_owned()))
            .collect();
        // the tool takes lists and numbers, and spells the side with an underscore
        tool.remove("max-side");
        tool.extend(
            [
                (
                    "with",
                    self.with
                        .map(|parts| Value::from(parts.split(',').collect::<Vec<_>>())),
                ),
                (
                    "object",
                    (!self.objects.is_empty())
                        .then(|| Value::from(self.objects.clone())),
                ),
                ("padding", self.padding.map(Value::from)),
                ("scale", self.scale.map(Value::from)),
                ("max_side", self.max_side.map(Value::from)),
            ]
            .into_iter()
            .filter_map(|(name, value)| Some((name.to_owned(), value?))),
        );
        (line, tool)
    }
}

/// A board as `show` told of it, the same on both surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownBoard {
    pub name: String,
    /// The workspace, then the folder if any, by id as the surface shows them.
    pub place: String,
    pub count: usize,
    pub kinds: BTreeMap<String, usize>,
    /// The parts asked for with `--with`, each as its JSON.
    pub parts: BTreeMap<String, String>,
}

static NO_BOARD: ShownBoard = ShownBoard {
    name: String::new(),
    place: String::new(),
    count: 0,
    kinds: BTreeMap::new(),
    parts: BTreeMap::new(),
};

#[derive(Deserialize)]
struct Shown {
    boards: Vec<ShownOnCli>,
    missing: Vec<Value>,
}

#[derive(Deserialize)]
struct ShownOnCli {
    name: String,
    workspace: Option<String>,
    folder: Option<String>,
    count: usize,
    objects: BTreeMap<String, usize>,
    members: Option<Value>,
    rights: Option<Value>,
    sharing: Option<Value>,
    note: Option<Value>,
}

impl From<ShownOnCli> for ShownBoard {
    fn from(board: ShownOnCli) -> Self {
        Self {
            name: board.name,
            place: [board.workspace, board.folder]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" / "),
            count: board.count,
            kinds: board.objects,
            parts: [
                ("members", board.members),
                ("rights", board.rights),
                ("sharing", board.sharing),
                ("note", board.note),
            ]
            .into_iter()
            .filter_map(|(name, part)| {
                part.map(|part| (name.to_owned(), part.to_string()))
            })
            .collect(),
        }
    }
}

/// Reads `Name @N · right · place`, the link, `N objects: kind n, ...` and the
/// parts, one block of the text of `show`.
fn board_of_block(block: &str) -> Option<ShownBoard> {
    let lines: Vec<&str> = block.lines().collect();
    lines.get(1).filter(|link| link.starts_with("http"))?;
    let (head, rest) = lines.first()?.split_once(" · ")?;
    let (name, _alias) = head.rsplit_once(' ')?;
    let place = rest.split_once(" · ")?.1.split(" · ").next()?;
    let (count, kinds) = lines.iter().find_map(|line| {
        let (count, kinds) = line.split_once(" objects")?;
        Some((
            count.parse().ok()?,
            kinds
                .strip_prefix(": ")
                .unwrap_or_default()
                .split(", ")
                .filter_map(|kind| {
                    let (kind, number) = kind.rsplit_once(' ')?;
                    Some((kind.to_owned(), number.parse().ok()?))
                })
                .collect(),
        ))
    })?;
    Some(ShownBoard {
        name: name.to_owned(),
        place: place.to_owned(),
        count,
        kinds,
        parts: ["members", "rights", "sharing", "note"]
            .into_iter()
            .filter_map(|part| {
                lines.iter().find_map(|line| {
                    line.strip_prefix(&format!("{part}: "))
                        .map(|json| (part.to_owned(), json.to_owned()))
                })
            })
            .collect(),
    })
}

fn boards_of_text(text: &str) -> Observation {
    Observation::Boards {
        shown: text.split("\n\n").filter_map(board_of_block).collect(),
        missing: text
            .lines()
            .filter(|line| line.starts_with("not found:"))
            .count(),
        exit: None,
    }
}

#[derive(Deserialize)]
struct Drawn {
    width: u32,
    height: u32,
}

/// Reads the `WxH px` of the words about a picture.
fn size_of_text(text: &str) -> Option<Observation> {
    let (width, height) = text
        .split(" · ")
        .find_map(|part| part.strip_suffix(" px"))?
        .split_once('x')?;
    Some(Observation::Size {
        width: width.parse().ok()?,
        height: height.parse().ok()?,
    })
}

impl On<'_> {
    /// Shows these boards, narrowed by `look`; the CLI says what it found even
    /// when it ends as not found.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn show_boards(
        &self,
        boards: &[&str],
        look: &Look<'_>,
    ) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                let output = cli::output(
                    self.holst.command("holst")?,
                    ["board", "show"]
                        .into_iter()
                        .map(str::to_owned)
                        .chain(boards.iter().map(|board| (*board).to_owned()))
                        .chain(look.arguments().0),
                    "",
                )
                .await?;
                match serde_json::from_slice::<Shown>(&output.stdout) {
                    Ok(shown) => Observation::Boards {
                        shown: shown.boards.into_iter().map(ShownBoard::from).collect(),
                        missing: shown.missing.len(),
                        exit: cli::failure(&output).and_then(|failed| {
                            if let Observation::Failed { signal, .. } = failed {
                                Some(signal)
                            } else {
                                None
                            }
                        }),
                    },
                    Err(error) => {
                        cli::failure(&output).map_or_else(|| Err(error), Ok)?
                    }
                }
            }
            Surface::Mcp => {
                let board = match boards {
                    [one] => Value::from(*one),
                    many => Value::from(many.to_vec()),
                };
                let (_, mut arguments) = look.arguments();
                arguments.insert("action".to_owned(), Value::from("show"));
                arguments.insert("board".to_owned(), board);
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board",
                    arguments,
                    |reply| boards_of_text(&reply.text),
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }

    /// Draws the board, narrowed by `look`, and tells the size of the picture.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn drawn_size(
        &self,
        board: &str,
        look: &Look<'_>,
        picture: PictureFormat,
    ) -> Result<Observation, ScenarioError> {
        static DRAWN: AtomicU32 = AtomicU32::new(0);
        let observation = match self.surface {
            Surface::Cli => {
                let file = self.holst.path("snapshots").join(format!(
                    "drawn-{}.{}",
                    DRAWN.fetch_add(1, Ordering::Relaxed),
                    picture.extension()
                ));
                cli::run(
                    self.holst.command("holst")?,
                    ["board".to_owned(), "snap".to_owned(), board.to_owned()]
                        .into_iter()
                        .chain([file.to_string_lossy().into_owned()])
                        .chain(look.arguments().0),
                    |stdout| {
                        let drawn = serde_json::from_slice::<Drawn>(stdout)?;
                        Ok(Observation::Size {
                            width: drawn.width,
                            height: drawn.height,
                        })
                    },
                )
                .await?
            }
            Surface::Mcp => {
                let (_, mut arguments) = look.arguments();
                arguments.insert("action".to_owned(), Value::from("snap"));
                arguments.insert("board".to_owned(), Value::from(board));
                arguments.insert("format".to_owned(), Value::from(picture.extension()));
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "board",
                    arguments,
                    |reply| size_of_text(&reply.text).unwrap_or(Observation::Done),
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

impl Holst {
    /// Draws the board on the CLI with no file, which goes to the downloads.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn snap_to_default(
        &self,
        board: &str,
    ) -> Result<Observation, ScenarioError> {
        let observation =
            cli::run(self.command("holst")?, ["board", "snap", board], |_| {
                Ok(Observation::Done)
            })
            .await?;
        self.checked(observation)
    }

    /// Draws the board on the CLI to its stdout.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn snap_to_stdout(
        &self,
        board: &str,
    ) -> Result<Observation, ScenarioError> {
        let observation = cli::run(
            self.command("holst")?,
            ["board", "snap", board, "-"],
            |stdout| Ok(Observation::Picture(PictureFormat::of(stdout))),
        )
        .await?;
        self.checked(observation)
    }

    /// Returns the link to the `number`th object of the fixture on the board.
    ///
    /// # Errors
    /// [`ScenarioError::NoBoard`] when the fake holds no such board.
    pub fn object_link(
        &self,
        board: &str,
        number: u32,
    ) -> Result<String, ScenarioError> {
        let id = self.fixture_object(board, number)?;
        Ok(format!("{}?objectId={id}", self.board_link(board)?))
    }
}

/// Expects things of what the downloads folder of the home holds.
#[must_use]
pub fn downloads_of(holst: &Holst) -> DownloadsThen {
    DownloadsThen {
        names: std::fs::read_dir(holst.path("home").join("Downloads"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
    }
}

#[derive(Debug)]
pub struct DownloadsThen {
    names: Vec<String>,
}

impl DownloadsThen {
    /// Expects a PNG named `name-<short id>.png`.
    ///
    /// # Panics
    /// When there is none.
    pub fn holds_a_png_of(&self, name: &str) {
        assert!(
            self.names.iter().any(|file| {
                file.starts_with(&format!("{name}-"))
                    && std::path::Path::new(file)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
            }),
            "expected a PNG of {name}, found {:?}",
            self.names
        );
    }
}

/// Expects things of the picture file `name` a snap left.
#[must_use]
pub fn picture(holst: &Holst, name: &str) -> PictureThen {
    PictureThen {
        bytes: std::fs::read(holst.path("snapshots").join(name)).ok(),
    }
}

#[derive(Debug, Clone)]
pub struct PictureThen {
    bytes: Option<Vec<u8>>,
}

impl PictureThen {
    /// Expects the file to hold what `other` held.
    ///
    /// # Panics
    /// When it differs.
    pub fn is_unchanged_since(&self, other: &Self) {
        assert!(self.bytes.is_some());
        assert_eq!(self.bytes, other.bytes);
    }

    /// Expects an SVG that holds its images as data.
    ///
    /// # Panics
    /// When it does not.
    pub fn embeds_its_images(&self) {
        assert!(self.text().contains("data:image"), "{}", self.text());
    }

    /// Expects an SVG that links its images.
    ///
    /// # Panics
    /// When it does not.
    pub fn links_its_images(&self) {
        let text = self.text();
        assert!(
            text.contains("<image") && !text.contains("data:image"),
            "{text}"
        );
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(self.bytes.as_deref().unwrap_or_default()).into_owned()
    }
}

impl Then<'_> {
    fn boards(&self) -> &[ShownBoard] {
        assert!(
            matches!(self.observation, Observation::Boards { .. }),
            "expected shown boards, got {:?}",
            self.observation
        );
        if let Observation::Boards { shown, .. } = self.observation {
            shown
        } else {
            &[]
        }
    }

    fn only_board(&self) -> &ShownBoard {
        assert_eq!(self.boards().len(), 1, "expected one shown board");
        self.boards().first().unwrap_or(&NO_BOARD)
    }

    /// Expects these boards shown, in this order, and none missing.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_boards(&self, names: &[&str]) {
        let shown: Vec<&str> = self
            .boards()
            .iter()
            .map(|board| board.name.as_str())
            .collect();
        assert_eq!(shown, names);
        self.reports_missing(0, None);
    }

    /// Expects the boards shown to be these, `missing` references not found,
    /// and the command line to end as `exit`.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn reports_missing(&self, missing: usize, exit: Option<ErrorExit>) {
        assert!(
            matches!(self.observation, Observation::Boards { missing: found, exit: ended, .. }
                if *found == missing && *ended == exit),
            "expected {missing} missing, ending as {exit:?}, got {:?}",
            self.observation
        );
    }

    /// Expects the boards shown to be these, in this order, whatever else the
    /// command told.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_only(&self, names: &[&str]) {
        let shown: Vec<&str> = self
            .boards()
            .iter()
            .map(|board| board.name.as_str())
            .collect();
        assert_eq!(shown, names);
    }

    /// Expects one board shown at the root of its workspace. The place is
    /// told by ids, which the tool turns into aliases, so only its depth is
    /// compared.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_it_at_the_root(&self) {
        assert_eq!(self.only_board().place.split(" / ").count(), 1);
    }

    /// Expects one board shown inside a folder.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_it_in_a_folder(&self) {
        assert_eq!(self.only_board().place.split(" / ").count(), 2);
    }

    /// Expects one board shown, with these counts of objects by kind.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_objects(&self, kinds: &[(&str, usize)]) {
        let board = self.only_board();
        let expected: BTreeMap<String, usize> = kinds
            .iter()
            .map(|(kind, count)| ((*kind).to_owned(), *count))
            .collect();
        assert_eq!(board.kinds, expected);
        assert_eq!(board.count, expected.values().sum::<usize>());
    }

    /// Expects one board shown with the part `part`, which holds `text`.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_part(&self, part: &str, text: &str) {
        let board = self.only_board();
        assert!(
            board
                .parts
                .get(part)
                .is_some_and(|json| json.contains(text)),
            "expected {part} to hold {text:?}, got {:?}",
            board.parts
        );
    }

    /// Expects one board shown with sharing settings that hold every field of
    /// `expected`.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_sharing(&self, expected: &Value) {
        let board = self.only_board();
        let shown: Value = board
            .parts
            .get("sharing")
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        assert!(!shown.is_null(), "expected sharing, got {:?}", board.parts);
        expected
            .as_object()
            .into_iter()
            .flatten()
            .for_each(|(field, value)| {
                assert_eq!(
                    shown.get(field).unwrap_or(&Value::Null),
                    value,
                    "the field {field} of {shown}"
                );
            });
    }

    /// Expects one board shown with no part beyond its objects.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn shows_no_part(&self) {
        assert!(self.only_board().parts.is_empty(), "{:?}", self.boards());
    }

    fn size(&self) -> (u32, u32) {
        assert!(
            matches!(self.observation, Observation::Size { .. }),
            "expected the size of a picture, got {:?}",
            self.observation
        );
        if let Observation::Size { width, height } = self.observation {
            (*width, *height)
        } else {
            (0, 0)
        }
    }

    /// Expects a picture smaller than `other`.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_smaller_than(&self, other: &Observation) {
        let (width, height) = self.size();
        let (other_width, other_height) = Then { observation: other }.size();
        assert!(
            width * height < other_width * other_height,
            "{width}x{height} is not smaller than {other_width}x{other_height}"
        );
    }

    /// Expects a picture `factor` times as wide as `other`.
    ///
    /// # Panics
    /// When it is not.
    pub fn is_as_wide_as(&self, other: &Observation, factor: u32) {
        let (width, _) = self.size();
        let (other_width, _) = Then { observation: other }.size();
        assert_eq!(width, other_width * factor);
    }

    /// Expects a picture none of whose sides is longer than `pixels`.
    ///
    /// # Panics
    /// When it is.
    pub fn fits_in(&self, pixels: u32) {
        let (width, height) = self.size();
        assert!(
            width.max(height) <= pixels,
            "{width}x{height} is longer than {pixels}"
        );
    }
}
