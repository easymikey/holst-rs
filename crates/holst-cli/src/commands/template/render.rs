//! What the template commands print.

use holst_core::{
    command::{
        change::{ChangeReport, PlannedRequest},
        template::{Changed, ListedTemplate, Listing},
    },
    domain::{
        template::{Template, TemplateCategory},
        text::{EMPTY, counted, joined},
    },
};
use serde_json::Value;

use crate::{
    error::Result,
    output::{self, OutputFormat},
};

pub(super) fn listing(format: OutputFormat, listing: &Listing) -> Result<()> {
    output::print(format, &listing_text(listing), listing)
}

pub(super) fn shown(format: OutputFormat, template: &Template) -> Result<()> {
    output::print(format, &shown_text(template), template)
}

pub(super) fn changed(format: OutputFormat, changed: &Changed) -> Result<()> {
    output::print(format, &changed_text(changed), changed)
}

fn listing_text(listing: &Listing) -> String {
    let lines: Vec<String> = listing
        .templates
        .iter()
        .map(template_line)
        .chain(listing.categories.iter().map(category_line))
        .collect();
    if lines.is_empty() {
        EMPTY.to_owned()
    } else {
        lines.join("\n")
    }
}

fn template_line(listed: &ListedTemplate) -> String {
    let id = listed.template.id.as_deref().unwrap_or("(no id)");
    let name = listed.template.name.as_deref().unwrap_or("(no name)");
    listed.under.as_ref().map_or_else(
        || format!("{id}  {name}"),
        |under| format!("{id}  {name}  ({under})"),
    )
}

fn category_line(category: &TemplateCategory) -> String {
    let count = category
        .template_count
        .map(|count| format!(", {}", counted(count, "template")))
        .unwrap_or_default();
    format!(
        "{}  {}  ({}{count})",
        category.slug, category.name, category.id
    )
}

fn shown_text(template: &Template) -> String {
    let head = format!(
        "{} ({})",
        template.name.as_deref().unwrap_or("(no name)"),
        template.id.as_deref().unwrap_or("no id")
    );
    let fields = match serde_json::to_value(template) {
        Ok(Value::Object(fields)) => fields,
        Ok(_) | Err(_) => serde_json::Map::new(),
    };
    let rest = fields
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "id" | "name"))
        .map(|(key, value)| match value {
            Value::String(text) => format!("{key}: {text}"),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => format!("{key}: {value}"),
        });
    joined(std::iter::once(head).chain(rest), "\n").to_string()
}

fn changed_text(changed: &Changed) -> String {
    let request = match &changed.outcome {
        ChangeReport::Sent => None,
        ChangeReport::DryRun { request } | ChangeReport::Held { request, .. } => {
            Some(request)
        }
    };
    joined(
        std::iter::once(changed.phrase())
            .chain(request.into_iter().flat_map(PlannedRequest::lines)),
        "\n",
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::{
            change::ChangeReport,
            template::{Action, Changed},
        },
        domain::template::Template,
    };
    use rstest::rstest;

    use crate::commands::template::render::{changed_text, shown_text};

    fn change(action: Action, outcome: ChangeReport) -> Changed {
        Changed {
            action,
            id: Some("t1".to_owned()),
            name: Some("Retro".to_owned()),
            objects: (action == Action::Create).then_some(3),
            outcome,
        }
    }

    #[rstest]
    #[case::created(Action::Create, "created template Retro (t1) with 3 objects")]
    #[case::replaced(Action::Edit, "replaced the body of template Retro (t1)")]
    #[case::deleted(Action::Delete, "deleted template Retro (t1)")]
    fn a_sent_change_says_what_it_did(#[case] action: Action, #[case] says: &str) {
        assert_eq!(changed_text(&change(action, ChangeReport::Sent)), says);
    }

    #[test]
    fn a_shown_template_starts_with_its_name_and_id() {
        let template = Template {
            id: Some("t1".to_owned()),
            name: Some("Retro".to_owned()),
            ..Template::default()
        };
        assert_eq!(shown_text(&template), "Retro (t1)");
    }
}
