//! Board templates: their bodies, replies and categories.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    domain::response::OtherFields,
    error::{Error, Result, input::InputError, lookup, lookup::LookupError},
};

/// A board template; also the body of one to create or update.
///
/// ```
/// use holst_core::domain::template::Template;
///
/// let template: Template =
///     serde_json::from_str(r#"{"id":"t1","name":"Retro","cells":3}"#)?;
/// assert_eq!(template.name.as_deref(), Some("Retro"));
/// assert_eq!(
///     serde_json::to_string(&template)?,
///     r#"{"id":"t1","name":"Retro","cells":3}"#
/// );
///
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub other: OtherFields,
}

impl TryFrom<Map<String, Value>> for Template {
    type Error = Error;

    /// # Errors
    ///
    /// [`InputError::TemplateNotObject`] when `id` or `name` is not text.
    fn try_from(fields: Map<String, Value>) -> Result<Self> {
        serde_json::from_value(Value::Object(fields))
            .map_err(|_| Error::Input(InputError::TemplateNotObject))
    }
}

/// The templates a listing holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Templates {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "board_templates"
    )]
    pub templates: Option<Vec<Template>>,
    #[serde(flatten)]
    pub other: OtherFields,
}

/// A category of ready-made templates; its templates are requested by `slug`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateCategory {
    pub id: String,
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_ru: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "count_templates"
    )]
    pub template_count: Option<usize>,
    #[serde(flatten)]
    pub other: OtherFields,
}

/// The reply of `board_template/categories/standard`. An entry that is not a
/// readable category is kept for the shells to show, and skipped by
/// [`TemplateCategories::categories`].
///
/// ```
/// use holst_core::domain::template::TemplateCategories;
///
/// let reply =
///     r#"{ "categories": [{ "id": "c1", "slug": "retro", "name": "Retro" }, {}] }"#;
/// let all: TemplateCategories = serde_json::from_str(reply)?;
/// assert_eq!(
///     all.categories().next().map(|c| c.slug.as_str()),
///     Some("retro")
/// );
/// assert_eq!(all.categories().count(), 1);
/// assert_eq!(serde_json::to_string(&all)?.matches("{}").count(), 1);
///
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateCategories {
    #[serde(default, rename = "categories")]
    listed: Vec<Listed>,
    #[serde(flatten)]
    pub other: OtherFields,
}

impl TemplateCategories {
    /// The categories that carry an id, a slug and a name.
    pub fn categories(&self) -> impl Iterator<Item = &TemplateCategory> + Clone {
        self.listed.iter().filter_map(|entry| match entry {
            Listed::Category(category) => Some(category),
            Listed::Unreadable(_) => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Listed {
    Category(TemplateCategory),
    Unreadable(Value),
}

/// The slug of the category with this slug, id, name or `name_ru`, any case.
///
/// # Errors
///
/// A [`LookupError`] of a template category, with the known slugs, when nothing
/// matches.
///
/// ```
/// use holst_core::domain::template::{TemplateCategory, category_slug};
///
/// let retro = TemplateCategory {
///     id: "c1".into(),
///     slug: "retro".into(),
///     name: "Retrospectives".into(),
///     name_ru: Some("Ретроспективы".into()),
///     template_count: Some(3),
///     ..TemplateCategory::default()
/// };
/// let all = [retro];
/// assert_eq!(category_slug(all.iter(), "C1")?, "retro");
/// assert_eq!(category_slug(all.iter(), "retrospectives")?, "retro");
/// assert_eq!(category_slug(all.iter(), "ретроспективы")?, "retro");
/// assert!(category_slug(all.iter(), "planning").is_err());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
pub fn category_slug<'a>(
    categories: impl Iterator<Item = &'a TemplateCategory> + Clone,
    reference: &str,
) -> Result<&'a str> {
    let reference = reference.trim();
    let lower = reference.to_lowercase();
    categories
        .clone()
        .find(|c| {
            [
                Some(&c.slug),
                Some(&c.id),
                Some(&c.name),
                c.name_ru.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|key| key.to_lowercase() == lower)
        })
        .map(|c| c.slug.as_str())
        .ok_or_else(|| {
            Error::Lookup(LookupError {
                kind: lookup::LookupKind::TemplateCategory,
                reference: reference.to_owned(),
                miss: lookup::LookupMiss::NotFound {
                    unread: lookup::SkippedWorkspaces::default(),
                    available: categories
                        .map(|c| lookup::Candidate {
                            name: c.slug.clone(),
                            ..lookup::Candidate::default()
                        })
                        .collect(),
                },
            })
        })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use crate::{
        domain::template::{Template, TemplateCategories, Templates, category_slug},
        error::{Error, input::InputError},
    };

    #[rstest]
    #[case::slug("strategic-planning", Some("strategic-planning"))]
    #[case::id("61971CA1-E031-4995-970A-AA764F7A91C1", Some("strategic-planning"))]
    #[case::name(" Strategic Planning ", Some("strategic-planning"))]
    #[case::russian_name_case("ПЛАНИРОВАНИЕ", Some("strategic-planning"))]
    #[case::russian_name("мозговой штурм", Some("brainstorming"))]
    #[case::unknown("retro", None)]
    fn category_by_slug_id_or_name(
        #[case] reference: &str,
        #[case] slug: Option<&str>,
    ) {
        // The live shape: `name` is English, the Russian name is in `name_ru`.
        let reply = json!({ "categories": [
            { "id": "61971ca1-e031-4995-970a-aa764f7a91c1", "slug": "strategic-planning",
              "name": "Strategic planning", "name_ru": "Планирование", "icon": "planning",
              "standard": true, "order_index": 0, "count_templates": 13,
              "created_at": "2026-07-14T18:46:40.616435Z",
              "updated_at": "2026-07-14T18:46:40.616435Z" },
            { "id": "c2", "slug": "brainstorming", "name": "Brainstorming",
              "name_ru": "Мозговой штурм", "icon": "brainstorm", "standard": true,
              "order_index": 1, "count_templates": 4,
              "created_at": "2026-07-14T18:46:40.616435Z",
              "updated_at": "2026-07-14T18:46:40.616435Z" },
            { "id": "broken" },
        ]});
        let all = serde_json::from_value::<TemplateCategories>(reply.clone())
            .unwrap_or_default();
        assert_eq!(all.categories().count(), 2);
        assert_eq!(serde_json::to_value(&all).ok(), Some(reply));
        let found = category_slug(all.categories(), reference);
        assert_eq!(found.as_ref().ok().copied(), slug);
        let unknown = found.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            slug.is_some()
                || unknown.ends_with("Available: strategic-planning, brainstorming"),
            "{unknown}"
        );
    }

    /// The live shape of a listing comes back as it came, whatever the program
    /// does not name.
    #[test]
    fn template_list_round_trips_the_shape_of_the_server() {
        let reply = json!({
            "board_templates": [
                { "id": "t1", "name": "Roadmap", "cells": 3, "preview": null },
                { "id": "t2" },
            ],
            "total": 2,
        });
        let list = serde_json::from_value::<Templates>(reply.clone());
        assert_eq!(
            list.as_ref()
                .ok()
                .and_then(|list| list.templates.as_ref().map(Vec::len)),
            Some(2)
        );
        assert_eq!(
            list.ok().and_then(|list| serde_json::to_value(list).ok()),
            Some(reply)
        );
    }

    #[test]
    fn a_template_name_that_is_not_text_is_refused() {
        let fields = json!({ "name": 5 })
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert!(matches!(
            Template::try_from(fields),
            Err(Error::Input(InputError::TemplateNotObject))
        ));
    }
}
