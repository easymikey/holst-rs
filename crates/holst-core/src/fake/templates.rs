//! The board templates: ready-made ones by category and the user's own.

use serde_json::{Map, Value, json};

use crate::fake::{
    FixtureError,
    FixtureObject,
    board::template_data,
    fixture::unknown,
    http::{Parts, Request, Response, found, object_body, refused},
};

/// A category of ready-made templates the fake holds when it starts.
#[derive(Debug, Clone)]
pub(crate) struct FixtureTemplateCategory {
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) templates: Vec<String>,
}

/// A template of the user's or of a workspace the fake holds when it starts.
#[derive(Debug, Clone)]
pub(crate) struct FixtureTemplate {
    pub(crate) name: String,
    /// The name of the workspace that shares it; none for a personal one.
    pub(crate) workspace: Option<String>,
    pub(crate) objects: Vec<FixtureObject>,
}

#[derive(Debug)]
struct Template {
    id: String,
    name: String,
    fields: Map<String, Value>,
}

impl Template {
    fn json(&self) -> Value {
        let mut shown = self.fields.clone();
        shown.insert("id".to_owned(), json!(self.id));
        shown.insert("name".to_owned(), json!(self.name));
        Value::Object(shown)
    }

    /// The id of the workspace that shares the template, if one does.
    fn shared_with(&self) -> Option<&str> {
        self.fields
            .get("scope")
            .and_then(Value::as_str)
            .filter(|scope| *scope == "workspace")
            .and_then(|_| self.fields.get("workspace_id"))
            .and_then(Value::as_str)
    }
}

#[derive(Debug)]
struct TemplateCategory {
    id: String,
    slug: String,
    name: String,
    templates: Vec<Template>,
}

#[derive(Debug, Default)]
pub(crate) struct Templates {
    categories: Vec<TemplateCategory>,
    personal: Vec<Template>,
    issued: usize,
}

fn listing(templates: &[&Template]) -> Response {
    found(
        &json!({ "board_templates": templates.iter().map(|template| template.json()).collect::<Vec<_>>() }),
    )
}

impl Templates {
    /// Fills the categories and the templates of the user and of workspaces;
    /// `workspaces` maps a workspace name to its id.
    ///
    /// # Errors
    /// [`FixtureError`] when a template is shared with a workspace the fixture
    /// does not hold.
    pub(crate) fn fixture(
        categories: &[FixtureTemplateCategory],
        own: &[FixtureTemplate],
        workspaces: &[(String, String)],
    ) -> Result<Self, FixtureError> {
        let empty = Self::default();
        let with_categories = categories.iter().fold(empty, |mut catalog, fixture| {
            let id = catalog.issue();
            let templates = fixture
                .templates
                .iter()
                .map(|name| catalog.template(name, Map::new()))
                .collect();
            catalog.categories.push(TemplateCategory {
                id,
                slug: fixture.slug.clone(),
                name: fixture.name.clone(),
                templates,
            });
            catalog
        });
        own.iter()
            .try_fold(with_categories, |mut catalog, fixture| {
                let workspace_id = fixture
                    .workspace
                    .as_ref()
                    .map(|name| {
                        workspaces
                            .iter()
                            .find(|(known, _)| known == name)
                            .map(|(_, id)| id.clone())
                            .ok_or_else(|| unknown("workspace", name))
                    })
                    .transpose()?;
                let scope = workspace_id.map_or_else(
                    || json!({ "scope": "personal" }),
                    |id| json!({ "scope": "workspace", "workspace_id": id }),
                );
                let data = (!fixture.objects.is_empty())
                    .then(|| json!({ "data": template_data(&fixture.objects) }));
                let fields = scope
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .chain(data.map(|data| ("data".to_owned(), data)))
                    .collect();
                let template = catalog.template(&fixture.name, fields);
                catalog.personal.push(template);
                Ok(catalog)
            })
    }

    fn issue(&mut self) -> String {
        self.issued += 1;
        format!("00000000-0000-4000-8000-{:012x}", 0xc000 + self.issued)
    }

    fn template(&mut self, name: &str, fields: Map<String, Value>) -> Template {
        Template {
            id: self.issue(),
            name: name.to_owned(),
            fields,
        }
    }

    /// The names of the user's own templates.
    pub(crate) fn personal_names(&self) -> Vec<String> {
        self.personal
            .iter()
            .filter(|template| template.shared_with().is_none())
            .map(|template| template.name.clone())
            .collect()
    }

    /// The names of the templates shared with the workspace `workspace_id`.
    pub(crate) fn workspace_names(&self, workspace_id: &str) -> Vec<String> {
        self.shared(workspace_id)
            .map(|template| template.name.clone())
            .collect()
    }

