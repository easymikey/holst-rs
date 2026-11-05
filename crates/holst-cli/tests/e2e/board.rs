//! Board deletion and trash, option checks, and cache-variable warnings.

use std::{path::PathBuf, process::Stdio};

use serde_json::Value;

use crate::{
    fake,
    support::{REVOKED, TestResult, holst_at, holst_in, home, saved_for, stderr},
};

fn signed_home(base: &str) -> std::io::Result<PathBuf> {
    let dir = home(None)?;
    let login = holst_in(base, &dir, &["auth", "login"], "mock@example.com\n123456\n")?;
    assert_eq!(login.status.code(), Some(0), "{}", stderr(&login));
    Ok(dir)
}

#[test]
fn permanent_delete_needs_the_whole_name() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    let delete = |name: &str| {
        holst_in(
            base,
            &dir,
            &["board", "rm", name, "--permanently", "--yes"],
            "",
        )
    };
    let part = delete("Pla")?;
    let text = stderr(&part);
    assert_eq!(part.status.code(), Some(66), "{text}");
    assert!(text.starts_with("error: board \"Pla\" not found"), "{text}");
    assert!(fake.id_of("Plans").is_some(), "a part deleted the board");
    let whole = delete("plans")?;
    assert_eq!(whole.status.code(), Some(0), "{}", stderr(&whole));
    assert!(
        fake.id_of("Plans").is_none(),
        "the whole name did not delete"
    );
    assert!(
        fake.trash_of("easy")
            .iter()
            .all(|entry| entry.name != "Plans")
    );
    Ok(())
}

#[test]
fn a_board_is_named_by_its_handle_from_the_last_listing() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    let listing = holst_in(base, &dir, &["board", "ls"], "")?;
    let shown = String::from_utf8_lossy(&listing.stdout).into_owned();
    let second = shown.lines().nth(1).unwrap_or_default().trim_start();
    let name = second.split("  ").next().unwrap_or_default();
    assert!(second.contains("  @2  "), "{shown}");
    let show = holst_in(base, &dir, &["board", "show", "@2"], "")?;
    let text = String::from_utf8_lossy(&show.stdout).into_owned();
    assert_eq!(show.status.code(), Some(0), "{}", stderr(&show));
    assert!(text.contains(name), "{name:?} not in:\n{text}");
    Ok(())
}

#[test]
fn an_object_is_named_by_its_handle_from_the_last_listing() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    for text in ["first", "second"] {
        let add =
            holst_in(base, &dir, &["object", "add", "sticker", "Plans", text], "")?;
        assert_eq!(add.status.code(), Some(0), "{}", stderr(&add));
    }
    let listing = holst_in(base, &dir, &["object", "ls", "Plans"], "")?;
    let shown = String::from_utf8_lossy(&listing.stdout).into_owned();
    let row = |at: usize| shown.lines().filter(|line| line.starts_with('@')).nth(at);
    let second = row(1).unwrap_or_default();
    assert!(second.starts_with("@2 "), "{shown}");
    let edit = holst_in(
        base,
        &dir,
        &[
            "object", "edit", "Plans", "--object", "@2", "--text", "changed",
        ],
        "",
    )?;
    assert_eq!(edit.status.code(), Some(0), "{}", stderr(&edit));
    let after = holst_in(base, &dir, &["object", "ls", "Plans"], "")?;
    let text = String::from_utf8_lossy(&after.stdout).into_owned();
    let changed: Vec<_> = text
        .lines()
        .filter(|line| line.contains("changed"))
        .collect();
    assert_eq!(changed.len(), 1, "{text}");
    assert!(
        text.lines().any(|line| line.contains("first")),
        "the first object changed:\n{text}"
    );
    let board = holst_in(base, &dir, &["board", "ls"], "")?;
    assert_eq!(board.status.code(), Some(0), "{}", stderr(&board));
    let wrong = holst_in(
        base,
        &dir,
        &["object", "edit", "Plans", "--object", "@2", "--text", "x"],
        "",
    )?;
    assert_eq!(wrong.status.code(), Some(64), "{}", stderr(&wrong));
    assert!(
        stderr(&wrong).contains("@2 names a board, not an object"),
        "{}",
        stderr(&wrong)
    );
    Ok(())
}

