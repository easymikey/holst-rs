//! Reading a board and searching the text on boards, on either surface.

use holst_core::domain::board::shape_type::ShapeLook;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    cli,
    edit::ObjectCall,
    error::ScenarioError,
    holst::{On, SendMode},
    mcp,
    observation::{ListedMatch, ListedName, ListedObject, ListedTextHit, Observation},
    surface::{Surface, Words, arguments},
    then::Then,
};

#[derive(Deserialize)]
struct Listing {
    objects: Vec<Listed>,
}

#[derive(Deserialize)]
struct Listed {
    id: String,
    kind: String,
    text: String,
    x: f64,
    y: f64,
    width: Option<f64>,
    height: Option<f64>,
    color: Option<String>,
    look: Option<ShapeLook>,
    parent: Option<String>,
    ends: Option<Ends>,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    rotation: f64,
    #[serde(default)]
    link_to: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    marks: Vec<String>,
    text_color: Option<String>,
    text_background: Option<String>,
}

#[derive(Deserialize)]
struct Ends {
    start: Option<String>,
    end: Option<String>,
}

#[derive(Deserialize)]
struct Found {
    hits: Vec<FoundHit>,
}

#[derive(Deserialize)]
#[serde(tag = "in", rename_all = "snake_case")]
enum FoundHit {
    Name {
        name: String,
        kind: String,
        in_trash: bool,
    },
    Text {
        board_name: String,
        kind: String,
        excerpt: String,
    },
}

/// A search: the words, and what narrows it; `mode` is spelled as on MCP.
#[derive(Debug, Clone, Copy)]
pub struct Search<'a> {
    query: &'a str,
    mode: Option<&'a str>,
    workspace: Option<&'a str>,
    board: Option<&'a str>,
    kind: Option<&'a str>,
    trash: bool,
    limit: Option<u32>,
}

impl<'a> Search<'a> {
    #[must_use]
    pub const fn new(query: &'a str) -> Self {
        Self {
            query,
            mode: None,
            workspace: None,
            board: None,
            kind: None,
            trash: false,
            limit: None,
        }
    }

    /// Searches `names`, `board_text` or `both`.
    #[must_use]
    pub const fn mode(self, mode: &'a str) -> Self {
        Self {
            mode: Some(mode),
            ..self
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

    /// Reads the text of this board only.
    #[must_use]
    pub const fn board(self, board: &'a str) -> Self {
        Self {
            board: Some(board),
            ..self
        }
    }

    /// Looks only for this kind of board, folder or object.
    #[must_use]
    pub const fn kind(self, kind: &'a str) -> Self {
        Self {
            kind: Some(kind),
            ..self
        }
    }

    /// Looks in the trash too.
    #[must_use]
    pub const fn in_trash(self) -> Self {
        Self {
            trash: true,
            ..self
        }
    }

    /// Shows at most this many matches; `0` is all.
    #[must_use]
    pub const fn limit(self, limit: u32) -> Self {
        Self {
            limit: Some(limit),
            ..self
        }
    }

    /// Returns the command line of `holst` and the fields of the tool.
    fn arguments(self) -> (Vec<String>, Map<String, Value>) {
        let cli_mode = self.mode.map(|mode| mode.replace('_', "-"));
        let cli_limit = self.limit.map(|limit| limit.to_string());
        let words = Words {
            positional: vec![("query", self.query)],
            options: [
                ("mode", cli_mode.as_deref()),
                ("workspace", self.workspace),
                ("board", self.board),
                ("kind", self.kind),
                ("limit", cli_limit.as_deref()),
            ]
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?)))
            .collect(),
            switches: self.trash.then_some("trash").into_iter().collect(),
            ..Words::default()
        };
        let (line, mut tool) = arguments(&words, SendMode::Send);
        // the tool spells the mode with underscores and takes the limit as a number
        tool.extend(self.mode.map(|mode| ("mode".to_owned(), Value::from(mode))));
        tool.extend(
            self.limit
                .map(|limit| ("limit".to_owned(), Value::from(limit))),
        );
        (
            ["search".to_owned()].into_iter().chain(line).collect(),
            tool,
        )
    }
}

