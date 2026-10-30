//! The caller's reactions on a sticker: one `reactionsStore` entry each.

use itertools::Itertools as _;
use serde_json::{Value, json};
use yrs::{Map as _, TransactionMut, types::ToJson as _};

use super::{Written, only};
use crate::{
    domain::{
        board::{ObjectKind, insert_map, json as stored_json},
        change::write::WriteTransaction,
    },
    error::Result,
    protocol::document::{AUTHOR_ID, CREATED_AT, ID, REACTION_ID, TARGET_ID},
};

pub(super) const FIELD: &str = "reactions";

/// BUNDLE: the most reaction kinds a sticker holds (`R2#54`, 202219-202241).
pub const MOST_KINDS: usize = 8;

pub(super) fn check(kind: &ObjectKind) -> Result<()> {
    only(kind, &ObjectKind::Sticker, FIELD)
}

/// Makes the entries of the caller on `id` exactly `wanted`; other users' stay.
pub(super) fn write(
    wanted: &[String],
    transaction: &mut TransactionMut<'_>,
    scope: &WriteTransaction<'_>,
    id: &str,
) -> Result<Written> {
    let store = &scope.roots.reactions;
    let mine: Vec<(String, String)> = store
        .iter(&*transaction)
        .filter_map(|(entry, value)| {
            let fields = stored_json(&value.to_json(&*transaction))?;
            let text = |name: &str| fields.get(name).and_then(Value::as_str);
            let reaction = text(REACTION_ID)?;
            (text(TARGET_ID)? == id && text(AUTHOR_ID)? == scope.user)
                .then(|| (entry.to_owned(), reaction.to_owned()))
        })
        .sorted()
        .collect();
    let kept: Vec<&(String, String)> = mine
        .iter()
        .filter(|(_, reaction)| wanted.contains(reaction))
        .unique_by(|(_, reaction)| reaction)
        .collect();
    mine.iter()
        .filter(|entry| !kept.contains(entry))
        .for_each(|(entry, _)| {
            store.remove(transaction, entry);
        });
    wanted
        .iter()
        .unique()
        .filter(|reaction| !kept.iter().any(|(_, held)| held == *reaction))
        .try_for_each(|reaction| {
            let entry = scope.context.ids.new_id();
            let pairs = [
                (ID, json!(entry)),
                (TARGET_ID, json!(id)),
                (REACTION_ID, json!(reaction)),
                (AUTHOR_ID, json!(scope.user)),
                (CREATED_AT, json!(scope.stamp.at.as_millis())),
            ]
            .map(|(name, value)| (name.to_owned(), value));
            insert_map(transaction, store, entry.as_str(), pairs)
        })?;
    Ok(Written {
        fields: Vec::new(),
        touched: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::collections::BTreeSet;

    use serde_json::Value;
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use crate::{
        domain::{
            board::{self, shape_type::ShapeType},
            change::{
                build::build,
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{
                    BoardChange,
                    ChangeSources,
                    NewEntry,
                    NewObject,
                    ObjectEdit,
                    ObjectPatch,
                },
            },
            geometry::Point,
            id_source::SequentialIds,
            rich_text::TextFormat,
            time::Timestamp,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    const ME: &str = "me";
    const OTHER: &str = "other";

    fn board_with(new: NewObject) -> Result<Doc> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new,
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn a_sticker() -> Result<Doc> {
        board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })
    }

    fn react(document: &Doc, user: &'static str, reactions: &[&str]) -> Result<()> {
        let patch = ObjectPatch {
            kind_edits: vec![KindEdit::Reaction(
                reactions.iter().map(|&each| each.to_owned()).collect(),
            )],
            ..ObjectPatch::default()
        };
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: "s-1".into(),
            patch,
            stacking: None,
            tags: None,
        }]);
        let ids = SequentialIds::new(user);
        build(
            document,
            &change,
            user,
            &ChangeSources {
                built_at: Timestamp::from_millis(5),
                ids: &ids,
            },
        )
        .map(drop)
    }

    /// The (author, reaction) pairs the store holds for the sticker.
    fn held(document: &Doc) -> BTreeSet<(String, String)> {
        let transaction = document.transact();
        let store = transaction.get_map("reactionsStore");
        store
            .iter()
            .flat_map(|map| map.iter(&transaction))
            .filter_map(|(_, entry)| board::json(&entry.to_json(&transaction)))
            .filter(|entry| entry["targetId"] == "s-1")
            .map(|entry| (text(&entry, "authorId"), text(&entry, "reactionId")))
            .collect()
    }

    fn text(entry: &Value, name: &str) -> String {
        entry[name].as_str().unwrap_or_default().to_owned()
    }

    fn pairs(user: &str, reactions: &[&str]) -> Vec<(String, String)> {
        reactions
            .iter()
            .map(|&each| (user.to_owned(), each.to_owned()))
            .collect()
    }

    #[test]
    fn the_callers_reactions_become_the_asked_ones_and_the_others_stay() -> Result<()> {
        let document = a_sticker()?;
        react(&document, OTHER, &["heart"])?;
        react(&document, ME, &["like", "fire"])?;
        react(&document, ME, &["fire", "star"])?;
        let expected: BTreeSet<_> = pairs(OTHER, &["heart"])
            .into_iter()
            .chain(pairs(ME, &["fire", "star"]))
            .collect();
        assert_eq!(held(&document), expected);
        Ok(())
    }

    #[test]
    fn an_entry_holds_the_target_the_author_and_the_time() -> Result<()> {
        let document = a_sticker()?;
        react(&document, ME, &["like"])?;
        let transaction = document.transact();
        let entries: Vec<Value> = transaction
            .get_map("reactionsStore")
            .iter()
            .flat_map(|map| map.iter(&transaction))
            .filter_map(|(_, entry)| board::json(&entry.to_json(&transaction)))
            .collect();
        assert_eq!(
            entries,
            vec![serde_json::json!({
                "id": "me-1", "targetId": "s-1", "reactionId": "like",
                "authorId": "me", "createdAt": 5,
            })]
        );
        Ok(())
    }

    #[test]
    fn none_takes_the_callers_away_and_asking_twice_adds_no_entry() -> Result<()> {
        let document = a_sticker()?;
        react(&document, OTHER, &["heart"])?;
        react(&document, ME, &["like"])?;
        react(&document, ME, &["like", "like"])?;
        let twice = held(&document);
        react(&document, ME, &[])?;
        let expected: BTreeSet<_> = pairs(OTHER, &["heart"]).into_iter().collect();
        let both: BTreeSet<_> = pairs(OTHER, &["heart"])
            .into_iter()
            .chain(pairs(ME, &["like"]))
            .collect();
        assert_eq!((twice, held(&document)), (both, expected));
        Ok(())
    }

    #[test]
    fn a_reaction_leaves_the_stickers_updated_stamp_as_it_was() -> Result<()> {
        let document = a_sticker()?;
        let stamp = |held: &Doc| {
            let transaction = held.transact();
            transaction
                .get_map("objects")
                .and_then(|objects| objects.get(&transaction, "s-1"))
                .and_then(|object| board::json(&object.to_json(&transaction)))
                .and_then(|object| object.get("updated").cloned())
        };
        let before = stamp(&document);
        react(&document, ME, &["like"])?;
        assert_eq!((before.is_some(), stamp(&document)), (true, before));
        Ok(())
    }

    #[test]
    fn a_shape_takes_no_reactions() -> Result<()> {
        let document = board_with(NewObject::Shape {
            text: "box".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
            kind: ShapeType::Square,
        })?;
        let refused = react(&document, ME, &["like"]);
        assert!(matches!(
            refused,
            Err(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "reactions",
                ..
            }))
        ));
        Ok(())
    }
}
