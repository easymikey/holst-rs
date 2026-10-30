//! The account's cache of a run.

use holst_core::{
    effect::settings::CacheSettings,
    remote::client::Client,
    storage::cache::Cache,
};

use crate::terminal::awaiting::optional;

pub(crate) async fn account_cache(
    client: &Client,
    settings: &CacheSettings,
) -> Option<Cache> {
    let asking = settings
        .root
        .as_ref()
        .map(|_| holst_core::command::account::known_account_id(client));
    let user = optional(asking).await.flatten();
    user.and_then(|user| Cache::from_settings(settings, &user, client.clock()))
}
