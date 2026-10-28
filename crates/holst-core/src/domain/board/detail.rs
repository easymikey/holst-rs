//! What a listing keeps beside an object: the page of a link card and the tags.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::Value;
use yrs::{Doc, Map as _, Transact as _, types::ToJson as _};

use crate::{
    domain::{
        board::{Object, ObjectKind, json},
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
    /// The `link` and `title` of a link object; an embed keeps them in
    /// `linkInfo`. `None` for another kind, and for a link without an address.
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

/// The tags of a board by id, each with the name it is shown under.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagNames(HashMap<String, String>);

impl TagNames {
    /// The definitions in the document's tag map; a definition without a name
    /// is left out, so its tag shows under its id.
    #[must_use]
    pub fn of(document: &Doc) -> Self {
        let store = document.get_or_insert_map(document::TAGS_STORE);
        let txn = document.transact();
        Self(
            store
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
        )
    }

    fn ids(object: &Object) -> impl Iterator<Item = &str> {
        object
            .fields
            .get(document::TAG_IDS)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }

    /// The names of the tags the objects hold, by id; a tag nobody defined is
    /// left out.
    #[must_use]
    pub fn titles_in(&self, objects: &[Object]) -> BTreeMap<String, String> {
        objects
            .iter()
            .flat_map(Self::ids)
            .filter_map(|id| self.0.get(id).map(|name| (id.to_owned(), name.clone())))
            .collect()
    }

    /// The names of the object's tags, in the order it holds them.
    #[must_use]
    pub fn of_object(&self, object: &Object) -> Vec<String> {
        Self::ids(object)
            .map(|id| self.0.get(id).map_or(id, String::as_str).to_owned())
            .collect()
    }

    /// Whether the object has the tag `wanted`, by name (ignoring case and ё)
    /// or by id.
    #[must_use]
    pub fn has(&self, object: &Object, wanted: &str) -> bool {
        let by_name = normalized(wanted);
        Self::ids(object).any(|id| {
            id == wanted
                || self
                    .0
                    .get(id)
                    .is_some_and(|name| normalized(name) == by_name)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yrs::{Any, Doc, Map as _, Transact as _};

    use crate::domain::board::{
        ObjectKind,
        detail::{LinkCard, TagNames},
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
            // imperative: yrs inserts through one `&mut` transaction
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
}
