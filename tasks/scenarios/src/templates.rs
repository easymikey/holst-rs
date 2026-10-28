//! Templates on either surface.

use serde_json::{Value, json};

use crate::{
    error::ScenarioError,
    holst::{Holst, On, SendMode, fields},
    mcp,
    observation::Observation,
    surface::{Surface, Words, arguments},
    then::Then,
};

const NO_SUCH_ID: &str = "00000000-0000-4000-8000-0000000000ee";

impl On<'_> {
    /// Reads the user's own templates.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_templates(&self) -> Result<Observation, ScenarioError> {
        self.reading(&["template", "ls"], "template", json!({ "action": "ls" }))
            .await
    }

    /// Reads the categories of ready-made templates.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_template_categories(&self) -> Result<Observation, ScenarioError> {
        self.reading(
            &["template", "ls", "--view", "categories"],
            "template",
            json!({ "action": "ls", "view": "categories" }),
        )
        .await
    }

    /// Reads the ready-made templates of the category `category` (a slug, id
    /// or name).
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_standard_templates(
        &self,
        category: &str,
    ) -> Result<Observation, ScenarioError> {
        self.reading(
            &[
                "template",
                "ls",
                "--view",
                "standard",
                "--category",
                category,
            ],
            "template",
            json!({ "action": "ls", "view": "standard", "category": category }),
        )
        .await
    }

    /// Reads the ready-made templates of the first category the listing
    /// showed, by the id or alias the surface handed out there.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_standard_templates_of_listed_category(
        &self,
    ) -> Result<Observation, ScenarioError> {
        match self.surface {
            Surface::Cli => {
                let listing = self
                    .reading(
                        &["template", "ls", "--view", "categories"],
                        "template",
                        json!({}),
                    )
                    .await?;
                let id = listing
                    .shown()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .and_then(|reply| {
                        reply
                            .pointer("/categories/0/id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    });
                self.list_standard_templates(&id.unwrap_or_default()).await
            }
            Surface::Mcp => {
                let observation = mcp::call_then(
                    self.holst.command("holst-mcp")?,
                    "template",
                    fields(json!({ "action": "ls", "view": "categories" })),
                    // One line per category, with its alias.
                    |reply| {
                        reply
                            .text
                            .lines()
                            .next()
                            .and_then(|line| {
                                line.split_whitespace()
                                    .find(|word| word.starts_with('@'))
                            })
                            .map(|alias| {
                                (
                                    "template",
                                    fields(json!({
                                        "action": "ls",
                                        "view": "standard",
                                        "category": alias,
                                    })),
                                )
                            })
                    },
                    |reply| Observation::Shown(reply.text.clone()),
                )
                .await?;
                self.holst.checked(observation)
            }
        }
    }

    /// Reads the templates shared with the workspace `workspace`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn list_workspace_templates(
        &self,
        workspace: &str,
    ) -> Result<Observation, ScenarioError> {
        self.reading(
            &["template", "ls", "--view", "workspace", "-w", workspace],
            "template",
            json!({ "action": "ls", "view": "workspace", "workspace": workspace }),
        )
        .await
    }

    /// Shows a template by `reference`: its name, id or `@N`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn show_template(
        &self,
        reference: &str,
    ) -> Result<Observation, ScenarioError> {
        self.reading(
            &["template", "show", reference],
            "template",
            json!({ "action": "show", "template": reference }),
        )
        .await
    }

    /// Makes a template called `name` of `part` of the board `board`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn create_template(
        &self,
        board: &str,
        name: &str,
        part: TemplatePart<'_>,
        audience: TemplateAudience,
    ) -> Result<Observation, ScenarioError> {
        let (frame, objects): (Option<&str>, Vec<String>) = match part {
            TemplatePart::Whole => (None, Vec::new()),
            TemplatePart::Frame(frame) => (Some(frame), Vec::new()),
            TemplatePart::Objects(numbers) => (
                None,
                numbers
                    .iter()
                    .filter_map(|number| self.holst.fake().object_id(board, *number))
                    .collect(),
            ),
        };
        let dry_run = self.mode == SendMode::DryRun;
        let line: Vec<String> = [
            "template",
            "create",
            board,
            "--name",
            name,
            "--for",
            audience.word(),
        ]
        .map(str::to_owned)
        .into_iter()
        .chain(
            frame
                .into_iter()
                .flat_map(|frame| ["--frame".to_owned(), frame.to_owned()]),
        )
        .chain(
            objects
                .iter()
                .flat_map(|id| ["--object".to_owned(), id.clone()]),
        )
        .chain(dry_run.then(|| "-n".to_owned()))
        .collect();
        let words: Vec<&str> = line.iter().map(String::as_str).collect();
        let arguments = Value::Object(
            [
                ("action", json!("create")),
                ("board", json!(board)),
                ("name", json!(name)),
                ("for", json!(audience.word())),
                ("dry_run", json!(dry_run)),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .chain(frame.map(|frame| ("frame".to_owned(), json!(frame))))
            .chain((!objects.is_empty()).then(|| ("object".to_owned(), json!(objects))))
            .collect(),
        );
        self.changing(&words, "template", arguments).await
    }

    /// Replaces the body of the own template called `template` with `body`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn replace_template_body(
        &self,
        template: &str,
        body: &str,
    ) -> Result<Observation, ScenarioError> {
        let id = self
            .holst
            .fake()
            .template_id(template)
            .unwrap_or_else(|| NO_SUCH_ID.to_owned());
        let path = self.holst.work_file("body.json");
        std::fs::write(&path, body)?;
        let from_file = format!("@{}", path.display());
        let words = Words {
            positional: vec![("template", &id)],
            options: vec![("body", &from_file)],
            confirms: true,
            writes: true,
            ..Words::default()
        };
        let line = self.template_line("edit", &words);
        let line: Vec<&str> = line.iter().map(String::as_str).collect();
        let arguments = self.admin_arguments(json!({
            "action": "edit",
            "template": id,
            "body": body,
        }));
        self.changing(&line, "template", arguments).await
    }

    /// Deletes the own template called `name`.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn delete_template(
        &self,
        name: &str,
    ) -> Result<Observation, ScenarioError> {
        let id = self
            .holst
            .fake()
            .template_id(name)
            .unwrap_or_else(|| NO_SUCH_ID.to_owned());
        let words = Words {
            positional: vec![("template", &id)],
            confirms: true,
            writes: true,
            ..Words::default()
        };
        let line = self.template_line("rm", &words);
        let line: Vec<&str> = line.iter().map(String::as_str).collect();
        let arguments = self.admin_arguments(json!({
            "action": "rm",
            "template": id,
        }));
        self.changing(&line, "template", arguments).await
    }

    /// Runs a template command on the template "Retro" or the board "Plan"
    /// the scenario may have set up.
    ///
    /// # Errors
    /// As [`ScenarioError`].
    pub async fn run_template_command(
        &self,
        command: TemplateCommand,
    ) -> Result<Observation, ScenarioError> {
        match command {
            TemplateCommand::List => self.list_templates().await,
            TemplateCommand::Show => self.show_template("Retro").await,
            TemplateCommand::Create => {
                self.create_template(
                    "Plan",
                    "Roadmap",
                    TemplatePart::Whole,
                    TemplateAudience::Me,
                )
                .await
            }
            TemplateCommand::Edit => self.replace_template_body("Retro", "{}").await,
            TemplateCommand::Remove => self.delete_template("Retro").await,
        }
    }

    /// The command line of a change that cannot be undone: confirmed once
    /// confirmed, only shown for a dry run.
    fn template_line(&self, command: &str, words: &Words<'_>) -> Vec<String> {
        ["template", command]
            .map(str::to_owned)
            .into_iter()
            .chain(arguments(words, self.mode).0)
            .collect()
    }
}

