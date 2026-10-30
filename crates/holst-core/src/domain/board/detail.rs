//! What a listing keeps beside an object: the page of a link card and the tags.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::Value;
use yrs::{Doc, Map as _, Transact as _, types::ToJson as _};

use crate::{
    domain::{
        board::{Object, ObjectKind, json},
        kinds::task::INFO_ID,
        tree::name_index::build::normalized,
    },
    protocol::document,
};

/// The page a link card or an embed points to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkCard {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl LinkCard {
    /// The `link` and `title` of a link object or embed (`linkInfo`), if any.
    #[must_use]
    pub fn of(object: &Object) -> Option<Self> {
        let text = |holder: &Value, key: &str| {
            holder
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
        };
        let info = object.fields.get(document::LINK_INFO);
        let from_info = |key: &str| info.and_then(|info| text(info, key));
        Some(Self {
            url: text(&object.fields, document::LINK)
                .or_else(|| from_info(document::LINK))
                .filter(|_| object.kind == ObjectKind::Link)?,
            title: text(&object.fields, document::TITLE)
                .or_else(|| from_info(document::TITLE)),
        })
    }
}

/// The strings of the `tagIds` list of a task card's info entry.
fn held_ids(info: &Value) -> Vec<String> {
    info.get(document::TAG_IDS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}

/// The tags of a board by id, each with the name it is shown under.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagNames {
    names: HashMap<String, String>,
    /// The tag ids of each task card's info entry, by the entry's id.
    cards: HashMap<String, Vec<String>>,
}

impl TagNames {
    /// The named definitions of the document's tag map.
    #[must_use]
    pub fn of(document: &Doc) -> Self {
        let store = document.get_or_insert_map(document::TAGS_STORE);
        let infos = document.get_or_insert_map(document::TASK_INFO_STORE);
        let txn = document.transact();
        Self {
            names: store
                .iter(&txn)
                .filter_map(|(id, definition)| {
                    let definition = json(&definition.to_json(&txn))?;
                    definition
                        .get(document::TITLE)
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .map(|name| (id.to_owned(), name.to_owned()))
                })
                .collect(),
            cards: infos
                .iter(&txn)
                .map(|(id, info)| {
                    let json = json(&info.to_json(&txn));
                    (id.to_owned(), json.as_ref().map_or_else(Vec::new, held_ids))
                })
                .collect(),
        }
    }

    /// The tag ids the object holds: its own, or its task card's info entry's.
    fn ids<'a>(&'a self, object: &'a Object) -> impl Iterator<Item = &'a str> {
        let card = object
            .fields
            .get(INFO_ID)
            .and_then(Value::as_str)
            .and_then(|info| self.cards.get(info))
            .into_iter()
            .flatten()
            .map(String::as_str);
        object
            .fields
            .get(document::TAG_IDS)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .chain(card)
    }

    /// The names of the tags the objects hold, by id.
    #[must_use]
    pub fn titles_in(&self, objects: &[Object]) -> BTreeMap<String, String> {
        objects
            .iter()
            .flat_map(|object| self.ids(object))
            .filter_map(|id| {
                self.names.get(id).map(|name| (id.to_owned(), name.clone()))
            })
            .collect()
    }

    /// The names of the object's tags, in the order it holds them.
    #[must_use]
    pub fn of_object(&self, object: &Object) -> Vec<String> {
        self.ids(object)
            .map(|id| self.names.get(id).map_or(id, String::as_str).to_owned())
            .collect()
    }

    /// Whether the object has the tag `wanted`, by id or name (case, ё
    /// ignored).
    #[must_use]
    pub fn has(&self, object: &Object, wanted: &str) -> bool {
        let by_name = normalized(wanted);
        self.ids(object).any(|id| {
            id == wanted
                || self
                    .names
                    .get(id)
                    .is_some_and(|name| normalized(name) == by_name)
        })
    }
}

/// How often each reaction was given to each object, by the object's id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reactions(HashMap<String, BTreeMap<String, usize>>);

impl Reactions {
    /// One pass over the document's reaction store.
    #[must_use]
    pub fn of(document: &Doc) -> Self {
        let store = document.get_or_insert_map(document::REACTIONS_STORE);
        let txn = document.transact();
        Self(
            store
                .iter(&txn)
                .filter_map(|(_, entry)| json(&entry.to_json(&txn)))
                .filter_map(|entry| {
                    let text = |name: &str| entry.get(name).and_then(Value::as_str);
                    Some((
                        text(document::TARGET_ID)?.to_owned(),
                        text(document::REACTION_ID)?.to_owned(),
                    ))
                })
                .fold(HashMap::new(), |mut counted, (target, reaction)| {
                    *counted
                        .entry(target)
                        .or_insert_with(BTreeMap::new)
                        .entry(reaction)
                        .or_insert(0) += 1;
                    counted
                }),
        )
    }

