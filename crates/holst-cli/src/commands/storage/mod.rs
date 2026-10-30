//! `holst storage`: what holst keeps on this machine, and cleaning it up.

mod render;
#[cfg(test)] mod tests;

use std::sync::Arc;

use holst_core::{
    command::{
        storage::{self, StorageVerb},
        table,
    },
    domain::session::Session,
    effect::{clock::Clock, settings::SessionLocation},
    storage::{local_data::DataRoots, session_store::SessionStoreKind},
    system::clock::SystemClock,
};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    output::{self, OutputFormat},
    run::{
        RunSettings,
        local_data::{data_roots, forgotten},
    },
};

/// The store the saved login was read from; a failed read goes to `warn`.
fn store_read(
    loaded: Result<Option<(Session, SessionStoreKind)>>,
    warn: impl FnOnce(&Error),
) -> Option<SessionStoreKind> {
    loaded
        .inspect_err(warn)
        .ok()
        .flatten()
        .map(|(_, kind)| kind)
}

async fn roots(run: &RunSettings) -> DataRoots {
    let warn = |err: &Error| output::warn_with("the saved login was not read", err);
    let store = match run.location() {
        SessionLocation::System { .. } => store_read(run.load().await, warn),
        SessionLocation::Dir(_) => None,
    };
    data_roots(run, store)
}

pub(crate) async fn run(
    verb: StorageVerb,
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let roots = roots(run).await;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    match verb {
        StorageVerb::Ls => {
            let _args: storage::LsArgs = table::parse(arguments)?;
            render::listing(format, &storage::ls(&roots, run.cache(), &clock))
        }
        StorageVerb::Clean => {
            let args = table::parse(arguments)?;
            let cleaned = storage::clean(&roots, (run.cache(), &clock), &args);
            let forgot = if cleaned.forgets_login() {
                Some(forgotten(run.clear(None).await?))
            } else {
                None
            };
            let gone: Option<&[SessionStoreKind]> =
                forgot.as_ref().map(|(gone, _)| gone.as_slice());
            render::cleaned(format, &cleaned, gone)?;
            let refused = cleaned.report.refusal().cloned().map(Error::Unconfirmed);
            let failed =
                cleaned
                    .failed
                    .into_iter()
                    .next()
                    .map(|failed| Error::NotCleaned {
                        name: failed.name,
                        error: failed.error,
                    });
            refused.or(failed).map_or_else(
                || forgot.and_then(|(_, kept)| kept).map_or(Ok(()), Err),
                Err,
            )
        }
    }
}
