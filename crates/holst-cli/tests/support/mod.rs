//! The test environment of `holst` and a fake server.

use std::{
    ffi::{OsStr, OsString},
    path::Path,
    process::Command,
};

use holst_fake::{Fake, Fixture};

/// The only variables a test command takes from the developer's environment.
const KEPT_VARIABLES: [&str; 3] = ["PATH", "SystemRoot", "LLVM_PROFILE_FILE"];

/// A command that sees a temporary `HOME`, `HOLST_HOME` and `HOLST_CACHE_DIR`
/// and none of the developer's `HOLST_*` variables.
pub(crate) fn isolated(program: impl AsRef<OsStr>, holst_home: &Path) -> Command {
    isolated_from(program, holst_home, std::env::vars_os())
}

/// Like [`isolated`], with `parent` standing for the environment of the test
/// process.
pub(crate) fn isolated_from(
    program: impl AsRef<OsStr>,
    holst_home: &Path,
    parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Command {
    // imperative: `Command` is a `&mut` builder.
    let mut command = Command::new(program);
    command
        .env_clear()
        .envs(parent.into_iter().filter(|(name, _)| {
            KEPT_VARIABLES
                .iter()
                .any(|kept| name.eq_ignore_ascii_case(kept))
        }))
        .env("HOME", holst_home.with_extension("home"))
        .env("HOLST_HOME", holst_home)
        .env("HOLST_CACHE_DIR", holst_home.with_extension("cache"));
    command
}

/// What most tests start from: the workspace `easy` with the folder `Q4` and,
/// in it, the boards `Plans` and `Old roadmap`.
pub(crate) fn easy() -> Fixture {
    Fixture::default()
        .workspace("easy")
        .folder("Q4", "easy")
        .board("Plans", "easy", Some("Q4"))
        .board("Old roadmap", "easy", Some("Q4"))
}

/// Starts a fake server on a thread of its own; it ends with the test process.
pub(crate) fn start() -> std::io::Result<Fake> {
    Fake::start_blocking(easy())
}
