//! The `storage` group (CLI only): what holst keeps here, and cleaning it.

use std::sync::Arc;

use holst_macros::CommandArgs;
use serde::{Deserialize, Serialize};

use crate::{
    command::{
        board_load::account_caches,
        change::ChangeReport,
        table::{Command, CommandArgs, Confirmation},
    },
    effect::{
        board_write::WriteMode,
        cache::{CachePort, CacheUsage},
        clock::Clock,
        confirmation::unless_confirmed,
        settings::CacheSettings,
    },
    error::confirmation::{ChangeRisk, Unconfirmed},
    storage::local_data::{self, CleanMode, DataPlace, DataRoots, FilePlace},
};

/// The verbs of `holst storage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum StorageVerb {
    Ls,
    Clean,
}

impl StorageVerb {
    #[must_use]
    pub const fn command(self) -> &'static Command {
        match self {
            StorageVerb::Ls => &LsArgs::COMMAND,
            StorageVerb::Clean => &CleanArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Storage,
    verb = "ls",
    risk = Read,
    surfaces = [Cli],
    block = Commands,
    about = "List what holst keeps on this machine, with sizes",
    help = "List what holst keeps on this machine, with sizes.\n\
            It only reads; nothing is removed.",
    example = "holst storage ls",
    example = "holst storage ls -o json"
)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "parsed from a JSON object"
)]
pub struct LsArgs {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Storage,
    verb = "clean",
    risk = Destroy,
    surfaces = [Cli],
    block = Commands,
    about = "Remove old logins, partial files and stale copies",
    help = "Remove old logins, partial files and stale copies that holst keeps on this machine.\n\
            The board cache and the saved login stay unless --cache or --all says otherwise.\n\
            --all removes the saved login, so it needs -y; without it only the dry run is shown. Plain clean and --cache need no -y.",
    example = "holst storage clean -n",
    example = "holst storage clean --cache",
    example = "holst storage clean --all -y"
)]
pub struct CleanArgs {
    #[argument(help = "Also remove the board cache")]
    pub cache: bool,
    #[argument(
        help = "Also remove the board cache and the saved login here, as `holst auth logout --local-only`"
    )]
    pub all: bool,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

/// Everything holst keeps here, and how much a plain `clean` would free.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub places: Vec<DataPlace>,
    pub removable_bytes: u64,
    pub cache_limit_bytes: Option<u64>,
}

/// Lists the places holst keeps data in; only reads.
#[must_use]
pub fn ls(roots: &DataRoots, cache: &CacheSettings, clock: &Arc<dyn Clock>) -> Listing {
    let places = local_data::places(roots);
    let removable_bytes = local_data::to_remove(&places, CleanMode::Leftovers)
        .iter()
        .map(|place| place.bytes)
        .sum::<u64>()
        .saturating_add(evictable(cache, clock).bytes);
    Listing {
        places,
        removable_bytes,
        cache_limit_bytes: cache.root.as_ref().map(|_| cache.max_bytes),
    }
}

/// A place that could not be removed.
#[derive(Debug, Serialize)]
pub struct NotRemoved {
    pub name: String,
    pub error: String,
}

/// The places a clean removed, or in a dry run would remove.
#[derive(Debug, Serialize)]
pub struct Cleaned {
    pub removed: Vec<FilePlace>,
    pub failed: Vec<NotRemoved>,
    /// The cached boards over the limit or unused for a month.
    pub evicted: Option<CacheUsage>,
    /// A storage clean sends no request, so its plan is `()`.
    #[serde(flatten)]
    pub report: ChangeReport<()>,
    #[serde(skip)]
    scope: CleanMode,
}

impl Cleaned {
    /// Whether the shell has to forget the saved login now.
    #[must_use]
    pub fn forgets_login(&self) -> bool {
        matches!(
            (self.scope, &self.report),
            (CleanMode::All, ChangeReport::Sent)
        )
    }
}

