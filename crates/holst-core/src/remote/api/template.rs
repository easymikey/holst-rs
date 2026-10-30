use holst_macros::{delete, get, patch, post};

use crate::{
    domain::template::{Template, TemplateCategories, Templates},
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

/// Categories of the ready-made templates.
#[get("board_template/categories/standard", class = Read, response = typed)]
pub async fn standard_categories(client: &Client) -> Result<TemplateCategories>;

/// The ready-made templates of a category.
#[get("board_template/categories/{category}/standard_templates", class = Read, response = typed)]
pub async fn standard_by_category(client: &Client, category: &str)
-> Result<Templates>;

/// Personal templates.
#[get("board_template/personal", class = Read, response = typed)]
pub async fn personal(client: &Client) -> Result<Templates>;

/// A workspace's templates. Not verified live.
#[get("board_template/workspace/{workspace_id}", class = Read, response = typed)]
pub async fn workspace(client: &Client, workspace_id: &str) -> Result<Templates>;

/// A template's content.
#[get("board_template/get", class = Read, query(board_template_id = template_id), response = typed)]
pub async fn get(client: &Client, template_id: &str) -> Result<Template>;

/// Delete a template.
#[delete("board_template/delete", class = Write, query(board_template_id = template_id), response = typed, request_only)]
pub async fn delete(client: &Client, template_id: &str) -> Result<Done>;

/// Create a template.
#[post("board_template/create", class = Write, response = typed, request_only)]
pub async fn create(client: &Client, #[body] template: &Template) -> Result<Done>;

/// Update a template. Not verified live.
#[patch("board_template/update", class = Write, query(board_template_id = template_id), response = typed, request_only)]
pub async fn update(
    client: &Client,
    template_id: &str,
    #[body] template: &Template,
) -> Result<Done>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[
    STANDARD_CATEGORIES,
    STANDARD_BY_CATEGORY,
    PERSONAL,
    WORKSPACE,
    GET,
    DELETE,
    CREATE,
    UPDATE,
];
