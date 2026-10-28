//! The connect step against fake client programs and in-memory lines.

#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

use std::{cell::RefCell, io};

use crate::setup::{
    SetupError,
    Talk,
    connect::{Program, Ran, connect},
};

const BINARY: &str = "/opt/holst/holst-mcp";

/// Client programs by command line; one not listed is missing from `PATH`.
struct Machine {
    lines: Vec<(&'static str, bool, &'static str)>,
    calls: RefCell<Vec<String>>,
}

impl Machine {
    fn with(lines: &[(&'static str, bool, &'static str)]) -> Self {
        Self {
            lines: lines.to_vec(),
            calls: RefCell::default(),
        }
    }
}

impl Program for Machine {
    fn run(&self, name: &str, args: &[&str]) -> io::Result<Ran> {
        let line = format!("{name} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        self.lines
            .iter()
            .find(|(known, ..)| *known == line)
            .map(|&(_, success, stderr)| Ran {
                success,
                stderr: stderr.to_owned(),
            })
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

/// Runs the step on `typed`; returns the terminal text and the command lines
/// run.
fn connected(
    machine: &Machine,
    typed: &str,
) -> Result<(String, Vec<String>), SetupError> {
    let mut talk = Talk::new(typed.as_bytes(), Vec::new(), false);
    connect(machine, &mut talk, BINARY)?;
    Ok((
        String::from_utf8_lossy(&talk.out).into_owned(),
        machine.calls.borrow().clone(),
    ))
}

const CLAUDE: &[(&str, bool, &str)] = &[
    ("claude --version", true, ""),
    ("claude mcp get holst-rs", false, "No MCP server found"),
    (
        "claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp",
        true,
        "",
    ),
];

const CODEX: &[(&str, bool, &str)] = &[
    ("codex --version", true, ""),
    ("codex mcp get holst-rs", false, ""),
    ("codex mcp add holst-rs -- /opt/holst/holst-mcp", true, ""),
];

#[test]
fn without_clients_only_the_snippets_are_printed() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(&[]), "")?;
    assert_eq!(calls, ["claude --version", "codex --version"]);
    insta::assert_snapshot!("snippets", shown);
    Ok(())
}

#[test]
fn enter_connects_claude_code_with_the_binary_path() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(CLAUDE), "\n")?;
    assert!(
        shown.starts_with("Connect Claude Code? [Y/n]: Connected Claude Code.\n"),
        "{shown}"
    );
    assert!(
        calls.contains(
            &"claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp".to_owned()
        ),
        "{calls:?}"
    );
    Ok(())
}

#[test]
fn yes_connects_codex() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(CODEX), "yes\n")?;
    assert!(
        shown.starts_with("Connect Codex? [Y/n]: Connected Codex.\n"),
        "{shown}"
    );
    assert_eq!(
        calls.last().map(String::as_str),
        Some("codex mcp add holst-rs -- /opt/holst/holst-mcp")
    );
    Ok(())
}

#[test]
fn no_runs_nothing_but_the_checks() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(CLAUDE), "n\n")?;
    assert!(!shown.contains("Connected"), "{shown}");
    assert_eq!(
        calls,
        [
            "claude --version",
            "claude mcp get holst-rs",
            "codex --version"
        ]
    );
    Ok(())
}

#[test]
fn an_existing_entry_is_kept_without_asking() -> Result<(), SetupError> {
    let machine = Machine::with(&[
        ("claude --version", true, ""),
        ("claude mcp get holst-rs", true, ""),
    ]);
    let (shown, calls) = connected(&machine, "")?;
    assert!(
        shown.starts_with("Claude Code has a holst-rs entry already; kept.\n"),
        "{shown}"
    );
    assert!(!shown.contains("Connect Claude Code?"), "{shown}");
    assert_eq!(
        calls,
        [
            "claude --version",
            "claude mcp get holst-rs",
            "codex --version"
        ]
    );
    Ok(())
}

#[test]
fn a_failing_command_shows_its_stderr_and_the_next_client_goes_on()
-> Result<(), SetupError> {
    let machine = Machine::with(&[
        ("claude --version", true, ""),
        ("claude mcp get holst-rs", false, ""),
        (
            "claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp",
            false,
            "boom\n",
        ),
        ("codex --version", true, ""),
        ("codex mcp get holst-rs", false, ""),
        ("codex mcp add holst-rs -- /opt/holst/holst-mcp", true, ""),
    ]);
    let (shown, _) = connected(&machine, "y\ny\n")?;
    assert!(
        shown.starts_with(
            "Connect Claude Code? [Y/n]: Could not connect Claude Code: boom\n\
             Connect Codex? [Y/n]: Connected Codex.\n"
        ),
        "{shown}"
    );
    Ok(())
}

#[test]
fn a_command_that_cannot_start_is_told_and_skipped() -> Result<(), SetupError> {
    let machine = Machine::with(&[
        ("claude --version", true, ""),
        ("claude mcp get holst-rs", false, ""),
    ]);
    let (shown, _) = connected(&machine, "\n")?;
    assert!(
        shown.starts_with(
            "Connect Claude Code? [Y/n]: Could not run claude: entity not found\n"
        ),
        "{shown}"
    );
    Ok(())
}