/// Which part of a board a template is made of.
#[derive(Debug, Clone, Copy)]
pub enum TemplatePart<'a> {
    Whole,
    /// The frame with this title and what lies in it.
    Frame(&'a str),
    /// The objects with these numbers on the fixture board, from 1.
    Objects(&'a [u32]),
}

/// Who can use a template.
#[derive(Debug, Clone, Copy)]
pub enum TemplateAudience {
    Me,
    Workspace,
}

impl TemplateAudience {
    const fn word(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Workspace => "workspace",
        }
    }
}

/// A template command a scenario runs to see what it needs.
#[derive(Debug, Clone, Copy)]
pub enum TemplateCommand {
    List,
    Show,
    Create,
    Edit,
    Remove,
}

impl Then<'_> {
    /// Expects the surface to have shown `words`.
    ///
    /// # Panics
    /// When it showed something else.
    pub fn shows(&self, words: &str) {
        assert!(
            matches!(self.observation, Observation::Shown(text) if text.contains(words)),
            "expected a reply showing {words:?}, got {:?}",
            self.observation
        );
    }

    /// Expects a change that went through.
    ///
    /// # Panics
    /// When the change failed or was only shown.
    pub fn succeeded(&self) {
        assert_eq!(self.observation, &Observation::Done);
    }
}

