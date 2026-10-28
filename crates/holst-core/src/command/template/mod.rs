//! The `template` group: list, show, make, replace and delete board templates.

mod create;
mod lookup;

use futures_util::future::join_all;
use holst_macros::CommandArgs;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    command::{
        change::{ChangeReport, settle},
        table::{
            Command,
            CommandArgs,
            Confirmation,
            EnumArgument,
            TextOrFile,
            not_value,
        },
    },
    domain::{
        lookup::choice,
        template::{Template, TemplateCategory, Templates, category_slug},
    },
    effect::{board_write::WriteMode, request::ApiRequest},
    error::{
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
    },
    remote::{api, client::Client, socket::write::connector},
};

const EDIT: &str = "template edit";
const LS: &str = "template ls";
/// The verbs of `holst template`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum TemplateVerb {
    Ls,
    Show,
    Create,
    Edit,
    Rm,
}

impl TemplateVerb {
    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            TemplateVerb::Ls => &LsArgs::COMMAND,
            TemplateVerb::Show => &ShowArgs::COMMAND,
            TemplateVerb::Create => &CreateArgs::COMMAND,
            TemplateVerb::Edit => &EditArgs::COMMAND,
            TemplateVerb::Rm => &RemoveArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Template,
    verb = "ls",
    risk = Read,
    about = "List templates or their categories",
    help = "List templates or, with --view categories, their categories.\n\
            It prints one line per template with its id; give the id or the name in the next command.",
    example = "holst tpl ls",
    example = "holst tpl ls --view standard --category strategic-planning",
    example = "holst tpl ls --view workspace -w easy"
)]
pub struct LsArgs {
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "Where names are looked up and new things go: a workspace id or part of its name; all of yours by default"
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "view",
        help = "Which templates: mine, workspace, standard or categories; mine by default"
    )]
    pub view: Option<TemplateView>,
    #[argument(
        value_name = "slug",
        help = "The template category: slug (strategic-planning), id or name; with --view standard; all by default"
    )]
    pub category: Option<String>,
}

/// Which templates `template ls` lists.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TemplateView {
    #[default]
    Mine,
    Workspace,
    Standard,
    Categories,
}

impl EnumArgument for TemplateView {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Template,
    verb = "show",
    risk = Read,
    about = "Show a template's content",
    help = "Show a template's content.\n\
            It prints the name, description and the objects the template holds.",
    example = "holst tpl show \"Retro\"",
    example = "holst tpl show \"Retro\" -o json"
)]
pub struct ShowArgs {
    #[argument(positional, help = "The template: id or name")]
    pub template: String,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Template,
    verb = "create",
    risk = Write,
    about = "Save a board or a part of it as a template",
    help = "Save a board or a part of it as a template.\n\
            The objects are converted to the web app's format and a thumbnail drawn by holst is uploaded. It prints the new template with its id.",
    example = "holst tpl create \"Roadmap\" --name \"Quarter plan\" --for workspace",
    example = "holst tpl create \"Roadmap\" --name \"Q3 frame\" --frame \"Q3\""
)]
pub struct CreateArgs {
    #[argument(positional, help = "The board to take the objects from")]
    pub board: String,
    #[argument(
        letter = 'w',
        value_name = "ws",
        help = "The workspace a new thing goes to: id or part of its name; needed when you have several"
    )]
    pub workspace: Option<String>,
    #[argument(value_name = "name", help = "The template's name; required")]
    pub name: String,
    #[argument(
        value_name = "text",
        help = "The template's description; none by default"
    )]
    pub description: Option<String>,
    #[argument(
        value_name = "who",
        help = "Who can use it: me or workspace; me by default"
    )]
    // imperative: the flag is named `for`, a keyword
    pub r#for: Option<Audience>,
    #[argument(
        value_name = "frame",
        help = "An area as a frame: id, link, title or part of a title"
    )]
    pub frame: Option<String>,
    #[argument(
        value_name = "object",
        help = "An object, repeat for several: id, link, @N or the first 8 characters of its id"
    )]
    pub object: Vec<String>,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// Who can use a template.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Audience {
    #[default]
    Me,
    Workspace,
}