impl On<'_> {
    /// Lists the objects of the board with that name.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn objects(&self, board: &str) -> Result<Observation, ScenarioError> {
        self.object(board, ObjectCall::list()).await
    }

    /// Searches the boards by name and the text on them.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn search(
        &self,
        search: Search<'_>,
    ) -> Result<Observation, ScenarioError> {
        let observation = match self.surface {
            Surface::Cli => {
                cli::run(
                    self.holst.command("holst")?,
                    search.arguments().0,
                    |stdout| {
                        Ok(Observation::Matches(
                            serde_json::from_slice::<Found>(stdout)?
                                .hits
                                .into_iter()
                                .map(|hit| match hit {
                                    FoundHit::Name {
                                        name,
                                        kind,
                                        in_trash,
                                    } => ListedMatch::Name(ListedName {
                                        name,
                                        kind,
                                        in_trash,
                                    }),
                                    FoundHit::Text {
                                        board_name,
                                        kind,
                                        excerpt,
                                    } => ListedMatch::Text(ListedTextHit {
                                        board: board_name,
                                        kind,
                                        text: excerpt,
                                    }),
                                })
                                .collect(),
                        ))
                    },
                )
                .await?
            }
            Surface::Mcp => {
                mcp::call(
                    self.holst.command("holst-mcp")?,
                    "search",
                    search.arguments().1,
                    |reply| {
                        Observation::Matches(
                            reply.text.lines().filter_map(listed_match).collect(),
                        )
                    },
                )
                .await?
            }
        };
        self.holst.checked(observation)
    }
}

/// Reads `@N name[/] [· place] [(trash)]`, a name hit, or `@N kind in @M
/// Board: excerpt`, a text hit.
fn listed_match(line: &str) -> Option<ListedMatch> {
    let rest = line.strip_prefix('@')?.split_once(' ')?.1;
    if rest.contains(" in @") {
        listed_text_hit(line).map(ListedMatch::Text)
    } else {
        listed_name(rest)
    }
}

fn listed_name(rest: &str) -> Option<ListedMatch> {
    let (rest, in_trash) = rest
        .strip_suffix(" (trash)")
        .map_or((rest, false), |rest| (rest, true));
    let name = rest.split(" · ").next()?;
    Some(ListedMatch::Name(ListedName {
        kind: if name.ends_with('/') {
            "folder"
        } else {
            "board"
        }
        .to_owned(),
        name: name.trim_end_matches('/').to_owned(),
        in_trash,
    }))
}

/// Reads what `holst object ls -o json` printed.
pub(crate) fn cli_objects(stdout: &[u8]) -> Result<Observation, ScenarioError> {
    Ok(Observation::Objects(
        serde_json::from_slice::<Listing>(stdout)?
            .objects
            .into_iter()
            .map(|listed| ListedObject {
                id: listed.id,
                kind: listed.kind,
                text: listed.text,
                corner: format!(
                    "{:.0},{:.0}",
                    listed.x.round() + 0.0,
                    listed.y.round() + 0.0
                ),
                in_frame: listed.parent.is_some(),
                locked: listed.locked,
                rotation: (listed.rotation != 0.0).then(|| listed.rotation.to_string()),
                link_to: listed.link_to,
                size: listed
                    .width
                    .zip(listed.height)
                    .map(|(width, height)| format!("{width:.0}x{height:.0}")),
                color: listed.color,
                look: listed.look.map(|look| look.words()),
                tags: listed.tags,
                ends: listed.ends.map(|ends| (ends.start, ends.end)),
                text_look: listed
                    .marks
                    .into_iter()
                    .chain(listed.text_color.map(|color| format!("text:{color}")))
                    .chain(listed.text_background.map(|color| format!("bg:{color}")))
                    .collect(),
            })
            .collect(),
    ))
}

