//! The connect step against fake client programs and in-memory lines.

#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

use std::{cell::RefCell, io};

use holst_form::{fake::Script, form::Form, line::Keystroke, terminal::TerminalKind};

use crate::setup::{
    SetupError,
    connect::{Clients, Program, Ran, connect},
};

const BINARY: &str = "/opt/holst/holst-mcp";

/// Client programs by command line; one not listed is missing from `PATH`.
pub(in crate::setup) struct Machine {
    lines: &'static [(&'static str, bool, &'static str)],
    pub(in crate::setup) calls: RefCell<Vec<String>>,
}

impl Machine {
    pub(in crate::setup) fn with(
        lines: &'static [(&'static str, bool, &'static str)],
    ) -> Self {
        Self {
            lines,
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

/// Runs the step on `script`; returns the terminal text and the command lines
/// run.
fn connected_on(
    machine: &Machine,
    script: Script,
    clients: &Clients,
) -> Result<(String, Vec<String>), SetupError> {
    let form = Form::bare(script);
    connect(machine, &form, BINARY, clients)?;
    Ok((form.terminal().written(), machine.calls.take()))
}

/// Asks which clients, answering the lines `typed`.
fn connected(
    machine: &Machine,
    typed: &[&'static str],
) -> Result<(String, Vec<String>), SetupError> {
    let script = Script::new(TerminalKind::Dumb, [], typed.iter().copied());
    connected_on(machine, script, &Clients::Ask)
}

const CLAUDE: &[(&str, bool, &str)] = &[
    ("claude mcp get holst-rs", false, "No MCP server found"),
    (
        "claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp",
        true,
        "",
    ),
];

const BOTH: &[(&str, bool, &str)] = &[
    ("claude mcp get holst-rs", false, ""),
    (
        "claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp",
        true,
        "",
    ),
    ("codex mcp get holst-rs", false, ""),
    ("codex mcp add holst-rs -- /opt/holst/holst-mcp", true, ""),
];

#[test]
fn without_clients_only_the_snippets_are_printed() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(&[]), &[])?;
    assert_eq!(calls, ["claude mcp get holst-rs", "codex mcp get holst-rs"]);
    insta::assert_snapshot!("snippets", shown);
    Ok(())
}

#[test]
fn enter_connects_claude_code_with_the_binary_path() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(CLAUDE), &[""])?;
    assert!(shown.contains("Connect which clients?"), "{shown}");
    assert!(shown.contains("Connected Claude Code."), "{shown}");
    assert!(
        calls.contains(
            &"claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp".to_owned()
        ),
        "{calls:?}"
    );
    Ok(())
}

#[test]
fn the_clients_without_an_entry_are_one_list_with_all_picked() -> Result<(), SetupError>
{
    let (shown, calls) = connected(&Machine::with(BOTH), &[""])?;
    assert!(shown.contains("1) [x] Claude Code"), "{shown}");
    assert!(shown.contains("2) [x] Codex"), "{shown}");
    assert!(shown.contains("Connected Claude Code."), "{shown}");
    assert!(shown.contains("Connected Codex."), "{shown}");
    assert_eq!(calls.len(), 4, "{calls:?}");
    Ok(())
}

#[test]
fn a_number_picks_only_that_client() -> Result<(), SetupError> {
    let (shown, calls) = connected(&Machine::with(BOTH), &["2"])?;
    assert!(shown.contains("Connected Codex."), "{shown}");
    assert!(!shown.contains("Connected Claude Code."), "{shown}");
    assert_eq!(
        calls.last().map(String::as_str),
        Some("codex mcp add holst-rs -- /opt/holst/holst-mcp")
    );
    Ok(())
}

#[test]
fn picking_none_runs_nothing_but_the_checks() -> Result<(), SetupError> {
    let keys = [Keystroke::Char('i'), Keystroke::Enter];
    let script = Script::new(TerminalKind::Live, keys, []);
    let (shown, calls) = connected_on(&Machine::with(CLAUDE), script, &Clients::Ask)?;
    assert!(!shown.contains("Connected"), "{shown}");
    assert_eq!(calls, ["claude mcp get holst-rs", "codex mcp get holst-rs"]);
    Ok(())
}

#[test]
fn named_clients_are_connected_without_asking() -> Result<(), SetupError> {
    let script = Script::new(TerminalKind::Dumb, [], []);
    let clients = Clients::parse("codex")?;
    let (shown, calls) = connected_on(&Machine::with(BOTH), script, &clients)?;
    assert!(!shown.contains("Connect which clients?"), "{shown}");
    assert!(shown.contains("Connected Codex."), "{shown}");
    assert_eq!(
        calls,
        [
            "codex mcp get holst-rs",
            "codex mcp add holst-rs -- /opt/holst/holst-mcp"
        ]
    );
    Ok(())
}

#[test]
fn a_named_client_that_cannot_start_is_told() -> Result<(), SetupError> {
    let script = Script::new(TerminalKind::Dumb, [], []);
    let clients = Clients::parse("claude")?;
    let (shown, _) = connected_on(&Machine::with(&[]), script, &clients)?;
    assert!(shown.contains("Could not run claude"), "{shown}");
    Ok(())
}

#[test]
fn an_unknown_client_name_lists_the_known_ones() {
    let shown = Clients::parse("claude,gemini")
        .err()
        .map(|err| err.to_string());
    assert_eq!(
        shown.as_deref(),
        Some("unknown client gemini: --clients takes claude or codex, comma-separated")
    );
}

#[test]
fn an_existing_entry_is_kept_without_asking() -> Result<(), SetupError> {
    let machine = Machine::with(&[("claude mcp get holst-rs", true, "")]);
    let (shown, calls) = connected(&machine, &[])?;
    assert!(
        shown.contains("Claude Code has a holst-rs entry already; kept."),
        "{shown}"
    );
    assert!(!shown.contains("Connect Claude Code?"), "{shown}");
    assert_eq!(calls, ["claude mcp get holst-rs", "codex mcp get holst-rs"]);
    Ok(())
}

#[test]
fn a_failing_command_shows_its_stderr_and_the_next_client_goes_on()
-> Result<(), SetupError> {
    let machine = Machine::with(&[
        ("claude mcp get holst-rs", false, ""),
        (
            "claude mcp add --scope user holst-rs -- /opt/holst/holst-mcp",
            false,
            "boom\n",
        ),
        ("codex mcp get holst-rs", false, ""),
        ("codex mcp add holst-rs -- /opt/holst/holst-mcp", true, ""),
    ]);
    let (shown, _) = connected(&machine, &[""])?;
    assert!(shown.contains("Could not connect Claude Code"), "{shown}");
    assert!(shown.contains("boom"), "{shown}");
    assert!(shown.contains("Connected Codex."), "{shown}");
    Ok(())
}

#[test]
fn a_command_that_cannot_start_is_told_and_skipped() -> Result<(), SetupError> {
    let machine = Machine::with(&[("claude mcp get holst-rs", false, "")]);
    let (shown, _) = connected(&machine, &[""])?;
    assert!(shown.contains("Could not run claude"), "{shown}");
    assert!(shown.contains("entity not found"), "{shown}");
    Ok(())
}
