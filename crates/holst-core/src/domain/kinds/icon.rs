//! A Phosphor icon: a name from the manifest in one of six weights.

use serde::Deserialize;
use serde_json::{Value, json};

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        board_file::compression::uncompressed,
        change::value::Made,
        geometry::{Point, Size},
        palette::Color,
    },
    protocol::document::{COLOR, FILL_COLOR, HEIGHT, OPACITY, WIDTH},
};

/// The stored type of an icon.
const STORED: &str = "phosphor-icon";

const ICON_NAME: &str = "iconName";
const WEIGHT: &str = "weight";

/// What the web client places an icon as, in board units.
pub const DEFAULT_SIDE: f64 = 48.0;

const DEFAULT_COLOR: &str = "gray12";

/// The colors the icon's color picker offers.
pub const COLORS: [&str; 12] = [
    DEFAULT_COLOR,
    "gray8",
    "red9",
    "orange9",
    "yellow9",
    "lime9",
    "green9",
    "mint9",
    "blue9",
    "indigo9",
    "violet9",
    "pink9",
];

/// The six weights the web client draws an icon in.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Weight {
    Thin,
    Light,
    Regular,
    Bold,
    Fill,
    #[default]
    Duotone,
}

/// Whether the object is an icon.
#[must_use]
pub fn is_icon(object: &Object) -> bool {
    object.kind.as_str() == STORED
}

/// Whether `name` is lower-case letters and digits joined by dashes.
#[must_use]
pub fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|each| matches!(each, 'a'..='z' | '0'..='9' | '-'))
}

/// Where the web client reads the icon manifest, under the static files.
pub const MANIFEST_PATH: &str = "icons/phosphor/manifest.json";

/// The most close names an unknown icon is refused with.
const MOST_CLOSE: usize = 5;

/// The icons the manifest lists, each with the words it is searched by.
#[derive(Debug, PartialEq, Eq)]
pub struct Manifest(Vec<Listed>);

#[derive(Debug, PartialEq, Eq)]
struct Listed {
    name: String,
    words: Vec<String>,
}

/// One manifest entry: the name `n` and the search fields `a`, `t` and `r`.
#[derive(Deserialize)]
struct Entry {
    n: String,
    #[serde(default)]
    a: Value,
    #[serde(default)]
    t: Value,
    #[serde(default)]
    r: Value,
}

impl Entry {
    fn listed(self) -> Listed {
        let words = [self.a, self.t, self.r]
            .into_iter()
            .flat_map(|field| match field {
                Value::String(word) => vec![word],
                Value::Array(words) => words
                    .into_iter()
                    .filter_map(|word| word.as_str().map(str::to_owned))
                    .collect(),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
                    Vec::new()
                }
            })
            .map(|word| word.to_lowercase())
            .collect();
        Listed {
            name: self.n,
            words,
        }
    }
}

impl Listed {
    /// The name, its dash-separated parts and the search words.
    fn terms(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str())
            .chain(self.name.split('-'))
            .chain(self.words.iter().map(String::as_str))
    }
}

impl Manifest {
    /// Manifest from gzipped or plain bytes; none unless a non-empty list.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let json = uncompressed(bytes).ok()?;
        serde_json::from_slice::<Vec<Entry>>(&json)
            .ok()
            .filter(|entries| !entries.is_empty())
            .map(|entries| Self(entries.into_iter().map(Entry::listed).collect()))
    }

    /// Whether the manifest lists an icon called `name`.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|listed| listed.name == name)
    }

    /// Up to five nearest names by edit distance to a name, part or word.
    #[must_use]
    pub fn close(&self, name: &str) -> Vec<String> {
        let allowed = (name.chars().count() / 3).max(1);
        let mut near: Vec<(usize, &str)> = self
            .0
            .iter()
            .filter_map(|listed| {
                let best = listed.terms().map(|term| distance(name, term)).min()?;
                (best <= allowed).then_some((best, listed.name.as_str()))
            })
            .collect();
        near.sort_by_key(|(best, listed)| (*best, listed.len(), *listed));
        near.into_iter()
            .take(MOST_CLOSE)
            .map(|(_, listed)| listed.to_owned())
            .collect()
    }
}

