//! `holst storage`: what holst keeps on this machine, and cleaning it up.

mod render;
#[cfg(test)] mod tests;

use std::sync::Arc;

use holst_core::{
    command::{
        storage::{self, StorageVerb},
        table,
    },
    effect::{clock::Clock, settings::SessionLocation},
    storage::{local_data::DataRoots, session_store::SessionStoreKind},
    system::clock::SystemClock,
};
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    output::OutputFormat,
    run::{
        RunSettings,
        local_data::{data_roots, forgotten},
    },
};

async fn roots(run: &RunSettings) -> DataRoots {
    let store = match run.location() {
        SessionLocation::System { .. } => {
            run.load().await.ok().flatten().map(|(_, kind)| kind)
        }
        SessionLocation::Dir(_) => None,
    };
    data_roots(run, store)
}

/// Runs a command of the `storage` group with the arguments the command line
/// gave it.
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
                Some(forgotten(run.clear().await?))
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
