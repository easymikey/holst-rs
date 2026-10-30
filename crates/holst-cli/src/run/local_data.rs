//! Forgetting and locating local data: the login, the cache, old leftovers.

use std::path::Path;

use holst_core::{
    domain::text::{joined, printable, size_text},
    effect::settings::SessionLocation,
    storage::{
        cache::Cache,
        local_data::{self, CleanMode, DataRoots},
        session_store::{LoginRemoval, SessionStoreKind},
    },
};

use crate::{error::Error, output, run::RunSettings};

pub(crate) fn forgotten(
    cleared: Vec<(SessionStoreKind, holst_core::Result<LoginRemoval>)>,
) -> (Vec<SessionStoreKind>, Option<Error>) {
    cleared
        .into_iter()
        .fold(
            (Vec::new(), None),
            |(mut gone, kept), (kind, removal)| match removal {
                Ok(LoginRemoval::Removed) => {
                    gone.push(kind);
                    (gone, kept)
                }
                Ok(LoginRemoval::Absent) => (gone, kept),
                Err(error) => {
                    let error = Error::NotForgotten {
                        store: kind.as_str(),
                        error,
                    };
                    (gone, kept.or(Some(error)))
                }
            },
        )
}

pub(crate) fn forget_cache(run: &RunSettings, user: Option<&str>) {
    Cache::forget(run.cache(), user)
        .err()
        .into_iter()
        .for_each(|err| {
            output::warn_with("the board cache was not removed", &Error::Holst(err));
        });
}

/// Where holst keeps data, for `holst storage` and the note after logout; the
/// login is the store it was read from, if known.
pub(crate) fn data_roots(
    run: &RunSettings,
    store: Option<SessionStoreKind>,
) -> DataRoots {
    DataRoots {
        home: run.home().map(Path::to_path_buf),
        location: run.location(),
        session_store: store,
        caches: run.cache().roots.clone(),
    }
}

pub(crate) fn leftovers_note(run: &RunSettings) -> Option<String> {
    let location = run.location();
    // With `HOLST_HOME` the real home is not this run's business.
    let roots = DataRoots {
        home: if matches!(location, SessionLocation::System { .. }) {
            run.home().map(Path::to_path_buf)
        } else {
            None
        },
        ..data_roots(run, None)
    };
    let left = local_data::removable_places(&roots, CleanMode::Leftovers);
    (!left.is_empty()).then(|| {
        format!(
            "{} of old holst data is left in {}; remove it with `holst storage clean`",
            size_text(left.iter().map(|place| place.bytes).sum::<u64>()),
            joined(left.iter().map(|place| printable(&place.name)), ", ")
        )
    })
}
