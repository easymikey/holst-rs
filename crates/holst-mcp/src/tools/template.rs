//! The `template` tool: list, show, create, edit, delete.

use holst_core::{
    command::{
        change::ChangeReport,
        template::{
            Changed,
            ListedTemplate,
            Listing,
            LsArgs,
            ShowArgs,
            TemplateVerb,
            create,
            edit,
            ls,
            rm,
            show,
        },
    },
    domain::{
        template::{Template, TemplateCategory},
        text::{EMPTY, counted, printable},
    },
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{server::McpServer, tools::workspace::joined_by_dots};

/// The templates a listing shows; the rest is cut.
const LIST_LIMIT: usize = 100;

impl McpServer {
    pub(super) async fn template_command(
        &self,
        verb: TemplateVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self.expanded_references(arguments) {
            Ok(arguments) => self.run_template_verb(verb, arguments).await,
            Err(err) => self.fail(&err),
        }
    }

    async fn run_template_verb(
        &self,
        verb: TemplateVerb,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match verb {
            TemplateVerb::Ls => {
                self.parsed(arguments, async |args: LsArgs| {
                    self.run(async |client| {
                        Ok(self.template_listing_text(&ls(&client, &args).await?))
                    })
                    .await
                })
                .await
            }
            TemplateVerb::Show => {
                self.parsed(arguments, async |args: ShowArgs| {
                    self.run(async |client| {
                        Ok(self.template_shown_text(&show(&client, &args).await?))
                    })
                    .await
                })
                .await
            }
            TemplateVerb::Create => {
                self.change(arguments, create, Self::template_changed_text)
                    .await
            }
            TemplateVerb::Edit => {
                self.change(arguments, edit, Self::template_changed_text)
                    .await
            }
            TemplateVerb::Rm => {
                self.change(arguments, rm, Self::template_changed_text)
                    .await
            }
        }
    }

    fn template_listing_text(&self, listing: &Listing) -> String {
        let total = listing.templates.len() + listing.categories.len();
        let lines: String = listing
            .templates
            .iter()
            .map(|listed| self.template_line(listed))
            .chain(
                listing
                    .categories
                    .iter()
                    .map(|each| self.category_line(each)),
            )
            .take(LIST_LIMIT)
            .collect();
        match (lines.is_empty(), total > LIST_LIMIT) {
            (true, _) => format!("{EMPTY}\n"),
            (false, true) => format!("{lines}{LIST_LIMIT} of {total}\n"),
            (false, false) => lines,
        }
    }

    fn template_line(&self, listed: &ListedTemplate) -> String {
        joined_by_dots(self.template_head(&listed.template), [listed.under.clone()])
            + "\n"
    }

    fn category_line(&self, category: &TemplateCategory) -> String {
        let head = format!(
            "{} {}",
            printable(&category.name),
            self.state.alias(&category.id)
        );
        let details = [
            Some(category.slug.clone()),
            category
                .template_count
                .map(|count| counted(count, "template")),
        ];
        joined_by_dots(head, details) + "\n"
    }

    fn template_head(&self, template: &Template) -> String {
        let name = template
            .name
            .as_deref()
            .map_or_else(|| "(no name)".into(), printable);
        match template.id.as_deref() {
            Some(id) => format!("{name} {}", self.state.alias(id)),
            None => name.into_owned(),
        }
    }

    fn template_shown_text(&self, template: &Template) -> String {
        let rest: Map<String, Value> = match serde_json::to_value(template) {
            Ok(Value::Object(fields)) => fields
                .into_iter()
                .filter(|(key, _)| !matches!(key.as_str(), "id" | "name"))
                .collect(),
            Ok(_) | Err(_) => Map::new(),
        };
        let head = self.template_head(template);
        if rest.is_empty() {
            format!("{head}\n")
        } else {
            let body = self.state.render(Value::Object(rest));
            format!("{head}\n{}\n", body.trim_end())
        }
    }

    fn template_changed_text(&self, changed: &Changed) -> String {
        let lines = match &changed.outcome {
            ChangeReport::Sent => String::new(),
            ChangeReport::DryRun { request } | ChangeReport::Held { request, .. } => {
                request.lines().join("\n") + "\n"
            }
        };
        self.state
            .alias_text(&format!("{}\n{lines}", changed.phrase()))
    }
}
