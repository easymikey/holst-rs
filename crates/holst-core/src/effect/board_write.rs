use serde::Deserialize;

/// Whether the load downloaded a board link or the seed already covered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSource {
    Skipped,
    Downloaded { bytes: u64 },
}

impl LoadSource {
    pub(crate) const fn plus(self, bytes: u64) -> Self {
        match (self, bytes) {
            (LoadSource::Skipped, 0) => LoadSource::Skipped,
            (LoadSource::Skipped, bytes) => LoadSource::Downloaded { bytes },
            (LoadSource::Downloaded { bytes: before }, more) => {
                LoadSource::Downloaded {
                    bytes: before.saturating_add(more),
                }
            }
        }
    }
}

/// Whether a change is only shown or also sent: it picks the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "bool")]
pub enum WriteMode {
    DryRun,
    Send,
}

impl From<bool> for WriteMode {
    fn from(dry_run: bool) -> Self {
        if dry_run { Self::DryRun } else { Self::Send }
    }
}
