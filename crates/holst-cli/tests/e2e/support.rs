//! Helpers shared by the end-to-end test modules.

use std::{
    io::Write as _,
    path::PathBuf,
    process::{Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
};

use crate::fake;

pub(crate) type TestResult = std::io::Result<()>;

pub(crate) const REVOKED: &str = r#"{"token":"csrf-old","cookies":"sessionToken=old"}"#;

/// A saved login the fake accepts.
pub(crate) fn accepted() -> String {
    format!(
        r#"{{"token":"{}","cookies":"{}"}}"#,
        holst_fake::ACCEPTED_TOKEN,
        holst_fake::ACCEPTED_COOKIES
    )
}

pub(crate) fn home(saved: Option<&str>) -> std::io::Result<PathBuf> {
    static RUN: AtomicU32 = AtomicU32::new(0);
    let n = RUN.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("holst-e2e-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    saved
        .map(|session| std::fs::write(dir.join("session"), session))
        .transpose()?;
    Ok(dir)
}

pub(crate) fn holst(
    base: &str,
    saved: Option<&str>,
    flags: &[&str],
    stdin: &str,
) -> std::io::Result<Output> {
    holst_in(base, &home(saved)?, flags, stdin)
}

pub(crate) fn holst_in(
    base: &str,
    home: &std::path::Path,
    flags: &[&str],
    stdin: &str,
) -> std::io::Result<Output> {
    fake::isolated(env!("CARGO_BIN_EXE_holst"), home)
        .args(flags)
        .env("HOLST_BASE_URL", base)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .map_or(Ok(()), |mut input| input.write_all(stdin.as_bytes()))?;
            child.wait_with_output()
        })
}

pub(crate) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(crate) fn assert_no_noise(text: &str) {
    let noise: Vec<&str> = [
        "Error:",
        "Caused by",
        "panicked",
        "Os {",
        "kind: ",
        "RUST_BACKTRACE",
    ]
    .into_iter()
    .filter(|noise| text.contains(noise))
    .collect();
    assert!(noise.is_empty(), "{noise:?} in:\n{text}");
}

pub(crate) fn holst_at(
    fake_home: &std::path::Path,
    dir: &std::path::Path,
    flags: &[&str],
) -> std::io::Result<Output> {
    fake::isolated(env!("CARGO_BIN_EXE_holst"), dir)
        .args(flags)
        .env("HOME", fake_home)
        .env("HOLST_BASE_URL", "http://127.0.0.1:9")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
}