/// Expects things of the user's own templates.
#[must_use]
pub fn templates_of(holst: &Holst) -> TemplatesThen {
    TemplatesThen {
        names: holst.fake().personal_templates(),
    }
}

/// Expects things of the templates shared with a workspace.
#[must_use]
pub fn workspace_templates_of(holst: &Holst, workspace: &str) -> TemplatesThen {
    TemplatesThen {
        names: holst.fake().workspace_templates(workspace),
    }
}

/// Expects things of the template called `name`.
#[must_use]
pub fn template_of(holst: &Holst, name: &str) -> TemplateThen {
    TemplateThen {
        name: name.to_owned(),
        fields: holst.fake().template_of(name),
    }
}

/// Expects things of the pictures the server got outside any board.
#[must_use]
pub fn thumbnails_of(holst: &Holst) -> ThumbnailsThen {
    ThumbnailsThen {
        count: holst.fake().thumbnails().len(),
        uploads: holst.fake().uploads().len(),
    }
}

#[derive(Debug)]
pub struct ThumbnailsThen {
    count: usize,
    uploads: usize,
}

impl ThumbnailsThen {
    /// Expects `count` pictures outside any board, and no other upload.
    ///
    /// # Panics
    /// When there are other numbers.
    pub fn are(&self, count: usize) {
        assert_eq!(
            (self.count, self.uploads),
            (count, count),
            "thumbnails and all uploads"
        );
    }
}

#[derive(Debug)]
pub struct TemplateThen {
    name: String,
    fields: Option<Value>,
}

impl TemplateThen {
    /// Expects the objects of the template to be of exactly these kinds, as the
    /// web client names them (`sticker`, `frame`), in any order.
    ///
    /// # Panics
    /// When they differ.
    pub fn holds_objects_of_kinds(&self, kinds: &[&str]) {
        let mut seen = self.kinds();
        seen.sort();
        let mut expected: Vec<String> =
            kinds.iter().map(|kind| (*kind).to_owned()).collect();
        expected.sort();
        assert_eq!(
            seen, expected,
            "the objects of the template {:?}",
            self.name
        );
    }

    /// Expects the field `field` of the template to hold `value`.
    ///
    /// # Panics
    /// When it does not.
    pub fn has_field(&self, field: &str, value: &Value) {
        assert_eq!(
            self.fields.as_ref().and_then(|fields| fields.get(field)),
            Some(value),
            "the field {field:?} of the template {:?}",
            self.name
        );
    }

    /// Expects the template not to have the field `field`.
    ///
    /// # Panics
    /// When it has it.
    pub fn lacks_field(&self, field: &str) {
        assert_eq!(
            self.fields.as_ref().and_then(|fields| fields.get(field)),
            None,
            "the field {field:?} of the template {:?}",
            self.name
        );
    }

    // The `data` field holds the objects as the text of a JSON array.
    fn kinds(&self) -> Vec<String> {
        self.fields
            .as_ref()
            .and_then(|fields| fields.pointer("/data/data"))
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Vec<Value>>(text).ok())
            .into_iter()
            .flatten()
            .filter_map(|object| {
                object
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct TemplatesThen {
    names: Vec<String>,
}

impl TemplatesThen {
    /// Expects exactly these templates, in any order.
    ///
    /// # Panics
    /// When they differ.
    pub fn are(&self, names: &[&str]) {
        let mut seen = self.names.clone();
        seen.sort();
        let mut expected: Vec<String> =
            names.iter().map(|name| (*name).to_owned()).collect();
        expected.sort();
        assert_eq!(seen, expected, "the templates");
    }

    /// Expects a template of this name.
    ///
    /// # Panics
    /// When there is none.
    pub fn has(&self, name: &str) {
        assert!(
            self.names.iter().any(|have| have == name),
            "expected the template {name:?}, the own templates are {:?}",
            self.names
        );
    }

    /// Expects no template of this name.
    ///
    /// # Panics
    /// When there is one.
    pub fn lacks(&self, name: &str) {
        assert!(
            self.names.iter().all(|have| have != name),
            "expected no template {name:?}, the own templates are {:?}",
            self.names
        );
    }
}