/// Reads what the `object` tool answered: a head and a line per object.
pub(crate) fn mcp_objects(text: &str) -> Observation {
    Observation::Objects(
        text.lines()
            .filter(|line| line.starts_with('@'))
            .filter_map(listed_object)
            .collect(),
    )
}

/// Reads `@N kind (x, y) [WxH] [in @M]  [color]  [A → B]  [#tag]  ["text"]`,
/// the way `object` shows an object.
fn listed_object(line: &str) -> Option<ListedObject> {
    let (head, text) = line.split_once("  \"").map_or((line, ""), |(head, text)| {
        (head, text.strip_suffix('"').unwrap_or(text))
    });
    let mut words = head.split_whitespace();
    let id = words.next()?.to_owned();
    let kind = words.next()?.to_owned();
    let corner = words
        .next()?
        .trim_start_matches('(')
        .trim_end_matches(',')
        .to_owned()
        + ","
        + words.next()?.trim_end_matches(')');
    let rest: Vec<&str> = words.collect();
    let parent = rest.iter().position(|word| *word == "in");
    let arrow = rest.iter().position(|word| *word == "→");
    // not the color: the parent with its `in`, and the ends with their arrow
    let spoken_for: Vec<usize> = parent
        .into_iter()
        .flat_map(|at| [at, at + 1])
        .chain(
            arrow
                .into_iter()
                .flat_map(|at| [at.saturating_sub(1), at, at + 1]),
        )
        .collect();
    let end = |word: Option<&&str>| {
        word.filter(|word| **word != "·")
            .map(|word| (*word).to_owned())
    };
    // the words after the color, up to the ends and tags: a shape's look
    let look = rest
        .iter()
        .enumerate()
        .filter(|(at, word)| {
            !spoken_for.contains(at)
                && !word.contains('×')
                && !word.starts_with(['#', '{'])
                && !word.ends_with('°')
                && !word.starts_with("link-to=")
        })
        .skip(1)
        .map(|(_, word)| *word)
        .collect::<Vec<_>>()
        .join(" ");
    Some(ListedObject {
        id,
        kind,
        corner,
        in_frame: parent.is_some(),
        locked: rest.contains(&"locked"),
        rotation: rest
            .iter()
            .find_map(|word| word.strip_suffix('°'))
            .map(str::to_owned),
        link_to: rest
            .iter()
            .find_map(|word| word.strip_prefix("link-to="))
            .map(str::to_owned),
        text: text.to_owned(),
        size: rest
            .iter()
            .find(|word| word.contains('×'))
            .map(|word| word.replace('×', "x")),
        color: rest
            .iter()
            .enumerate()
            .find(|(at, word)| {
                !spoken_for.contains(at)
                    && !word.contains('×')
                    && !word.starts_with(['#', '{'])
                    && **word != "locked"
                    && !word.ends_with('°')
                    && !word.starts_with("link-to=")
            })
            .map(|(_, word)| (*word).to_owned()),
        look: Some(look).filter(|look| !look.is_empty()),
        tags: rest
            .iter()
            .filter_map(|word| word.strip_prefix('#'))
            .map(str::to_owned)
            .collect(),
        text_look: rest
            .iter()
            .find_map(|word| word.strip_prefix('{')?.strip_suffix('}'))
            .into_iter()
            .flat_map(|inner| inner.split(','))
            .map(str::to_owned)
            .collect(),
        ends: arrow.map(|at| {
            (
                end(at.checked_sub(1).and_then(|before| rest.get(before))),
                end(rest.get(at + 1)),
            )
        }),
    })
}

