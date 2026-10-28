use holst_macros::get;

use crate::{
    domain::backup::{Backup, Backups},
    error::Result,
    remote::client::Client,
};

/// A board's automatic snapshots.
#[get("backup/list", query(name = channel_id), response = typed)]
pub async fn list(client: &Client, channel_id: &str) -> Result<Backups>;

/// A board snapshot; usual provider:
/// [`crate::domain::backup::DEFAULT_PROVIDER`].
///
/// # Errors
/// [`Client::download`] errors;
/// [`crate::error::http::HttpError::JsonInsteadOfBackup`].
pub async fn download(client: &Client, key: &str, provider: &str) -> Result<Backup> {
    let url = client.base_url().backup_download(key, provider)?;
    Backup::try_from(client.download(url.as_str()).await?)
}
