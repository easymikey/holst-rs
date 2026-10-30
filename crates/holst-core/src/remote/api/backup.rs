use holst_macros::get;

use crate::{
    domain::backup::Backups,
    error::Result,
    remote::{catalogue::Endpoint, client::Client},
};

/// A board's automatic snapshots.
#[get("backup/list", class = Read, query(name = channel_id), response = typed)]
pub async fn list(client: &Client, channel_id: &str) -> Result<Backups>;

pub(crate) const ENDPOINTS: &[Endpoint] = &[LIST];

/// The bytes of a board snapshot, from the provider of a backup listing.
#[get(
    "backup/download",
    unlisted,
    query(key = key, server_provider_type = provider),
    response = bytes
)]
pub async fn download(client: &Client, key: &str, provider: &str) -> Result<Vec<u8>>;

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use crate::{Result, remote::api::backup::download_request};

    #[test]
    fn the_request_carries_key_and_provider() -> Result<()> {
        let request = download_request("a b", "hud")?;
        let query: Vec<_> = request
            .query
            .iter()
            .map(|parameter| (parameter.name.as_str(), parameter.value.as_str()))
            .collect();
        assert_eq!(request.path, "/gapi/backup/download");
        assert_eq!(query, [("key", "a b"), ("server_provider_type", "hud")]);
        Ok(())
    }
}
