//! The connect step of `holst-mcp setup`: the MCP clients get the server.

use std::{
    io::{self, BufRead, Write},
    process::{Command, Stdio},
};

use serde_json::json;

use super::{SetupError, Talk};

/// The name the server is registered under in every client.
const SERVER: &str = "holst-rs";

/// What a client command did: whether it succeeded, and what it said on stderr.
pub(crate) struct Ran {
    pub(crate) success: bool,
    pub(crate) stderr: String,
}

/// Runs the command line of a client; an `Err` means it could not be started.
pub(crate) trait Program {
    fn run(&self, name: &str, args: &[&str]) -> io::Result<Ran>;
}

/// The programs of the machine, found on `PATH`.
pub(crate) struct System;

impl Program for System {
    fn run(&self, name: &str, args: &[&str]) -> io::Result<Ran> {
        Command::new(name)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map(|output| Ran {
                success: output.status.success(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
    }
}

/// The clients that have a command to register a server.
#[derive(Debug, Clone, Copy)]
enum Cli {
    Claude,
    Codex,
}

impl Cli {
    const fn label(self) -> &'static str {
        match self {
            Cli::Claude => "Claude Code",
            Cli::Codex => "Codex",
        }
    }

    const fn program(self) -> &'static str {
        match self {
            Cli::Claude => "claude",
            Cli::Codex => "codex",
        }
    }

    /// The arguments of the client's `mcp add` for the server.
    fn add(self, binary: &str) -> Vec<&str> {
        let scope: &[&str] = match self {
            Cli::Claude => &["--scope", "user"],
            Cli::Codex => &[],
        };
        ["mcp", "add"]
            .into_iter()
            .chain(scope.iter().copied())
            .chain([SERVER, "--", binary])
            .collect()
    }
}

/// Offers each client found on `PATH`, then prints what to put in the others.
pub(super) fn connect<R: BufRead, W: Write>(
    programs: &impl Program,
    talk: &mut Talk<R, W>,
    binary: &str,
) -> Result<(), SetupError> {
    [Cli::Claude, Cli::Codex]
        .into_iter()
        .try_for_each(|cli| offer(programs, talk, binary, cli))?;
    talk.say(&snippets(binary))
}

fn offer<R: BufRead, W: Write>(
    programs: &impl Program,
    talk: &mut Talk<R, W>,
    binary: &str,
    cli: Cli,
) -> Result<(), SetupError> {
    if programs.run(cli.program(), &["--version"]).is_ok() {
        offer_found(programs, talk, binary, cli)
    } else {
        Ok(())
    }
}

fn offer_found<R: BufRead, W: Write>(
    programs: &impl Program,
    talk: &mut Talk<R, W>,
    binary: &str,
    cli: Cli,
) -> Result<(), SetupError> {
    let registered = programs
        .run(cli.program(), &["mcp", "get", SERVER])
        .is_ok_and(|ran| ran.success);
    if registered {
        talk.say(&format!(
            "{} has a {SERVER} entry already; kept.",
            cli.label()
        ))
    } else {
        let answer = talk.ask(&format!("Connect {}? [Y/n]", cli.label()))?;
        if matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes") {
            add(programs, talk, binary, cli)
        } else {
            Ok(())
        }
    }
}

fn add<R: BufRead, W: Write>(
    programs: &impl Program,
    talk: &mut Talk<R, W>,
    binary: &str,
    cli: Cli,
) -> Result<(), SetupError> {
    let (label, program) = (cli.label(), cli.program());
    match programs.run(program, &cli.add(binary)) {
        Ok(ran) if ran.success => talk.say(&format!("Connected {label}.")),
        Ok(ran) => {
            talk.say(&format!("Could not connect {label}: {}", ran.stderr.trim()))
        }
        Err(err) => talk.say(&format!("Could not run {program}: {err}")),
    }
}

/// The entries for clients set up by hand; nothing is written for the person.
fn snippets(binary: &str) -> String {
    // A JSON string is also a YAML double-quoted scalar.
    let quoted = json!(binary);
    let opencode = json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": { SERVER: { "type": "local", "command": [binary], "enabled": true } },
    });
    let generic = json!({ "mcpServers": { SERVER: { "command": binary } } });
    format!(
        "Other clients: add the server by hand.\n\
         \n\
         Hermes, in ~/.hermes/config.yaml:\n\
         mcp_servers:\n  {SERVER}:\n    command: {quoted}\n\
         \n\
         opencode, in opencode.json:\n{opencode:#}\n\
         \n\
         Any client that takes an mcpServers entry:\n{generic:#}"
    )
}

#[cfg(test)] mod tests;
