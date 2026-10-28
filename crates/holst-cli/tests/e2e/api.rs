//! Argument mistakes, dry runs and raw API replies.

use crate::{
    fake,
    support::{self, REVOKED, TestResult, accepted, assert_no_noise, holst, stderr},
};

#[test]
fn a_bad_flag_exits_with_the_usage_code_and_help_with_zero() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let bad = holst(&base, None, &["auth", "status", "--no-such-flag"], "")?;
    assert_eq!(bad.status.code(), Some(64), "{}", stderr(&bad));
    let help = holst(&base, None, &["--help"], "")?;
    assert_eq!(help.status.code(), Some(0), "{}", stderr(&help));
    assert!(String::from_utf8_lossy(&help.stdout).contains("holst help exit-codes"));
    Ok(())
}

#[test]
fn the_exit_codes_command_prints_the_table() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let shown = holst(&base, None, &["exit-codes"], "")?;
    assert_eq!(shown.status.code(), Some(0), "{}", stderr(&shown));
    let out = String::from_utf8_lossy(&shown.stdout);
    assert!(out.contains("EXIT CODES:"), "{out}");
    Ok(())
}

#[test]
fn mistakes_in_api_arguments_are_explained() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let cases: [(&[&str], i32, &[&str]); 5] = [
        (
            &["api", "GET", "gapi/current_user"],
            64,
            &["error: ", "must start with \"/\""],
        ),
        (
            &["api", "FETCH", "/gapi/current_user"],
            64,
            &["FETCH", "[possible values: GET, POST, PUT, PATCH, DELETE]"],
        ),
        (
            &["api", "GET", "/gapi/x", "-q", "novalue"],
            64,
            &["expected key=value, got \"novalue\""],
        ),
        (
            &[
                "api",
                "POST",
                "/gapi/x",
                "--body",
                "@/nonexistent/holst-body.json",
            ],
            66,
            &[
                "error: could not read /nonexistent/holst-body.json",
                "\n  cause: ",
            ],
        ),
        // Held before any request: the revoked login is never tried.
        (
            &["api", "DELETE", "/gapi/x"],
            77,
            &[
                "error: sending DELETE /gapi/x cannot be undone; confirm with --yes",
                "hint: check the target, then run the command again with `--yes`",
            ],
        ),
    ];
    cases.iter().try_for_each(|(args, code, wanted)| {
        let out = holst(&base, Some(REVOKED), args, "")?;
        let text = stderr(&out);
        assert_eq!(out.status.code(), Some(*code), "{args:?}: {text}");
        assert!(text.starts_with("error: "), "{args:?}: {text}");
        let missing: Vec<&&str> = wanted
            .iter()
            .filter(|want| !text.contains(**want))
            .collect();
        assert!(missing.is_empty(), "{args:?}: no {missing:?} in\n{text}");
        assert_no_noise(&text);
        Ok::<_, std::io::Error>(())
    })?;
    Ok(())
}

#[test]
fn a_dry_run_shows_the_request_and_sends_nothing() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let board = "00000000-0000-4000-8000-0000000000b1";
    let path = format!("/gapi/channel/{board}/delete");
    let body = r#"{"permanently":true}"#;
    let cases: [&[&str]; 1] = [&["api", "POST", &path, "--body", body, "--dry-run"]];
    cases.iter().try_for_each(|args| {
        let out = holst(&base, Some(REVOKED), args, "")?;
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!(
                "dry run, nothing sent:\nPOST /gapi/channel/{board}/delete\n{{\"permanently\":true}}\n"
            ),
            "{args:?}"
        );
        Ok::<_, std::io::Error>(())
    })?;
    Ok(())
}

#[test]
fn empty_reply_to_a_change_says_done() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        Some(REVOKED),
        &[
            "api",
            "POST",
            "/gapi/send_temporary_password",
            "--yes",
            "--body",
            "{}",
        ],
        "",
    )?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "done\n");
    Ok(())
}

#[test]
fn a_get_prints_what_the_server_answered() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        Some(&accepted()),
        &["api", "GET", "/gapi/current_user"],
        "",
    )?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    assert!(reply.is_object(), "{reply}");
    Ok(())
}

#[test]
fn a_post_with_the_short_yes_is_sent() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        Some(REVOKED),
        &[
            "api",
            "POST",
            "/gapi/send_temporary_password",
            "-y",
            "--body",
            "{}",
        ],
        "",
    )?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "done\n");
    Ok(())
}

#[test]
fn a_post_without_yes_shows_the_plan_and_asks_for_it() -> TestResult {
    let base = fake::start()?.base_url().to_owned();
    let out = holst(
        &base,
        Some(REVOKED),
        &[
            "api",
            "POST",
            "/gapi/send_temporary_password",
            "--body",
            "{}",
        ],
        "",
    )?;
    let text = stderr(&out);
    assert_eq!(out.status.code(), Some(77), "{text}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "POST /gapi/send_temporary_password\n{}\n"
    );
    assert!(text.contains("cannot be undone"), "{text}");
    assert!(text.contains("--yes"), "{text}");
    Ok(())
}

/// The exit code, stdout and stderr of `holst args`, with `HOLST_HOME` set or
/// not.
fn shown(
    args: &[&str],
    keep_home: bool,
) -> std::io::Result<(Option<i32>, Vec<u8>, Vec<u8>)> {
    let home = support::home(None)?;
    let mut command = fake::isolated(env!("CARGO_BIN_EXE_holst"), &home);
    if !keep_home {
        command.env_remove("HOLST_HOME");
    }
    let out = command.args(args).env("NO_COLOR", "1").output()?;
    Ok((out.status.code(), out.stdout, out.stderr))
}

#[test]
fn a_group_without_a_subcommand_shows_its_help_with_holst_home_set() -> TestResult {
    [&[][..], &["b"][..]].into_iter().try_for_each(|args| {
        let with = shown(args, true)?;
        let without = shown(args, false)?;
        assert_eq!(with, without, "holst {args:?}");
        assert_eq!(with.0, Some(0), "{}", String::from_utf8_lossy(&with.2));
        let text = String::from_utf8_lossy(&with.1);
        assert!(text.contains("USAGE"), "{text}");
        assert!(!text.contains("exit-codes,"), "{text}");
        Ok(())
    })
}