    /// The reactions on the object with their counts; empty when it has none.
    #[must_use]
    pub fn of_object(&self, object: &Object) -> BTreeMap<String, usize> {
        self.0.get(&object.id).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yrs::{Any, Doc, Map as _, Transact as _};

    use crate::domain::board::{
        ObjectKind,
        detail::{LinkCard, Reactions, TagNames},
        object_of,
        tests::plain_object,
    };

    fn with_fields(
        kind: ObjectKind,
        fields: serde_json::Value,
    ) -> crate::domain::board::Object {
        crate::domain::board::Object {
            fields,
            ..plain_object("o1", kind, "")
        }
    }

    #[test]
    fn a_link_card_keeps_its_address_and_title() {
        let card = object_of(json!({
            "id": "l1", "type": "link", "position": {"x": 0, "y": 0},
            "link": "https://example.com", "title": "Example Domain"
        }))
        .as_ref()
        .and_then(LinkCard::of);
        assert_eq!(
            card,
            Some(LinkCard {
                url: "https://example.com".into(),
                title: Some("Example Domain".into())
            })
        );
    }

    #[test]
    fn an_embed_keeps_them_in_its_link_info() {
        let embed = with_fields(
            ObjectKind::Link,
            json!({"linkInfo": {"link": "https://youtu.be/x", "title": "YouTube"}}),
        );
        let bare = with_fields(ObjectKind::Link, json!({"link": " "}));
        let sticker =
            with_fields(ObjectKind::Sticker, json!({"link": "https://example.com"}));
        assert_eq!(
            (
                LinkCard::of(&embed),
                LinkCard::of(&bare),
                LinkCard::of(&sticker)
            ),
            (
                Some(LinkCard {
                    url: "https://youtu.be/x".into(),
                    title: Some("YouTube".into())
                }),
                None,
                None
            )
        );
    }

    #[test]
    fn tags_show_under_their_names_and_match_by_name_or_id() {
        let document = Doc::new();
        let store = document.get_or_insert_map("task-card-tagsStore");
        {
            let mut txn = document.transact_mut();
            let definition = json!({"id": "t1", "title": "Urgent"}).to_string();
            store.insert(
                &mut txn,
                "t1",
                Any::from_json(&definition).unwrap_or(Any::Null),
            );
        }
        let names = TagNames::of(&document);
        let tagged = with_fields(ObjectKind::Sticker, json!({"tagIds": ["t1", "t2"]}));
        let plain = with_fields(ObjectKind::Sticker, json!({}));
        assert_eq!(
            (
                names.of_object(&tagged),
                names.of_object(&plain),
                names.has(&tagged, "URGENT"),
                names.has(&tagged, "t2"),
                names.has(&tagged, "later"),
                names.has(&plain, "urgent"),
            ),
            (
                vec!["Urgent".to_owned(), "t2".to_owned()],
                Vec::new(),
                true,
                true,
                false,
                false
            )
        );
    }

    #[test]
    fn a_task_card_shows_the_tags_of_its_info_entry() {
        let document = Doc::new();
        let tags = document.get_or_insert_map("task-card-tagsStore");
        let infos = document.get_or_insert_map("task-card-infoStore");
        {
            let mut txn = document.transact_mut();
            let definition = json!({"id": "t1", "title": "Urgent"}).to_string();
            tags.insert(
                &mut txn,
                "t1",
                Any::from_json(&definition).unwrap_or(Any::Null),
            );
            let info = json!({"tagIds": ["t1"], "assigneeIds": []}).to_string();
            infos.insert(&mut txn, "i1", Any::from_json(&info).unwrap_or(Any::Null));
        }
        let names = TagNames::of(&document);
        let card = with_fields(ObjectKind::Task, json!({"taskCardInfoId": "i1"}));
        let bare = with_fields(ObjectKind::Task, json!({"taskCardInfoId": "i2"}));
        assert_eq!(
            (
                names.of_object(&card),
                names.has(&card, "urgent"),
                names.of_object(&bare),
                names.titles_in(&[card]),
            ),
            (
                vec!["Urgent".to_owned()],
                true,
                Vec::new(),
                [("t1".to_owned(), "Urgent".to_owned())].into()
            )
        );
    }

    #[test]
    fn reactions_are_counted_per_object() {
        let document = Doc::new();
        let store = document.get_or_insert_map("reactionsStore");
        {
            let mut txn = document.transact_mut();
            [
                ("r1", "o1", "like"),
                ("r2", "o1", "like"),
                ("r3", "o1", "fire"),
                ("r4", "o2", "like"),
            ]
            .into_iter()
            .map(|(entry, target, reaction)| {
                let fields =
                    json!({"id": entry, "targetId": target, "reactionId": reaction});
                store.insert(
                    &mut txn,
                    entry,
                    Any::from_json(&fields.to_string()).unwrap_or(Any::Null),
                );
            })
            .for_each(drop);
        }
        let counts = Reactions::of(&document);
        let first = with_fields(ObjectKind::Sticker, json!({}));
        let other = crate::domain::board::Object {
            id: "o9".into(),
            ..first.clone()
        };
        assert_eq!(
            (counts.of_object(&first), counts.of_object(&other)),
            (
                [("fire".to_owned(), 1), ("like".to_owned(), 2)].into(),
                std::collections::BTreeMap::new()
            )
        );
    }
}