/// The edit distance between two words: inserts, deletes and changes.
fn distance(first: &str, second: &str) -> usize {
    let second: Vec<char> = second.chars().collect();
    first
        .chars()
        .enumerate()
        .fold(
            (0..=second.len()).collect::<Vec<_>>(),
            |above, (row, each)| {
                above.iter().zip(above.iter().skip(1)).zip(&second).fold(
                    vec![row + 1],
                    |mut cells, ((diagonal, over), other)| {
                        let left = cells.last().copied().unwrap_or_default();
                        cells.push(
                            (diagonal + usize::from(each != *other))
                                .min(over + 1)
                                .min(left + 1),
                        );
                        cells
                    },
                )
            },
        )
        .last()
        .copied()
        .unwrap_or_default()
}

/// The name of the icon `made` builds; none for another object.
#[must_use]
pub fn named(made: &Made) -> Option<&str> {
    (made.kind == ObjectKind::Other(STORED.to_owned()))
        .then(|| {
            made.fields
                .iter()
                .find(|(key, _)| *key == ICON_NAME)
                .and_then(|(_, name)| name.as_str())
        })
        .flatten()
}

/// Icon `side` wide in `weight`, `color` filled; corner set by caller.
#[must_use]
pub fn made(name: &str, weight: Weight, color: Option<&Color>, side: f64) -> Made {
    Made {
        kind: ObjectKind::Other(STORED.to_owned()),
        at: Point::ORIGIN,
        size: Size::new(side, side).ok(),
        parent: None,
        fields: vec![
            (ICON_NAME, json!(name)),
            (WEIGHT, json!(weight.as_ref())),
            (WIDTH, json!(side)),
            (HEIGHT, json!(side)),
            (
                FILL_COLOR,
                json!({
                    (COLOR): color.map_or(DEFAULT_COLOR, Color::as_str),
                    (OPACITY): 1,
                }),
            ),
        ],
        texts: Vec::new(),
        code: None,
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// The name as the text and the `weight`, each when it is stored.
pub(super) fn detail(icon: &Object) -> Detail {
    let stored = |name| icon.fields.get(name).filter(|each| each.is_string());
    Detail {
        text: stored(ICON_NAME)
            .and_then(|name| name.as_str())
            .map(str::to_owned),
        words: stored(WEIGHT)
            .map(|weight| (WEIGHT, weight.clone()))
            .into_iter()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::Doc;

    use super::{COLORS, Manifest, Weight, is_icon, is_name, made, named};
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            board_file::compression::{Compression, compressed},
            change::value::Made,
            kinds::detail,
            palette::Color,
        },
    };

    #[test]
    fn a_made_icon_has_the_keys_and_defaults_of_the_web_client() -> Result<()> {
        let red: Color = "red9".parse()?;
        let plain = made("rocket-launch", Weight::default(), None, 48.0);
        let drawn = made("star", Weight::Fill, Some(&red), 96.0);
        assert_eq!(
            (
                plain.kind,
                plain.fields,
                plain.size.map(|size| (size.width(), size.height())),
                drawn.fields,
            ),
            (
                ObjectKind::Other("phosphor-icon".to_owned()),
                vec![
                    ("iconName", json!("rocket-launch")),
                    ("weight", json!("duotone")),
                    ("width", json!(48.0)),
                    ("height", json!(48.0)),
                    ("fillColor", json!({"color": "gray12", "opacity": 1})),
                ],
                Some((48.0, 48.0)),
                vec![
                    ("iconName", json!("star")),
                    ("weight", json!("fill")),
                    ("width", json!(96.0)),
                    ("height", json!(96.0)),
                    ("fillColor", json!({"color": "red9", "opacity": 1})),
                ],
            )
        );
        Ok(())
    }

    #[test]
    fn the_picker_offers_twelve_colors_gray12_first() {
        assert_eq!(
            (COLORS.len(), COLORS.first(), COLORS.last()),
            (12, Some(&"gray12"), Some(&"pink9"))
        );
    }

    #[test]
    fn a_name_is_lower_case_words_joined_by_dashes() {
        assert_eq!(
            [
                "rocket-launch",
                "a-arrow-down",
                "x",
                "",
                "Rocket Launch",
                "rocket_launch",
                "rocket-launch ",
            ]
            .map(is_name),
            [true, true, true, false, false, false, false]
        );
    }

    fn listed(fields: &Value) -> (Option<String>, Vec<(&'static str, Value)>) {
        let mut stored = json!({"id": "i1", "type": "phosphor-icon",
            "position": {"x": 0, "y": 0}});
        stored
            .as_object_mut()
            .zip(fields.as_object())
            .into_iter()
            .for_each(|(stored, fields)| stored.extend(fields.clone()));
        object_of(stored)
            .map(|object| detail(&Doc::new(), &object))
            .map(|detail| (detail.text, detail.words))
            .unwrap_or_default()
    }

    #[test]
    fn an_icon_lists_its_name_as_text_and_its_weight() {
        assert_eq!(
            [
                listed(&json!({"iconName": "rocket-launch", "weight": "duotone"})),
                listed(&json!({"iconName": "star"})),
                listed(&json!({"weight": "bold"})),
                listed(&json!({"iconName": 3, "weight": 4})),
            ],
            [
                (
                    Some("rocket-launch".to_owned()),
                    vec![("weight", json!("duotone"))]
                ),
                (Some("star".to_owned()), vec![]),
                (None, vec![("weight", json!("bold"))]),
                (None, vec![]),
            ]
        );
    }

    const LISTED: &str = r#"[{"n":"rocket","t":["space"]},
        {"n":"rocket-launch","a":["liftoff"]},{"n":"star","r":"favorite"},{"n":"cat"}]"#;

    #[test]
    fn a_manifest_is_read_plain_or_gzipped_and_names_are_looked_up() -> Result<()> {
        let packed = compressed(LISTED.as_bytes(), Compression::Gzip)?;
        let [plain, gzipped] =
            [LISTED.as_bytes(), packed.as_ref()].map(Manifest::parse);
        let has = |manifest: &Option<Manifest>| {
            ["rocket-launch", "star", "rokcet", "rocket-"]
                .map(|name| manifest.as_ref().map(|listed| listed.has(name)))
        };
        assert_eq!(
            [has(&plain), has(&gzipped)],
            [[Some(true), Some(true), Some(false), Some(false)]; 2]
        );
        assert_eq!(
            [
                Manifest::parse(b"not json"),
                Manifest::parse(br#"{"n":"star"}"#),
                Manifest::parse(b"[]"),
            ],
            [None, None, None]
        );
        Ok(())
    }

    #[test]
    fn close_names_are_the_nearest_by_name_part_and_search_word_at_most_five() {
        let many: Vec<String> = ["star", "star-half", "star-four", "star-of-david"]
            .into_iter()
            .chain(["star-and-crescent", "starfish", "stars"])
            .map(|name| format!(r#"{{"n":"{name}"}}"#))
            .collect();
        let crowded = Manifest::parse(format!("[{}]", many.join(",")).as_bytes());
        let small = Manifest::parse(LISTED.as_bytes());
        let close = |manifest: &Option<Manifest>, name: &str| {
            manifest
                .as_ref()
                .map(|listed| listed.close(name))
                .unwrap_or_default()
        };
        assert_eq!(
            [
                close(&small, "rokcet"),
                close(&small, "liftof"),
                close(&small, "zzzzzzzz"),
                close(&crowded, "star"),
            ],
            [
                vec!["rocket".to_owned(), "rocket-launch".to_owned()],
                vec!["rocket-launch".to_owned()],
                vec![],
                [
                    "star",
                    "star-four",
                    "star-half",
                    "star-of-david",
                    "star-and-crescent"
                ]
                .map(str::to_owned)
                .to_vec(),
            ]
        );
    }

    #[test]
    fn a_made_icon_gives_its_name_and_other_objects_none() {
        let icon = made("star", Weight::Bold, None, 48.0);
        let other = Made {
            kind: ObjectKind::Other("stamp".to_owned()),
            ..made("star", Weight::Bold, None, 48.0)
        };
        assert_eq!([named(&icon), named(&other)], [Some("star"), None]);
    }

    #[test]
    fn only_a_phosphor_icon_is_one() {
        let kind = |name: &str| {
            object_of(json!({"id": "o", "type": name, "position": {"x": 0, "y": 0}}))
                .map(|object| is_icon(&object))
        };
        assert_eq!(
            [kind("phosphor-icon"), kind("reaction-stamp"), kind("stamp")],
            [Some(true), Some(false), Some(false)]
        );
    }
}