impl EnumArgument for Audience {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Template,
    verb = "edit",
    risk = Destroy,
    about = "Replace a template's body",
    help = "Replace a template's body.\n\
            The old body is lost, so it needs -y; without it only the dry run is shown.",
    example = "holst tpl edit \"Retro\" --body @body.json -y",
    example = "holst tpl edit \"Retro\" --body @body.json -n"
)]
pub struct EditArgs {
    #[argument(positional, help = "The template: id or name")]
    pub template: String,
    #[argument(
        value_name = "body",
        help = "The new body: a JSON string, @file to read a file, - for stdin; required"
    )]
    pub body: TextOrFile,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Template,
    verb = "rm",
    risk = Destroy,
    about = "Delete a template for good",
    help = "Delete a template for good; templates have no trash.\n\
            Other people lose it too, so it needs -y; without it only the dry run is shown.",
    example = "holst tpl rm \"Retro\" -n",
    example = "holst tpl rm \"Retro\" -y"
)]
pub struct RemoveArgs {
    #[argument(positional, help = "The template: id or name")]
    pub template: String,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// The templates, or the categories, a listing found.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub view: TemplateView,
    pub templates: Vec<ListedTemplate>,
    pub categories: Vec<TemplateCategory>,
}

/// A template with where it was listed.
#[derive(Debug, Clone, Serialize)]
pub struct ListedTemplate {
    /// The workspace or category it was listed under; none for your own.
    pub under: Option<String>,
    #[serde(flatten)]
    pub template: Template,
}

/// What a change did to its template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Edit,
    Delete,
}

/// A change of a template: what it touched and whether it went out.
#[derive(Debug, Serialize)]
pub struct Changed {
    pub action: Action,
    pub id: Option<String>,
    pub name: Option<String>,
    /// How many objects a created template holds.
    pub objects: Option<usize>,
    pub outcome: ChangeReport,
}

impl Changed {
    /// One sentence on what the change did, or what it would do.
    #[must_use]
    pub fn phrase(&self) -> String {
        let would = !matches!(self.outcome, ChangeReport::Sent);
        let name = self.name.as_deref().unwrap_or_default();
        let id = self
            .id
            .as_deref()
            .filter(|_| !would)
            .map(|id| format!(" ({id})"))
            .unwrap_or_default();
        let objects = self
            .objects
            .map(|count| format!(" with {count} objects"))
            .unwrap_or_default();
        let (verb, part) = match (self.action, would) {
            (Action::Create, false) => ("created", "template"),
            (Action::Create, true) => ("would create", "template"),
            (Action::Edit, false) => ("replaced", "the body of template"),
            (Action::Edit, true) => ("would replace", "the body of template"),
            (Action::Delete, false) => ("deleted", "template"),
            (Action::Delete, true) => ("would delete", "template"),
        };
        format!("{verb} {part} {name}{id}{objects}")
    }
}

/// Lists templates, or the categories of the ready-made ones.
///
/// # Errors
/// A workspace or category that is not found, `--category` outside the
/// standard view, and errors of the requests.
pub async fn ls(client: &Client, args: &LsArgs) -> Result<Listing> {
    let view = args.view.unwrap_or_default();
    match (view, &args.category) {
        (TemplateView::Standard, _) | (_, None) => listing(client, view, args).await,
        (
            TemplateView::Mine | TemplateView::Workspace | TemplateView::Categories,
            Some(_),
        ) => Err(not_value(
            LS,
            "category",
            "left out unless --view is standard",
        )),
    }
}

async fn listing(
    client: &Client,
    view: TemplateView,
    args: &LsArgs,
) -> Result<Listing> {
    let (templates, categories) = match view {
        TemplateView::Mine => (own(client).await?, Vec::new()),
        TemplateView::Workspace => (
            in_workspaces(client, args.workspace.as_deref()).await?,
            Vec::new(),
        ),
        TemplateView::Standard => (
            standard(client, args.category.as_deref()).await?,
            Vec::new(),
        ),
        TemplateView::Categories => (
            Vec::new(),
            api::template::standard_categories(client)
                .await?
                .categories()
                .cloned()
                .collect(),
        ),
    };
    Ok(Listing {
        view,
        templates,
        categories,
    })
}

/// A template's content, by id or name.
///
/// # Errors
/// A template that is not found or is named by several, and errors of the
/// requests.
pub async fn show(client: &Client, args: &ShowArgs) -> Result<Template> {
    lookup::found(client, &args.template)
        .await
        .map(|(_, template)| template)
}

/// Makes a template of a board, a frame or objects of it. A thumbnail drawn of
/// the same part is uploaded first. A dry run draws and uploads nothing.
///
/// # Errors
/// A blank name, a board, workspace, frame or object that is not found, a part
/// that holds no object, and errors of the requests.
pub async fn create(client: &Client, args: &CreateArgs) -> Result<Changed> {
    create::created(client, connector(), args).await
}

