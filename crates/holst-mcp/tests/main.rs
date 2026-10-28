//! The built `holst-mcp`; its test also makes cargo build it for scenarios.
#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

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

#[tokio::test]
async fn the_tools_are_listed_over_the_wire() -> Result<(), WireError> {
    let root =
        std::env::temp_dir().join(format!("holst-mcp-list-{}", std::process::id()));
    let mut command = Command::new(env!("CARGO_BIN_EXE_holst-mcp"));
    command
        .env_clear()
        .envs(std::env::vars_os().filter(|(name, _)| {
            ["PATH", "SystemRoot", "LLVM_PROFILE_FILE"]
                .iter()
                .any(|kept| name.eq_ignore_ascii_case(kept))
        }))
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("HOLST_HOME", root.join("holst"))
        .env("HOLST_CACHE_DIR", root.join("cache"));
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
    assert!(!expected.is_empty());
    assert_eq!(names, expected);
    Ok(())
}