    fn shared<'a>(
        &'a self,
        workspace_id: &'a str,
    ) -> impl Iterator<Item = &'a Template> {
        self.personal
            .iter()
            .filter(move |template| template.shared_with() == Some(workspace_id))
    }

    /// The id of the user's own template called `name`.
    pub(crate) fn personal_id(&self, name: &str) -> Option<String> {
        self.personal
            .iter()
            .find(|template| template.name == name)
            .map(|template| template.id.clone())
    }

    /// The fields of the template of the user's or of a workspace called
    /// `name`.
    pub(crate) fn json_of(&self, name: &str) -> Option<Value> {
        self.personal
            .iter()
            .find(|template| template.name == name)
            .map(Template::json)
    }

    /// Answers the template endpoints; none for any other request.
    pub(crate) fn respond(&mut self, request: &Request) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "board_template", "categories", "standard"]) => {
                Some(self.categories())
            }
            (
                "GET",
                [
                    "",
                    "gapi",
                    "board_template",
                    "categories",
                    slug,
                    "standard_templates",
                ],
            ) => Some(
                self.categories
                    .iter()
                    .find(|category| category.slug == *slug)
                    .map_or_else(
                        || refused(404, "category-not-found"),
                        |category| {
                            listing(&category.templates.iter().collect::<Vec<_>>())
                        },
                    ),
            ),
            ("GET", ["", "gapi", "board_template", "personal"]) => Some(listing(
                &self
                    .personal
                    .iter()
                    .filter(|template| template.shared_with().is_none())
                    .collect::<Vec<_>>(),
            )),
            ("GET", ["", "gapi", "board_template", "workspace", workspace_id]) => {
                Some(listing(&self.shared(workspace_id).collect::<Vec<_>>()))
            }
            ("PATCH", ["", "gapi", "board_template", "update"]) => {
                Some(parts.query("board_template_id").map_or_else(
                    || refused(400, "bad-request"),
                    |id| self.update(id, request),
                ))
            }
            ("GET", ["", "gapi", "board_template", "get"]) => Some(
                parts
                    .query("board_template_id")
                    .and_then(|id| self.find(id))
                    .map_or_else(|| refused(404, "not-found"), |t| found(&t.json())),
            ),
            ("POST", ["", "gapi", "board_template", "create"]) => {
                Some(self.create(request))
            }
            ("DELETE", ["", "gapi", "board_template", "delete"]) => Some(
                parts
                    .query("board_template_id")
                    .map_or_else(|| refused(400, "bad-request"), |id| self.delete(id)),
            ),
            _ => None,
        }
    }

    fn categories(&self) -> Response {
        found(
            &json!({ "categories": self.categories.iter().map(|category| json!({
            "id": category.id,
            "slug": category.slug,
            "name": category.name,
            "standard": true,
            "count_templates": category.templates.len(),
        })).collect::<Vec<_>>() }),
        )
    }

    fn find(&self, id: &str) -> Option<&Template> {
        self.personal
            .iter()
            .chain(
                self.categories
                    .iter()
                    .flat_map(|category| category.templates.iter()),
            )
            .find(|template| template.id == id)
    }

    fn create(&mut self, request: &Request) -> Response {
        object_body(request).map_or_else(
            || refused(400, "bad-request"),
            |mut fields| {
                let name = fields
                    .remove("name")
                    .and_then(|name| name.as_str().map(str::to_owned))
                    .unwrap_or_default();
                // The server wants the workspace for every scope.
                if fields.get("workspace_id").and_then(Value::as_str).is_none() {
                    refused(400, "invalid-workspace-id")
                } else {
                    let template = self.template(&name, fields);
                    let shown = found(&template.json());
                    self.personal.push(template);
                    shown
                }
            },
        )
    }

    /// Changes the fields of the template of the user or of a workspace that
    /// the query names by `id`.
    fn update(&mut self, id: &str, request: &Request) -> Response {
        match object_body(request) {
            Some(mut changes) => self
                .personal
                .iter_mut()
                .find(|template| template.id == id)
                .map_or_else(
                    || refused(404, "not-found"),
                    |template| {
                        changes
                            .remove("name")
                            .and_then(|name| name.as_str().map(str::to_owned))
                            .into_iter()
                            .for_each(|name| template.name = name);
                        template.fields.extend(changes);
                        found(&template.json())
                    },
                ),
            None => refused(400, "bad-request"),
        }
    }

    fn delete(&mut self, id: &str) -> Response {
        self.personal
            .iter()
            .position(|template| template.id == id)
            .map_or_else(
                || refused(404, "not-found"),
                |at| {
                    self.personal.remove(at);
                    found(&json!({}))
                },
            )
    }
}