/// Reads `@N kind in @M Board: excerpt`, the way a text hit is shown.
fn listed_text_hit(line: &str) -> Option<ListedTextHit> {
    let (head, text) = line.split_once(": ")?;
    let (object, board) = head.split_once(" in ")?;
    Some(ListedTextHit {
        board: board.split_once(' ')?.1.to_owned(),
        kind: object.split_whitespace().nth(1)?.to_owned(),
        text: text.to_owned(),
    })
}

impl Then<'_> {
    /// Expects exactly these objects (kind, text), in this order.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_objects(&self, expected: &[(&str, &str)]) {
        let shown = self
            .objects()
            .iter()
            .map(|row| (row.kind.as_str(), row.text.as_str()));
        assert!(
            shown.clone().eq(expected.iter().copied()),
            "expected the objects {expected:?}, got {:?}",
            shown.collect::<Vec<_>>()
        );
    }

    /// Expects exactly these objects (kind, text), in any order.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_objects_in_any_order(&self, expected: &[(&str, &str)]) {
        let shown: std::collections::BTreeSet<(&str, &str)> = self
            .objects()
            .iter()
            .map(|row| (row.kind.as_str(), row.text.as_str()))
            .collect();
        let wanted: std::collections::BTreeSet<(&str, &str)> =
            expected.iter().copied().collect();
        assert_eq!(shown, wanted, "expected the objects {expected:?}");
    }

    /// Expects the object with this text in the corner `x`, `y`.
    ///
    /// # Panics
    /// When there is none, or it is elsewhere.
    pub fn lists_object_at(&self, text: &str, (x, y): (i32, i32)) {
        let corner = format!("{x},{y}");
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.corner == corner),
            "expected {text:?} at {corner}, got {:?}",
            self.observation
        );
    }

    /// Expects an object of this kind and text inside a frame.
    ///
    /// # Panics
    /// When there is none.
    pub fn lists_object_in_a_frame(&self, kind: &str, text: &str) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.kind == kind && row.text == text && row.in_frame),
            "expected the {kind} {text:?} in a frame, got {:?}",
            self.observation
        );
    }

    /// Expects exactly the objects with these texts inside a frame or a group.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_inside_a_parent(&self, texts: &[&str]) {
        let inside: std::collections::BTreeSet<&str> = self
            .objects()
            .iter()
            .filter(|row| row.in_frame)
            .map(|row| row.text.as_str())
            .collect();
        let wanted: std::collections::BTreeSet<&str> = texts.iter().copied().collect();
        assert_eq!(inside, wanted, "{:?}", self.observation);
    }

    /// Expects the object with this text to be shown turned by these degrees.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_object_turned(&self, text: &str, degrees: &str) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.rotation.as_deref() == Some(degrees)),
            "expected {text:?} turned {degrees}, got {:?}",
            self.observation
        );
    }

    /// Expects the object with this text to be shown linked to an address that
    /// ends in `tail`, or to none when `tail` is empty.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_object_linked(&self, text: &str, tail: &str) {
        assert!(
            self.objects().iter().any(|row| {
                row.text == text
                    && row.link_to.as_deref().unwrap_or_default().ends_with(tail)
                    && row.link_to.is_some() != tail.is_empty()
            }),
            "expected {text:?} linked to {tail:?}, got {:?}",
            self.observation
        );
    }

    /// Expects exactly the objects with these texts to be shown locked.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_locked(&self, texts: &[&str]) {
        let locked: std::collections::BTreeSet<&str> = self
            .objects()
            .iter()
            .filter(|row| row.locked)
            .map(|row| row.text.as_str())
            .collect();
        let wanted: std::collections::BTreeSet<&str> = texts.iter().copied().collect();
        assert_eq!(locked, wanted, "{:?}", self.observation);
    }

    /// Expects no object inside a frame.
    ///
    /// # Panics
    /// When there is one.
    pub fn lists_no_object_in_a_frame(&self) {
        assert!(
            self.objects().iter().all(|row| !row.in_frame),
            "expected no object in a frame, got {:?}",
            self.observation
        );
    }

    /// Expects an object with this text, or none when `listed` is false.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_text(&self, text: &str, listed: bool) {
        assert_eq!(
            self.objects().iter().any(|row| row.text == text),
            listed,
            "expected {text:?} {}listed, got {:?}",
            if listed { "" } else { "not " },
            self.observation
        );
    }

    /// Expects this many objects.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn lists_this_many_objects(&self, count: usize) {
        assert_eq!(self.objects().len(), count, "{:?}", self.observation);
    }

    /// Expects the object with this text to have this size, as `WxH`.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_object_sized(&self, text: &str, size: &str) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.size.as_deref() == Some(size)),
            "expected {text:?} sized {size}, got {:?}",
            self.observation
        );
    }

    /// Expects the object with this text to have this color.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_object_coloured(&self, text: &str, color: &str) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.color.as_deref() == Some(color)),
            "expected {text:?} coloured {color}, got {:?}",
            self.observation
        );
    }

    /// Expects the object with this text to show exactly these marks and
    /// colors of its whole text, e.g. `bold`, `text:red6`, `bg:yellow4`.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_object_styled(&self, text: &str, look: &[&str]) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.text_look == look),
            "expected {text:?} styled {look:?}, got {:?}",
            self.observation
        );
    }

    /// Expects the shape with this text to list this kind and outline, in the
    /// words of a listing line.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_shape_looking(&self, text: &str, look: &str) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.look.as_deref() == Some(look)),
            "expected {text:?} looking {look:?}, got {:?}",
            self.observation
        );
    }

    /// Expects an arrow listing this line, heads and stroke, in the words of
    /// a listing line.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_arrow_looking(&self, look: &str) {
        let words = |row: &ListedObject| {
            [row.color.as_deref(), row.look.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert!(
            self.objects()
                .iter()
                .any(|row| row.ends.is_some() && words(row) == look),
            "expected an arrow looking {look:?}, got {:?}",
            self.observation
        );
    }

    /// Expects the object with this text to carry exactly these tags.
    ///
    /// # Panics
    /// When there is none, or it differs.
    pub fn lists_object_tagged(&self, text: &str, tags: &[&str]) {
        assert!(
            self.objects()
                .iter()
                .any(|row| row.text == text && row.tags == tags),
            "expected {text:?} tagged {tags:?}, got {:?}",
            self.observation
        );
    }

    /// Expects an arrow from the object with the text `from` to the one with
    /// the text `to`.
    ///
    /// # Panics
    /// When there is none.
    pub fn lists_arrow_between(&self, from: &str, to: &str) {
        let rows = self.objects();
        let text_of = |end: &Option<String>| {
            rows.iter()
                .find(|row| Some(&row.id) == end.as_ref())
                .map(|row| row.text.as_str())
        };
        assert!(
            rows.iter().any(|row| {
                row.ends.as_ref().is_some_and(|(start, end)| {
                    text_of(start) == Some(from) && text_of(end) == Some(to)
                })
            }),
            "expected an arrow from {from:?} to {to:?}, got {:?}",
            self.observation
        );
    }

    /// Expects two objects with this text and two different ids.
    ///
    /// # Panics
    /// When there are not.
    pub fn lists_two_objects_with_text(&self, text: &str) {
        let ids: Vec<&str> = self
            .objects()
            .iter()
            .filter(|row| row.text == text)
            .map(|row| row.id.as_str())
            .collect();
        assert!(
            matches!(ids.as_slice(), [first, second] if first != second),
            "expected two objects with {text:?}, got {:?}",
            self.observation
        );
    }

    /// Expects the object with the text `text` not to overlap the one with the
    /// text `other`; both need a corner and a size.
    ///
    /// # Panics
    /// When one is missing or they overlap.
    pub fn lists_objects_apart(&self, text: &str, other: &str) {
        let rect = |wanted: &str| {
            self.objects()
                .iter()
                .find(|row| row.text == wanted)
                .and_then(|row| {
                    let (x, y) = row.corner.split_once(',')?;
                    let (width, height) = row.size.as_deref()?.split_once('x')?;
                    Some([x, y, width, height].map(|number| number.parse::<f64>().ok()))
                })
                .and_then(|numbers| numbers.into_iter().collect::<Option<Vec<f64>>>())
        };
        let apart = rect(text).zip(rect(other)).is_some_and(|(a, b)| {
            matches!((a.as_slice(), b.as_slice()), (
                [ax, ay, aw, ah],
                [bx, by, bw, bh],
            ) if ax + aw <= *bx || bx + bw <= *ax || ay + ah <= *by || by + bh <= *ay)
        });
        assert!(
            apart,
            "expected {text:?} apart from {other:?}, got {:?}",
            self.observation
        );
    }

    fn objects(&self) -> &[ListedObject] {
        let rows = self.observation.objects();
        assert!(
            rows.is_some(),
            "expected a list of objects, got {:?}",
            self.observation
        );
        rows.unwrap_or_default()
    }

    /// Expects exactly these text matches (board, kind, text), best first;
    /// name matches are not counted.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn finds_text(&self, expected: &[(&str, &str, &str)]) {
        let shown: Vec<(&str, &str, &str)> = self
            .matches()
            .iter()
            .filter_map(|found| match found {
                ListedMatch::Text(hit) => {
                    Some((hit.board.as_str(), hit.kind.as_str(), hit.text.as_str()))
                }
                ListedMatch::Name(_) => None,
            })
            .collect();
        assert_eq!(shown, expected, "expected the texts {expected:?}");
    }

    /// Expects exactly these names (kind, name) among the matches, in any
    /// order; text matches are not counted.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn finds_names(&self, expected: &[(&str, &str)]) {
        let mut shown: Vec<(&str, &str)> = self
            .matches()
            .iter()
            .filter_map(|found| match found {
                ListedMatch::Name(name) => {
                    Some((name.kind.as_str(), name.name.as_str()))
                }
                ListedMatch::Text(_) => None,
            })
            .collect();
        let mut wanted = expected.to_vec();
        shown.sort_unstable();
        wanted.sort_unstable();
        assert_eq!(shown, wanted, "expected the names {expected:?}");
    }

    /// Expects the match named `name` to be in the trash.
    ///
    /// # Panics
    /// When there is none, or it is not in the trash.
    pub fn finds_a_trashed(&self, name: &str) {
        assert!(
            self.matches().iter().any(|found| matches!(found,
                ListedMatch::Name(seen) if seen.name == name && seen.in_trash)),
            "expected {name:?} in the trash, got {:?}",
            self.observation
        );
    }

    /// Expects exactly these text matches (board, kind, text), in any order;
    /// name matches are not counted.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn finds_texts(&self, expected: &[(&str, &str, &str)]) {
        let mut shown: Vec<(&str, &str, &str)> = self
            .matches()
            .iter()
            .filter_map(|found| match found {
                ListedMatch::Text(hit) => {
                    Some((hit.board.as_str(), hit.kind.as_str(), hit.text.as_str()))
                }
                ListedMatch::Name(_) => None,
            })
            .collect();
        let mut wanted = expected.to_vec();
        shown.sort_unstable();
        wanted.sort_unstable();
        assert_eq!(shown, wanted, "expected the texts {expected:?}");
    }

    /// Expects this many matches, names and texts together.
    ///
    /// # Panics
    /// When the observation differs.
    pub fn finds_matches(&self, count: usize) {
        assert_eq!(
            self.matches().len(),
            count,
            "expected {count} matches, got {:?}",
            self.observation
        );
    }

    fn matches(&self) -> &[ListedMatch] {
        let found = self.observation.found();
        assert!(
            found.is_some(),
            "expected the matches of a search, got {:?}",
            self.observation
        );
        found.unwrap_or_default()
    }
}
