//! The last listing the CLI printed, so that `@N` names one of its rows.

use std::path::PathBuf;

use holst_core::{
    domain::{id::uuid_spans, lookup::rules::is_alias},
    effect::settings::SessionLocation,
    storage::{file, local_data::CONFIG_DIR},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    run::RunSettings,
};

const FILE: &str = "listing";

/// What the rows of a listing are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ListedKind {
    Board,
    Object,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    kind: ListedKind,
    ids: Vec<String>,
    /// The board an object listing is of; an old file has none.
    #[serde(default)]
    board: Option<ListedBoard>,
}

/// The board an object listing was taken from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ListedBoard {
    pub(crate) id: String,
    /// The text the user named it by.
    pub(crate) asked: String,
}

/// Keeps `ids`, in row order, as the listing `@N` takes its row from.
pub(crate) fn save(
    run: &RunSettings,
    kind: ListedKind,
    ids: Vec<String>,
    board: Option<ListedBoard>,
) -> Result<()> {
    path(run).map_or(Ok(()), |path| {
        let saved = Saved { kind, ids, board };
        let bytes = serde_json::to_vec(&saved).map_err(Error::Render)?;
        path.parent().map_or(Ok(()), file::create_dir_all)?;
        Ok(file::replace_private(&path, &[&bytes])?)
    })
}

/// Replaces each `@N` given as a board or an object with the id of row N of
/// the last listing.
pub(crate) fn name_rows(
    arguments: &mut Map<String, Value>,
    run: &RunSettings,
) -> Result<()> {
    expand(arguments, || load(run))
}

/// What a row of the listing must be for the argument `name` to take it.
pub(crate) fn wanted(name: &str) -> Option<ListedKind> {
    match name {
        "board" => Some(ListedKind::Board),
        "object" | "frame" | "from" | "to" | "parent" => Some(ListedKind::Object),
        _ => None,
    }
}

fn expand(
    arguments: &mut Map<String, Value>,
    saved: impl Fn() -> Option<Saved>,
) -> Result<()> {
    let board = arguments
        .get("board")
        .and_then(Value::as_str)
        .map(str::to_owned);
    arguments
        .iter_mut()
        .filter_map(|(name, value)| wanted(name).map(|kind| (kind, value)))
        .try_for_each(|(kind, value)| {
            let given = match value {
                Value::Array(items) => items.iter_mut().collect(),
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Object(_) => vec![value],
            };
            given
                .into_iter()
                .filter_map(|each| match each {
                    Value::String(text) => Some(text),
                    Value::Null
                    | Value::Bool(_)
                    | Value::Number(_)
                    | Value::Array(_)
                    | Value::Object(_) => None,
                })
                .try_for_each(|text| {
                    number(text).map_or(Ok(()), |number| {
                        *text =
                            row_id(saved().as_ref(), kind, number, board.as_deref())?;
                        Ok(())
                    })
                })
        })
}

/// N of `@N`, if `text` is one.
fn number(text: &str) -> Option<usize> {
    is_alias(text)
        .then(|| text.trim().trim_start_matches('@').parse().ok())
        .flatten()
}

fn row_id(
    saved: Option<&Saved>,
    kind: ListedKind,
    number: usize,
    board: Option<&str>,
) -> Result<String> {
    let saved = saved.ok_or(Error::NoListing { number })?;
    match (saved.kind, kind) {
        (ListedKind::Object, ListedKind::Board) => {
            return Err(Error::NotBoard { number });
        }
        (ListedKind::Board, ListedKind::Object) => {
            return Err(Error::NotObject { number });
        }
        (ListedKind::Object, ListedKind::Object) => {
            let listed = saved.board.as_ref().ok_or(Error::NoListing { number })?;
            if let Some(given) = board.filter(|given| other_board(listed, given)) {
                return Err(Error::OtherBoard {
                    number,
                    listed: listed.asked.clone(),
                    given: given.to_owned(),
                });
            }
        }
        (ListedKind::Board, ListedKind::Board) => {}
    }
    number
        .checked_sub(1)
        .and_then(|row| saved.ids.get(row))
        .cloned()
        .ok_or(Error::PastListing {
            number,
            rows: saved.ids.len(),
        })
}

/// Whether `given` surely names another board than the listed one; an id
/// against a name cannot be told apart, so it is not.
fn other_board(listed: &ListedBoard, given: &str) -> bool {
    let given = given.trim().to_lowercase();
    let id = listed.id.to_lowercase();
    let asked = listed.asked.trim().to_lowercase();
    let given_is_id = id_like(&given);
    if given.contains(&id) || (given_is_id && id.starts_with(&given)) {
        return false;
    }
    if given_is_id != id_like(&asked) {
        return false;
    }
    given_is_id || !(asked.contains(&given) || given.contains(&asked))
}

