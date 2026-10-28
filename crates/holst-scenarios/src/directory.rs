//! The temporary home, login directory and cache of one run.

use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};

const PARTS: [&str; 4] = ["home", "holst", "cache", "snapshots"];

#[derive(Debug)]
pub(crate) struct TestDirectory {
    root: PathBuf,
}

impl TestDirectory {
    pub(crate) fn create() -> io::Result<Self> {
        static RUN: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "holst-scenarios-{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
        PARTS
            .iter()
            .try_for_each(|part| std::fs::create_dir_all(root.join(part)))?;
        Ok(Self { root })
    }

    pub(crate) fn join(&self, part: &str) -> PathBuf {
        self.root.join(part)
    }
}

// GUARD: removes the temporary directory of the run
impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap_or_default();
    }
}
