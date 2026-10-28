//! Piped login and the reports of a refused login.

use crate::{
    fake,
    support::{TestResult, assert_no_noise, holst, holst_in, home, stderr},
};

#[test]
fn login_through_a_pipe() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        None,
        &["auth", "login", "--no-save"],
        "mock@example.com\n123456\n",
    )?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("◇  Logged in ─"), "{text}");
    assert!(text.contains("│  Fake User "), "{text}");
    assert!(text.contains("│  not saved (--no-save) "), "{text}");
    assert!(!text.contains("Code sent"), "{text}");
    assert_eq!(text.matches("mock@example.com").count(), 1, "{text}");
    assert!(text.ends_with("└  Done\n\n"), "{text}");
    assert!(
        !text.contains(holst_fake::LOGIN_CODE),
        "the code was printed:\n{text}"
    );
    assert!(text.contains("◇  Code\n│  ▪▪▪▪▪▪"), "{text}");
    assert_no_noise(&text);
    Ok(())
}

#[test]
fn second_login_terminates_the_first_session_only() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let dir = home(None)?;
    let login = "mock@example.com\n123456\n";
    let first = holst_in(&base, &dir, &["auth", "login"], login)?;
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert!(
        !stderr(&first).contains("previous session"),
        "{}",
        stderr(&first)
    );
    let old = std::fs::read_to_string(dir.join("session"))?;
    let second = holst_in(&base, &dir, &["auth", "login"], login)?;
    let text = stderr(&second);
    assert_eq!(second.status.code(), Some(0), "{text}");
    assert!(text.contains("│  previous session terminated"), "{text}");
    let alive = holst_in(&base, &dir, &["auth", "status"], "")?;
    assert_eq!(alive.status.code(), Some(0), "{}", stderr(&alive));
    let gone = holst(&base, Some(&old), &["auth", "status"], "")?;
    assert_eq!(gone.status.code(), Some(77), "{}", stderr(&gone));
    Ok(())
}

#[test]
fn piped_login_without_a_trailing_newline_works() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        None,
        &["auth", "login", "--no-save"],
        "mock@example.com\r\n123456",
    )?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    Ok(())
}

#[test]
fn wrong_code_ends_a_piped_login_with_a_reason() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        None,
        &["auth", "login", "--no-save"],
        "mock@example.com\n000000\n",
    )?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(77), "{text}");
    assert!(text.contains("└  the login code did not match"), "{text}");
    assert!(!text.contains("HTTP 400"), "{text}");
    assert!(
        text.contains(
            "hint: run `holst auth login` again and type the code from the new email"
        ),
        "{text}"
    );
    assert_no_noise(&text);
    Ok(())
}

#[test]
fn empty_stdin_is_no_input() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(&base, None, &["auth", "login", "--no-save"], "")?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(66), "{text}");
    assert!(
        text.contains("no input: stdin ended before Email was given"),
        "{text}"
    );
    Ok(())
}

#[test]
fn bare_token_says_log_in() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(&base, Some("bad"), &["workspace", "ls"], "")?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(77), "{text}");
    assert!(text.starts_with("error: "), "{text}");
    assert!(text.contains("holst auth login"), "{text}");
    assert_no_noise(&text);
    Ok(())
}
