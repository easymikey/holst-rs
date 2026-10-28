use holst_macros::{delete, get, patch, post};

use crate::{
    domain::template::{Template, TemplateCategories, Templates},
    error::Result,
    remote::client::Client,
};

/// Categories of the ready-made templates.
#[get("board_template/categories/standard", response = typed)]
pub async fn standard_categories(client: &Client) -> Result<TemplateCategories>;

/// The ready-made templates of a category.
#[get("board_template/categories/{category}/standard_templates", response = typed)]
pub async fn standard_by_category(client: &Client, category: &str)
-> Result<Templates>;

/// Personal templates.
#[get("board_template/personal", response = typed)]
pub async fn personal(client: &Client) -> Result<Templates>;

/// A workspace's templates. Not verified live.
#[get("board_template/workspace/{workspace_id}", response = typed)]
pub async fn workspace(client: &Client, workspace_id: &str) -> Result<Templates>;

/// A template's content.
#[get("board_template/get", query(board_template_id = template_id), response = typed)]
pub async fn get(client: &Client, template_id: &str) -> Result<Template>;

/// Delete a template.
#[delete("board_template/delete", query(board_template_id = template_id), response = typed, request_only)]
pub async fn delete(client: &Client, template_id: &str) -> Result<IgnoredBody>;

/// Create a template.
#[post("board_template/create", body = template, response = typed, request_only)]
pub async fn create(client: &Client, template: &Template) -> Result<IgnoredBody>;

/// Update a template. Not verified live.
#[patch("board_template/update", query(board_template_id = template_id), body = template, response = typed, request_only)]
pub async fn update(
    client: &Client,
    template_id: &str,
    template: &Template,
) -> Result<IgnoredBody>;
