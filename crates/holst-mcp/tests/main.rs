//! The built `holst-mcp`; its test also makes cargo build it for scenarios.
#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests hand the real environment to the program under test"
)]

use std::process::Command;

use rmcp::{ServiceExt as _, transport::TokioChildProcess};

/// The names the `tools_list` snapshot of the server tests holds.
fn snapshot_names() -> Result<Vec<String>, serde_json::Error> {
    let snapshot = include_str!(
        "../src/server/snapshots/holst_mcp__server__tests__tools_list.snap"
    );
    let body = snapshot.splitn(3, "---").nth(2).unwrap_or_default();
    let tools: Vec<serde_json::Value> = serde_json::from_str(body)?;
    Ok(tools
        .iter()
        .filter_map(|tool| tool.get("name")?.as_str().map(str::to_owned))
        .collect())
}

/// What can go wrong between the test and the child it asks.
#[derive(Debug, thiserror::Error)]
enum WireError {
    #[error(transparent)]
    Spawn(#[from] std::io::Error),
    #[error(transparent)]
    Start(Box<rmcp::service::ClientInitializeError>),
    #[error(transparent)]
    Ask(#[from] rmcp::ServiceError),
    #[error(transparent)]
    Stop(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Snapshot(#[from] serde_json::Error),
}

/// The binary with an environment of its own: no login, no token, no cache.
fn isolated(name: &str) -> Command {
    let root =
        std::env::temp_dir().join(format!("holst-mcp-{name}-{}", std::process::id()));
    let mut command = Command::new(env!("CARGO_BIN_EXE_holst-mcp"));
    command
        .env_clear()
        .envs(std::env::vars_os().filter(|(var, _)| {
            ["PATH", "SystemRoot", "LLVM_PROFILE_FILE"]
                .iter()
                .any(|kept| var.eq_ignore_ascii_case(kept))
        }))
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("HOLST_HOME", root.join("holst"))
        .env("HOLST_CACHE_DIR", root.join("cache"));
    command
}

#[tokio::test]
async fn the_tools_are_listed_over_the_wire() -> Result<(), WireError> {
    let command = isolated("list");
    let client = ()
        .serve(TokioChildProcess::new(tokio::process::Command::from(
            command,
        ))?)
        .await
        .map_err(|error| WireError::Start(Box::new(error)))?;
    let listed = client.list_all_tools().await?;
    client.cancel().await?;
    let mut names: Vec<String> = listed
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let mut expected = snapshot_names()?;
    names.sort();
    expected.sort();
    assert_ne!(expected, Vec::<String>::new());
    assert_eq!(names, expected);
    Ok(())
}

#[test]
fn a_public_address_without_a_token_does_not_start() -> Result<(), WireError> {
    let output = isolated("public").args(["--http", "0.0.0.0:0"]).output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("0.0.0.0:0 is not a loopback address"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn print_config_prints_the_settings_with_the_token() -> Result<(), WireError> {
    let output = isolated("config")
        .env("HOLST_MCP_TOKEN", "test-token-test-token-test-token")
        .args(["--http", "127.0.0.1:8808", "--print-config"])
        .output()?;
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn print_config_creates_the_token_file_for_a_public_address() -> Result<(), WireError> {
    let output = isolated("created")
        .args(["--http", "0.0.0.0:8808", "--print-config"])
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Authorization: Bearer "));
    Ok(())
}
