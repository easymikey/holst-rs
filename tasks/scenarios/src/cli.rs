//! The `holst` binary as a user runs it with `-o json`.

use std::{
    ffi::OsStr,
    io::Write as _,
    process::{Command, Output, Stdio},
};

use crate::{
    error::ScenarioError,
    observation::{ErrorExit, Observation},
};

/// Runs `holst -o json` with `arguments`; a success is read by `observe`, a
/// failure is read here, the same for every command.
pub(crate) async fn run(
    command: Command,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
    observe: impl FnOnce(&[u8]) -> Result<Observation, ScenarioError>,
) -> Result<Observation, ScenarioError> {
    let output = output(command, arguments, "").await?;
    failure(&output).map_or_else(|| observe(&output.stdout), Ok)
}

/// Runs `holst -o json` with `arguments`, `input` on its stdin, and waits for
/// all it printed.
pub(crate) async fn output(
    mut command: Command,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
    input: &str,
) -> Result<Output, ScenarioError> {
    command
        .args(["-o", "json"])
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    child
        .stdin
        .take()
        .map_or(Ok(()), |mut stdin| stdin.write_all(input.as_bytes()))?;
    Ok(tokio::task::spawn_blocking(move || child.wait_with_output()).await??)
}

pub(crate) fn failure(output: &Output) -> Option<Observation> {
    Some(output.status.code())
        .filter(|&code| code != Some(0))
        .map(|code| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Observation::Failed {
                message: paragraph(&stderr, "error: "),
                hint: labelled(&stderr, "hint: "),
                signal: code.map_or(ErrorExit::Killed, ErrorExit::ExitCode),
            }
        })
}

/// Returns the text after `label` on its first line and the indented lines
/// right after it (clap lists the possible values there).
fn paragraph(text: &str, label: &str) -> String {
    let mut lines = text.lines().skip_while(|line| !line.starts_with(label));
    lines
        .next()
        .and_then(|first| first.strip_prefix(label))
        .into_iter()
        .chain(
            lines
                .take_while(|line| {
                    line.starts_with(char::is_whitespace) && !line.trim().is_empty()
                })
                .map(str::trim),
        )
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns the text after `label` on the first line that starts with it.
pub(crate) fn labelled(text: &str, label: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_default()
        .to_owned()
}