/// Replaces the body of a template with a JSON object of the new fields.
///
/// # Errors
/// A body that is not a JSON object, a template that is not found or is named
/// by several, and errors of the requests.
pub async fn edit(client: &Client, args: &EditArgs) -> Result<Changed> {
    let body = serde_json::from_str::<Map<String, Value>>(&args.body.0)
        .map_err(|_| not_value(EDIT, "body", "a JSON object"))?;
    let (id, current) = lookup::found(client, &args.template).await?;
    let template = Template::try_from(body)?;
    let unconfirmed = Unconfirmed::new(
        format!(
            "replacing the body of template {}",
            current.name.as_deref().unwrap_or(&id)
        ),
        ChangeRisk::Irreversible,
    );
    carry(
        client,
        Plan {
            request: api::template::update_request(&id, &template)?,
            action: Action::Edit,
            name: current.name,
            id: Some(id),
            objects: None,
            unconfirmed: Some(unconfirmed),
        },
        (args.dry_run, args.confirm),
    )
    .await
}

/// Deletes a template for good; templates have no trash.
///
/// # Errors
/// A template that is not found or is named by several, and errors of the
/// requests.
pub async fn rm(client: &Client, args: &RemoveArgs) -> Result<Changed> {
    let (id, current) = lookup::found(client, &args.template).await?;
    let unconfirmed = Unconfirmed::new(
        format!(
            "deleting template {} for good",
            current.name.as_deref().unwrap_or(&id)
        ),
        ChangeRisk::Irreversible,
    );
    carry(
        client,
        Plan {
            request: api::template::delete_request(&id)?,
            action: Action::Delete,
            name: current.name,
            id: Some(id),
            objects: None,
            unconfirmed: Some(unconfirmed),
        },
        (args.dry_run, args.confirm),
    )
    .await
}

fn listed_under(
    under: Option<String>,
    templates: Templates,
) -> impl Iterator<Item = ListedTemplate> {
    templates
        .templates
        .unwrap_or_default()
        .into_iter()
        .map(move |template| ListedTemplate {
            under: under.clone(),
            template,
        })
}

async fn own(client: &Client) -> Result<Vec<ListedTemplate>> {
    Ok(listed_under(None, api::template::personal(client).await?).collect())
}

/// The workspace's templates, or those of every workspace of the account.
async fn in_workspaces(
    client: &Client,
    reference: Option<&str>,
) -> Result<Vec<ListedTemplate>> {
    let all = crate::command::lookup::workspaces(client).await?;
    let chosen = match reference {
        Some(reference) => vec![choice::workspace(&all, Some(reference))?],
        None => all,
    };
    let listings = join_all(
        chosen
            .iter()
            .map(|workspace| api::template::workspace(client, &workspace.id)),
    )
    .await;
    chosen
        .into_iter()
        .zip(listings)
        .map(|(workspace, listing)| {
            listing.map(|templates| listed_under(Some(workspace.name), templates))
        })
        .flatten_ok()
        .collect()
}

/// The ready-made templates of the category, or of every category.
async fn standard(
    client: &Client,
    category: Option<&str>,
) -> Result<Vec<ListedTemplate>> {
    let all = api::template::standard_categories(client).await?;
    let slugs: Vec<String> = match category {
        Some(reference) => {
            vec![category_slug(all.categories(), reference)?.to_owned()]
        }
        None => all.categories().map(|listed| listed.slug.clone()).collect(),
    };
    let listings = join_all(
        slugs
            .iter()
            .map(|slug| api::template::standard_by_category(client, slug)),
    )
    .await;
    slugs
        .into_iter()
        .zip(listings)
        .map(|(slug, listing)| {
            listing.map(|templates| listed_under(Some(slug), templates))
        })
        .flatten_ok()
        .collect()
}

struct Plan {
    request: ApiRequest,
    action: Action,
    id: Option<String>,
    name: Option<String>,
    objects: Option<usize>,
    unconfirmed: Option<Unconfirmed>,
}

/// Sends the plan, reports it for a dry run, or holds it for want of its
/// confirmation (a dry run needs none).
async fn carry(
    client: &Client,
    plan: Plan,
    (dry_run, confirm): (WriteMode, Confirmation),
) -> Result<Changed> {
    let Plan {
        request,
        action,
        id,
        name,
        objects,
        unconfirmed,
    } = plan;
    let (outcome, reply) =
        settle(client, request, unconfirmed, (dry_run, confirm)).await?;
    let reply = reply.unwrap_or(Value::Null);
    let text = |key| reply.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(Changed {
        action,
        id: id.or_else(|| text("id")),
        name: name.or_else(|| text("name")),
        objects,
        outcome,
    })
}

#[cfg(test)] mod tests;
