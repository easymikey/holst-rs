//! The last listing the CLI printed, so that `@N` names one of its rows.

use std::path::PathBuf;

use holst_core::{
    domain::lookup::rules::is_alias,
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
}

/// Keeps `ids`, in row order, as the listing `@N` takes its row from.
pub(crate) fn save(
    run: &RunSettings,
    kind: ListedKind,
    ids: Vec<String>,
) -> Result<()> {
    path(run).map_or(Ok(()), |path| {
        let bytes = serde_json::to_vec(&Saved { kind, ids }).map_err(Error::Render)?;
        path.parent().map_or(Ok(()), file::create_dir_all)?;
        Ok(file::replace_private(&path, &[&bytes])?)
    })
}

/// Replaces each `@N` given as a `board` with the id of row N of the last
/// listing.
pub(crate) fn name_boards(
    arguments: &mut Map<String, Value>,
    run: &RunSettings,
) -> Result<()> {
    expand(arguments, || load(run))
}

fn expand(
    arguments: &mut Map<String, Value>,
    saved: impl Fn() -> Option<Saved>,
) -> Result<()> {
    let given = match arguments.get_mut("board") {
        Some(Value::Array(items)) => items.iter_mut().collect(),
        Some(one) => vec![one],
        None => Vec::new(),
    };
    given
        .into_iter()
        .filter_map(|value| match value {
            Value::String(text) => Some(text),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => None,
        })
        .try_for_each(|text| {
            number(text).map_or(Ok(()), |number| {
                *text = board_id(saved().as_ref(), number)?;
                Ok(())
            })
        })
}

/// N of `@N`, if `text` is one.
fn number(text: &str) -> Option<usize> {
    is_alias(text)
        .then(|| text.trim().trim_start_matches('@').parse().ok())
        .flatten()
}

fn board_id(saved: Option<&Saved>, number: usize) -> Result<String> {
    let saved = saved.ok_or(Error::NoListing { number })?;
    match saved.kind {
        ListedKind::Board => number
            .checked_sub(1)
            .and_then(|row| saved.ids.get(row))
            .cloned()
            .ok_or(Error::PastListing {
                number,
                rows: saved.ids.len(),
            }),
        ListedKind::Object => Err(Error::NotBoard { number }),
    }
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
        listing::{ListedKind, Saved, expand},
    };

    fn saved(kind: ListedKind) -> Saved {
        Saved {
            kind,
            ids: vec!["b1".to_owned(), "b2".to_owned()],
        }
    }

    fn boards(given: Value, saved: Option<&Saved>) -> Result<Value, Error> {
        let mut arguments = Map::new();
        arguments.insert("board".to_owned(), given);
        expand(&mut arguments, || saved.cloned())?;
        Ok(arguments.remove("board").unwrap_or_default())
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
}
