//! The CLI keeps the workspace trees of an account between runs.
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use holst_core::fake::Fake;

use crate::{
    fake,
    support::{TestResult, home, stderr},
};

const LOGIN: &str = "mock@example.com\n123456\n";

fn run(
    base: &str,
    dir: &Path,
    flags: &[&str],
    env: &[(&str, &str)],
    stdin: &str,
) -> std::io::Result<Output> {
    let mut child = fake::isolated(env!("CARGO_BIN_EXE_holst"), dir)
        .args(flags)
        .envs(env.iter().copied())
        .env("HOLST_BASE_URL", base)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .map_or(Ok(()), |mut input| input.write_all(stdin.as_bytes()))?;
    child.wait_with_output()
}

fn ok(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", stderr(output));
}

fn signed_home(base: &str) -> std::io::Result<PathBuf> {
    let dir = home(None)?;
    ok(&run(base, &dir, &["auth", "login"], &[], LOGIN)?);
    Ok(dir)
}

/// What the fake was asked by `work`, as `(trees, current_user)` counts.
fn asked(fake: &Fake, work: impl FnOnce() -> std::io::Result<()>) -> (usize, usize) {
    let before = fake.requests().len();
    drop(work());
    let log = fake.requests().split_off(before);
    let count = |end: &str| log.iter().filter(|line| line.ends_with(end)).count();
    (count("/tree"), count("/current_user"))
}

fn show(base: &str, dir: &Path, env: &[(&str, &str)]) -> std::io::Result<()> {
    ok(&run(base, dir, &["board", "show", "Plans"], env, "")?);
    Ok(())
}

fn tree_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .flat_map(|path| {
            let inside = tree_files(&path);
            std::iter::once(path).chain(inside).collect::<Vec<_>>()
        })
        .filter(|path| path.parent().is_some_and(|up| up.ends_with("trees")))
        .collect()
}

#[test]
fn the_second_show_by_name_asks_no_tree_and_no_account() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    show(base, &dir, &[])?;
    assert_eq!(asked(&fake, || show(base, &dir, &[])), (0, 0));
    Ok(())
}

#[test]
fn with_the_cache_off_the_second_show_asks_the_trees() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    let off = [("HOLST_CACHE", "off")];
    show(base, &dir, &off)?;
    assert_eq!(asked(&fake, || show(base, &dir, &off)).0, 1);
    Ok(())
}

#[test]
fn a_write_asks_a_tree_older_than_an_hour_and_a_read_does_not() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    show(base, &dir, &[])?;
    let cache = dir.with_extension("cache");
    tree_files(&cache).iter().try_for_each(|file| {
        let text = std::fs::read_to_string(file)?;
        let body = text.split_once('\n').map_or("", |(_, body)| body);
        std::fs::write(
            file,
            format!(
                "1
{body}"
            ),
        )
    })?;
    assert_eq!(asked(&fake, || show(base, &dir, &[])).0, 0);
    let write = || {
        ok(&run(
            base,
            &dir,
            &["object", "add", "sticker", "Plans", "hi"],
            &[],
            "",
        )?);
        Ok(())
    };
    assert_eq!(asked(&fake, write).0, 1);
    Ok(())
}

#[test]
fn a_board_refused_as_deleted_drops_the_tree_holding_it() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    show(base, &dir, &[])?;
    let id = fake.id_of("Plans").unwrap_or_default();
    fake.fail(&format!("channel/{id}"), 404);
    let gone = run(base, &dir, &["board", "show", &id], &[], "")?;
    assert_ne!(gone.status.code(), Some(0));
    let again = || run(base, &dir, &["board", "show", "Plans"], &[], "").map(drop);
    assert_eq!(asked(&fake, again).0, 1);
    Ok(())
}
