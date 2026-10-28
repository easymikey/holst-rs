//! The `template` tool: list, show, create, edit, delete.

use holst_core::{
    command::{
        change::ChangeReport,
        table::CommandArgs,
        template::{
            Changed,
            CreateArgs,
            EditArgs,
            ListedTemplate,
            Listing,
            LsArgs,
            RemoveArgs,
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
        text::{EMPTY, printable},
    },
    effect::board_write::WriteMode,
    remote::client::Client,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    login::HandedClient,
    server::McpServer,
    tools::{said, workspace::joined_by_dots},
};

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
                let mode = |args: &CreateArgs| args.dry_run;
                self.template_changing(arguments, mode, create).await
            }
            TemplateVerb::Edit => {
                let mode = |args: &EditArgs| args.dry_run;
                self.template_changing(arguments, mode, edit).await
            }
            TemplateVerb::Rm => {
                let mode = |args: &RemoveArgs| args.dry_run;
                self.template_changing(arguments, mode, rm).await
            }
        }
    }

    /// Runs a change: a dry run reads only, and a held one is refused with
    /// the words that name `confirm`.
    async fn template_changing<A: CommandArgs>(
        &self,
        arguments: Map<String, Value>,
        mode: impl Fn(&A) -> WriteMode,
        make: impl AsyncFn(&Client, &A) -> holst_core::Result<Changed>,
    ) -> CallToolResult {
        self.parsed(arguments, async |args: A| {
            let call = async |client: HandedClient| {
                let changed = make(&client, &args).await?;
                said(changed.outcome.refusal(), || {
                    self.template_changed_text(&changed)
                })
            };
            self.by_mode(mode(&args), call).await
        })
        .await
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
                .map(|count| format!("{count} templates")),
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
