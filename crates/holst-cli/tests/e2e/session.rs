//! Saved-login handling: rotation, logout and the JSON session commands.

use std::process::Output;

use serde_json::{Value, json};

#[cfg(unix)] use crate::support::{REVOKED, accepted, assert_no_noise};
use crate::{
    fake,
    support::{TestResult, holst_in, home, stderr},
};

/// Whether a 0500 directory refused a write; root writes into it anyway.
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum Denial {
    Enforced,
    Ignored,
}

#[cfg(unix)]
fn read_only_home(saved: &str) -> std::io::Result<(std::path::PathBuf, Denial)> {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = home(Some(saved))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))?;
    let probe = dir.join("probe");
    let writable = std::fs::write(&probe, "").is_ok();
    let denial = if writable {
        std::fs::remove_file(&probe)?;
        Denial::Ignored
    } else {
        Denial::Enforced
    };
    Ok((dir, denial))
}

#[cfg(unix)]
fn writable_again(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(unix)]
fn holst_read_only(
    base: &str,
    saved: &str,
    flags: &[&str],
) -> std::io::Result<(Output, std::path::PathBuf, Denial)> {
    let (dir, denial) = read_only_home(saved)?;
    let out = holst_in(base, &dir, flags, "");
    writable_again(&dir)?;
    out.map(|out| (out, dir, denial))
}

/// Checks the login a refused or allowed write leaves, and the warning it
/// brings.
#[cfg(unix)]
fn assert_rotation_saved_or_warned(
    denial: Denial,
    text: &str,
    saved: &str,
    before: &str,
) {
    match denial {
        Denial::Enforced => {
            assert!(
                text.starts_with(
                    "warning: the refreshed login was not saved: could not write "
                ),
                "{text}"
            );
            assert!(text.contains("\nhint: "), "{text}");
            assert_eq!(saved, before);
        }
        // Root can write: the refreshed login is saved without a word.
        Denial::Ignored => {
            assert!(!text.contains("warning:"), "{text}");
            assert_ne!(saved, before);
            assert!(saved.contains("rotated"), "{saved}");
        }
    }
}

#[cfg(unix)]
#[test]
fn unsaved_rotation_is_a_warning_not_a_failure() -> TestResult {
    let fake = holst_core::fake::Fake::start_blocking(fake::easy().rotating_cookie())?;
    let before = accepted();
    let args = ["-o", "json", "api", "GET", "/gapi/current_user"];
    let (out, dir, denial) = holst_read_only(fake.base_url(), &before, &args)?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Fake User"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_rotation_saved_or_warned(
        denial,
        &text,
        &std::fs::read_to_string(dir.join("session"))?,
        &before,
    );
    assert_no_noise(&text);
    Ok(())
}

/// Checks that a logout fails on a refused removal and succeeds on an allowed
/// one.
#[cfg(unix)]
fn assert_logout_failed_or_removed(
    denial: Denial,
    out: &Output,
    dir: &std::path::Path,
) {
    let text = stderr(out);
    let printed = String::from_utf8_lossy(&out.stdout);
    match denial {
        Denial::Enforced => {
            assert_eq!(out.status.code(), Some(74), "{text}");
            assert!(
                text.starts_with(
                    "error: could not remove the saved login from the file"
                ),
                "{text}"
            );
            assert!(!printed.contains("there was no saved login"), "{printed}");
            assert!(dir.join("session").exists());
        }
        // Root can remove it: the login is gone and the exit is clean.
        Denial::Ignored => {
            assert_eq!(out.status.code(), Some(0), "{text}");
            assert!(!dir.join("session").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn logout_that_cannot_remove_the_login_fails() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let (out, dir, denial) = holst_read_only(&base, REVOKED, &["auth", "logout"])?;
    assert_logout_failed_or_removed(denial, &out, &dir);
    Ok(())
}

#[test]
fn logout_with_an_unreadable_login_says_so() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let dir = home(Some(r#"{"cookies":"x"}"#))?;
    let out = holst_in(&base, &dir, &["auth", "logout"], "")?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.starts_with(
            "warning: the session on the server was not ended: the saved login could not be \
             read: the saved login is corrupted"
        ),
        "{text}"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "forgot the login in: file\n"
    );
    assert!(!dir.join("session").exists());
    Ok(())
}

#[test]
fn logout_names_what_is_left_in_this_home_and_not_in_the_real_one() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let dir = home(Some(r#"{"cookies":"x"}"#))?;
    let partial = dir.join(".session.1-2.tmp");
    std::fs::write(&partial, "half")?;
    let real = dir.with_extension("home").join(".config/holst/browser");
    std::fs::create_dir_all(&real)?;
    std::fs::write(real.join("cookies"), "old")?;
    let out = holst_in(&base, &dir, &["auth", "logout"], "")?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.contains("of old holst data is left in")
            && text.contains(&partial.display().to_string())
            && text.contains("holst storage clean"),
        "{text}"
    );
    assert!(!text.contains(&real.display().to_string()), "{text}");
    Ok(())
}

fn fields(out: &Output, a: &str, b: &str) -> Option<(Value, Value)> {
    serde_json::from_slice::<Value>(&out.stdout)
        .ok()
        .map(|value| {
            let field = |name: &str| value.get(name).cloned().unwrap_or_default();
            (field(a), field(b))
        })
}

#[test]
fn session_commands_print_json_when_asked() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let dir = home(None)?;
    let run = |args: &[&str], stdin: &str| holst_in(&base, &dir, args, stdin);
    let login = run(
        &["-o", "json", "auth", "login"],
        "mock@example.com\n123456\n",
    )?;
    assert_eq!(login.status.code(), Some(0), "{}", stderr(&login));
    assert_eq!(
        fields(&login, "name", "saved"),
        Some((json!("Fake User"), json!("file")))
    );
    let status = run(&["-o", "json", "auth", "status"], "")?;
    assert_eq!(status.status.code(), Some(0), "{}", stderr(&status));
    assert_eq!(
        fields(&status, "source", "status"),
        Some((json!("file"), json!("active")))
    );
    let counts = serde_json::from_slice::<Value>(&status.stdout)
        .ok()
        .and_then(|value| value.pointer("/workspaces/0/boards").cloned());
    assert!(
        counts.is_some_and(|boards| boards.is_number()),
        "no board count"
    );
    let logout = run(&["-o", "json", "auth", "logout"], "")?;
    assert_eq!(logout.status.code(), Some(0), "{}", stderr(&logout));
    assert_eq!(
        fields(&logout, "server", "forgotten"),
        Some((json!("ended"), json!(["file"])))
    );
    let none = run(&["-o", "json", "auth", "status"], "")?;
    assert_eq!(none.status.code(), Some(77), "{}", stderr(&none));
    assert_eq!(
        fields(&none, "logged_in", "file").map(|(logged_in, _)| logged_in),
        Some(json!(false))
    );
    let again = run(&["-o", "json", "auth", "logout"], "")?;
    assert_eq!(
        fields(&again, "server", "forgotten"),
        Some((json!("skipped"), json!([])))
    );
    Ok(())
}