/// Removes old logins, partial files and stale copies; `--cache` and `--all`
/// reach further. `--all` needs the confirmation, a dry run never does.
#[must_use]
pub fn clean(
    roots: &DataRoots,
    (cache, clock): (&CacheSettings, &Arc<dyn Clock>),
    args: &CleanArgs,
) -> Cleaned {
    let scope = match (args.all, args.cache) {
        (true, _) => CleanMode::All,
        (false, true) => CleanMode::Cache,
        (false, false) => CleanMode::Leftovers,
    };
    let sending = args.dry_run == WriteMode::Send;
    let unconfirmed = (scope == CleanMode::All).then(|| {
        Unconfirmed::new(
            "removing the saved login and the board cache",
            ChangeRisk::Irreversible,
        )
    });
    let held = unless_confirmed(
        unconfirmed,
        args.confirm == Confirmation::Given,
        if sending {
            WriteMode::Send
        } else {
            WriteMode::DryRun
        },
    )
    .err();
    let removing = sending && held.is_none();
    let places = local_data::places(roots);
    let results: Vec<_> = local_data::to_remove(&places, scope)
        .into_iter()
        .map(|place| {
            (
                place,
                if removing {
                    local_data::remove(place)
                } else {
                    Ok(())
                },
            )
        })
        .collect();
    let mut failed: Vec<NotRemoved> = results
        .iter()
        .filter_map(|(place, result)| {
            result.as_ref().err().map(|error| NotRemoved {
                name: place.name.clone(),
                error: error.to_string(),
            })
        })
        .collect();
    let removed = results
        .iter()
        .filter(|(_, result)| result.is_ok())
        .map(|(place, _)| (*place).clone())
        .collect();
    let evicted = (scope == CleanMode::Leftovers).then(|| {
        if removing {
            let (usage, errors) = evict(cache, clock);
            failed.extend(errors);
            usage
        } else {
            evictable(cache, clock)
        }
    });
    Cleaned {
        removed,
        failed,
        evicted: evicted.filter(|usage| usage.boards > 0),
        report: match (held, sending) {
            (Some(refusal), _) => ChangeReport::Held {
                request: (),
                needs: format!("{} {}", refusal.change, refusal.risk),
                refusal,
            },
            (None, true) => ChangeReport::Sent,
            (None, false) => ChangeReport::DryRun { request: () },
        },
        scope,
    }
}

fn evictable(settings: &CacheSettings, clock: &Arc<dyn Clock>) -> CacheUsage {
    account_caches(settings, clock)
        .iter()
        .map(CachePort::evictable)
        .sum::<CacheUsage>()
}

fn evict(
    settings: &CacheSettings,
    clock: &Arc<dyn Clock>,
) -> (CacheUsage, Vec<NotRemoved>) {
    let results: Vec<_> = account_caches(settings, clock)
        .iter()
        .map(CachePort::evict)
        .collect();
    let usage = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .copied()
        .sum::<CacheUsage>();
    let failures = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .map(|error| NotRemoved {
            name: "a board cache".to_owned(),
            error: error.to_string(),
        })
        .collect();
    (usage, failures)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "tests build and read real files in temp directories"
    )]
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::{path::PathBuf, sync::Arc};

    use serde_json::{Value, json};

    use crate::{
        Result,
        command::{
            storage::{CleanArgs, Cleaned, clean},
            table::parse,
        },
        effect::{
            clock::{Clock, FixedClock},
            settings::{CacheSettings, SessionLocation},
        },
        storage::local_data::{CONFIG_DIR, DataRoots},
    };

    /// A home with a browser profile in holst's directory, which `clean`
    /// counts as old.
    fn home(name: &str) -> Result<(DataRoots, PathBuf)> {
        let dir = std::env::temp_dir()
            .join(format!("holst-core-storage-{name}-{}", std::process::id()));
        let profile = dir.join(CONFIG_DIR).join("browser").join("Default");
        crate::storage::file::create_dir_all(&profile)?;
        std::fs::write(profile.join("cookies"), b"0123456789")?;
        let roots = DataRoots {
            home: Some(dir.clone()),
            location: SessionLocation::Dir(dir.join(CONFIG_DIR)),
            session_store: None,
            caches: Vec::new(),
        };
        Ok((roots, dir.join(CONFIG_DIR).join("browser")))
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(FixedClock { unix_millis: 0 })
    }

    fn status(done: &Cleaned) -> Option<String> {
        serde_json::to_value(done)
            .ok()?
            .get("status")?
            .as_str()
            .map(str::to_owned)
    }

    fn arguments(given: &Value) -> Result<CleanArgs> {
        parse(given.as_object().cloned().unwrap_or_default())
    }

    #[test]
    fn all_needs_the_confirmation_and_then_forgets_the_login() -> Result<()> {
        let (roots, profile) = home("all")?;
        let cache = CacheSettings::default();
        let on = (&cache, &clock());
        let held = clean(&roots, on, &arguments(&json!({ "all": true }))?);
        let held_left = profile.exists();
        let dry = clean(
            &roots,
            on,
            &arguments(&json!({ "all": true, "dry_run": true }))?,
        );
        let done = clean(
            &roots,
            on,
            &arguments(&json!({ "all": true, "confirm": true }))?,
        );
        assert_eq!(
            (
                status(&held),
                held_left,
                held.forgets_login(),
                status(&dry),
                dry.forgets_login(),
                done.forgets_login(),
                profile.exists(),
            ),
            (
                Some("held".to_owned()),
                true,
                false,
                Some("dry_run".to_owned()),
                false,
                true,
                false
            )
        );
        Ok(())
    }
}