#[test]
fn an_object_handle_used_with_another_board_is_refused() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    let add = holst_in(
        base,
        &dir,
        &["object", "add", "sticker", "Plans", "one"],
        "",
    )?;
    assert_eq!(add.status.code(), Some(0), "{}", stderr(&add));
    let listing = holst_in(base, &dir, &["object", "ls", "Plans"], "")?;
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    let wrong = holst_in(
        base,
        &dir,
        &["object", "get", "Old roadmap", "--object", "@1"],
        "",
    )?;
    assert_eq!(wrong.status.code(), Some(64), "{}", stderr(&wrong));
    let message = stderr(&wrong);
    assert!(
        message
            .contains(r#"@1 is from the listing of board "Plans", not "Old roadmap""#),
        "{message}"
    );
    assert!(
        message.contains("run `holst o ls Old roadmap` first"),
        "{message}"
    );
    Ok(())
}
#[test]
fn tree_with_trash_shows_the_trash() -> TestResult {
    let fake = fake::start()?;
    let base = fake.base_url();
    let dir = signed_home(base)?;
    let trashed = holst_in(base, &dir, &["board", "rm", "Old roadmap"], "")?;
    assert_eq!(trashed.status.code(), Some(0), "{}", stderr(&trashed));
    let plain = holst_in(base, &dir, &["board", "ls"], "")?;
    let shown = String::from_utf8_lossy(&plain.stdout).into_owned();
    assert!(!shown.contains("Old roadmap"), "{shown}");
    let with = holst_in(base, &dir, &["board", "ls", "--view", "both"], "")?;
    let trash_shown = String::from_utf8_lossy(&with.stdout).into_owned();
    assert_eq!(with.status.code(), Some(0), "{}", stderr(&with));
    let id = fake.id_of("Old roadmap").unwrap_or_default();
    assert!(
        trash_shown
            .lines()
            .any(|line| line.starts_with("Old roadmap  ")
                && line.contains(&id)
                && line.ends_with("  [in trash]")),
        "{trash_shown}"
    );
    let json = holst_in(
        base,
        &dir,
        &["-o", "json", "board", "ls", "--view", "both"],
        "",
    )?;
    let mut names: Vec<String> = serde_json::from_slice::<Value>(&json.stdout)
        .ok()
        .and_then(|reply| reply.get("nodes").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|channel| {
            channel
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    assert_eq!(names, ["Old roadmap", "Plans", "Q4"]);
    Ok(())
}

#[test]
fn snapshot_options_out_of_range_are_listed_together() -> TestResult {
    // A saved login that is never sent: the address has no server.
    let root = home(Some(&saved_for("http://127.0.0.1:9", REVOKED)))?;
    let run = |options: &[&str]| {
        let board = ["board", "snap", "00000001-0000-4000-8000-000000000001"];
        holst_at(&root, &root, &[&board[..], options].concat())
    };
    let scale = "scale 0.0 is out of range: expected above 0, at most 100 pixels per \
                 board unit";
    let side = "max side 5.0 is out of range: expected 16 to 16384 pixels";
    let one = run(&["--scale", "0"])?;
    assert_eq!(
        (one.status.code(), stderr(&one)),
        (Some(64), format!("error: {scale}\n"))
    );
    let two = run(&["--scale", "0", "--max-side", "5"])?;
    assert_eq!(
        (two.status.code(), stderr(&two)),
        (
            Some(64),
            format!("error: 2 problems:\n  - {scale}\n  - {side}\n")
        )
    );
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

#[rstest::rstest]
#[case::size("HOLST_CACHE_MAX", "lots")]
#[case::trust("HOLST_CACHE_TRUST", "a while")]
#[case::relative_dir("HOLST_CACHE_DIR", "cache")]
fn a_bad_cache_variable_is_a_warning(
    #[case] name: &str,
    #[case] value: &str,
) -> TestResult {
    let root = home(None)?;
    let fake_home = root.join("home");
    let dir = root.join("hh");
    std::fs::create_dir_all(&dir)?;
    let run = |args: &[&str]| {
        fake::isolated(env!("CARGO_BIN_EXE_holst"), &dir)
            .args(args)
            .env("HOME", &fake_home)
            .env("USERPROFILE", &fake_home)
            .env("HOLST_BASE_URL", "http://127.0.0.1:9")
            .env("NO_COLOR", "1")
            .env(name, value)
            .stdin(Stdio::null())
            .output()
    };
    // `status` has no login to report on, so it fails after the warning.
    let commands: [(&[&str], i32); 4] = [
        (&["auth", "status"], 77),
        (&["auth", "logout", "--local-only"], 0),
        (&["storage", "ls"], 0),
        (&["storage", "clean", "--dry-run"], 0),
    ];
    commands.iter().try_for_each(|(args, code)| {
        let out = run(args)?;
        let text = stderr(&out);
        assert_eq!(out.status.code(), Some(*code), "{args:?}: {text}");
        let warned = text
            .lines()
            .filter(|line| line.starts_with("warning: ") && line.contains(name))
            .count();
        assert_eq!(warned, 1, "{args:?}: {text}");
        Ok::<_, std::io::Error>(())
    })?;
    std::fs::remove_dir_all(&root)?;
    Ok(())
}