/// A uuid, a link with one, or the first characters of one.
fn id_like(text: &str) -> bool {
    uuid_spans(text).next().is_some()
        || (text.len() >= 8 && text.chars().all(|each| each.is_ascii_hexdigit()))
}

fn load(run: &RunSettings) -> Option<Saved> {
    let bytes = file::read(path(run)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Beside the saved login: `HOLST_HOME`, else holst's folder in the home.
fn path(run: &RunSettings) -> Option<PathBuf> {
    match run.location() {
        SessionLocation::Dir(dir) => Some(dir),
        SessionLocation::System { home } => home.map(|home| home.join(CONFIG_DIR)),
    }
    .map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use crate::{
        error::Error,
        listing::{ListedBoard, ListedKind, Saved, expand},
    };

    fn saved(kind: ListedKind) -> Saved {
        Saved {
            kind,
            ids: vec!["b1".to_owned(), "b2".to_owned()],
            board: Some(ListedBoard {
                id: "0123456789abcdef".to_owned(),
                asked: "A".to_owned(),
            }),
        }
    }

    fn named(name: &str, given: Value, saved: Option<&Saved>) -> Result<Value, Error> {
        let mut arguments = Map::new();
        arguments.insert(name.to_owned(), given);
        expand(&mut arguments, || saved.cloned())?;
        Ok(arguments.remove(name).unwrap_or_default())
    }

    fn boards(given: Value, saved: Option<&Saved>) -> Result<Value, Error> {
        named("board", given, saved)
    }

    #[test]
    fn a_handle_names_its_row_and_other_text_is_kept() {
        let listing = saved(ListedKind::Board);
        let given = json!(["@2", "Plans", "@1 apples", "@12345678"]);
        assert_eq!(
            boards(given, Some(&listing)).ok(),
            Some(json!(["b2", "Plans", "@1 apples", "@12345678"]))
        );
        assert_eq!(
            boards(json!(" @1 "), Some(&listing)).ok(),
            Some(json!("b1"))
        );
    }

    #[test]
    fn a_handle_without_a_listing_is_refused() {
        let error = boards(json!("@3"), None).err();
        assert!(
            matches!(error, Some(Error::NoListing { number: 3 })),
            "{error:?}"
        );
        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("no listing to take `@3` from")
        );
    }

    #[test]
    fn a_handle_past_the_listing_says_how_long_it_is() {
        let error = boards(json!("@3"), Some(&saved(ListedKind::Board))).err();
        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("@3 is past the last listing (2 rows)")
        );
        let zero = boards(json!("@0"), Some(&saved(ListedKind::Board))).err();
        assert!(
            matches!(zero, Some(Error::PastListing { number: 0, .. })),
            "{zero:?}"
        );
    }

    #[test]
    fn a_handle_of_an_object_listing_is_no_board() {
        let error = boards(json!("@1"), Some(&saved(ListedKind::Object))).err();
        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("@1 names an object, not a board")
        );
    }

    #[test]
    fn an_object_handle_names_a_row_of_an_object_listing() {
        let listing = saved(ListedKind::Object);
        for name in ["object", "frame", "from", "to", "parent"] {
            assert_eq!(
                named(name, json!(["@2", "a1"]), Some(&listing)).ok(),
                Some(json!(["b2", "a1"])),
                "{name}"
            );
        }
        assert_eq!(
            named("workspace", json!("@2"), Some(&listing)).ok(),
            Some(json!("@2"))
        );
        let error = named("object", json!("@3"), Some(&listing)).err();
        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("@3 is past the last listing (2 rows)")
        );
    }

    #[test]
    fn a_handle_of_a_board_listing_is_no_object() {
        let error = named("object", json!("@1"), Some(&saved(ListedKind::Board))).err();
        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("@1 names a board, not an object")
        );
    }

    #[test]
    fn an_object_handle_of_another_board_says_which_board_it_is_from() {
        let listing = saved(ListedKind::Object);
        let error = named_on("@2", "B", &listing).err();
        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some(r#"@2 is from the listing of board "A", not "B""#)
        );
        ["A", " a ", "0123456789abcdef", "01234567"]
            .iter()
            .for_each(|same| {
                assert_eq!(
                    named_on("@2", same, &listing).ok(),
                    Some(json!("b2")),
                    "{same}"
                );
            });
    }

    #[test]
    fn an_object_handle_of_a_listing_without_a_board_is_refused() {
        let mut listing = saved(ListedKind::Object);
        listing.board = None;
        let error = named_on("@1", "A", &listing).err();
        assert!(
            matches!(error, Some(Error::NoListing { number: 1 })),
            "{error:?}"
        );
    }

    fn named_on(handle: &str, board: &str, listing: &Saved) -> Result<Value, Error> {
        let mut arguments = Map::new();
        arguments.insert("board".to_owned(), json!(board));
        arguments.insert("object".to_owned(), json!(handle));
        expand(&mut arguments, || Some(listing.clone()))?;
        Ok(arguments.remove("object").unwrap_or_default())
    }
}
