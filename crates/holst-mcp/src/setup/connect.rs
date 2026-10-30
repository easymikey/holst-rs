//! The connect step of `holst-mcp setup`: the MCP clients get the server.

use std::{
    io,
    process::{Command, Stdio},
};

use holst_form::{error::FormError, form::Form, prompt::Choice, terminal::Terminal};
use serde_json::json;

use super::SetupError;

/// The name the server is registered under in every client.
pub(crate) const SERVER: &str = "holst-rs";

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
    #[expect(
        clippy::disallowed_methods,
        reason = "setup runs the clients' own `mcp add` commands; this is the one door"
    )]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cli {
    Claude,
    Codex,
}

impl Cli {
    const ALL: [Cli; 2] = [Cli::Claude, Cli::Codex];

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

/// Which clients to connect: ask for the ones found, or the ones named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Clients {
    Ask,
    /// `--clients claude,codex`
    Named(Vec<Cli>),
}

impl Clients {
    /// The clients of a comma-separated list of program names.
    pub(crate) fn parse(list: &str) -> Result<Self, SetupError> {
        list.split(',')
            .map(|name| {
                Cli::ALL
                    .into_iter()
                    .find(|cli| cli.program() == name)
                    .ok_or_else(|| SetupError::UnknownClient(name.to_owned()))
            })
            .collect::<Result<_, _>>()
            .map(Clients::Named)
    }

    fn names(&self, cli: Cli) -> bool {
        match self {
            Clients::Ask => true,
            Clients::Named(named) => named.contains(&cli),
        }
    }
}

/// Connects the clients asked for without an entry, then prints what to put in
/// the others.
pub(super) fn connect<T: Terminal>(
    programs: &impl Program,
    form: &Form<T>,
    binary: &str,
    clients: &Clients,
) -> Result<(), SetupError> {
    let without = without_entry(programs, form, clients)?;
    chosen(form, clients, &without)?
        .into_iter()
        .try_for_each(|cli| add(programs, form, binary, cli))?;
    form.terminal()
        .write(&format!("{}\n", snippets(binary)))
        .map_err(|err| SetupError::Form(FormError::Terminal(err)))
}

/// The wanted clients that can take an entry; a named client off `PATH` stays,
/// so that its failure is told.
fn without_entry<T: Terminal>(
    programs: &impl Program,
    form: &Form<T>,
    clients: &Clients,
) -> Result<Vec<Cli>, SetupError> {
    Ok(Cli::ALL
        .into_iter()
        .filter(|cli| clients.names(*cli))
        .map(
            |cli| match programs.run(cli.program(), &["mcp", "get", SERVER]) {
                Err(_) if *clients == Clients::Ask => Ok(None),
                Ok(ran) if ran.success => form
                    .step(
                        &format!("{} has a {SERVER} entry already; kept.", cli.label()),
                        "",
                    )
                    .map(|()| None),
                _ => Ok(Some(cli)),
            },
        )
        .filter_map(Result::transpose)
        .collect::<Result<_, FormError>>()?)
}

/// All of `without` when named, else the ones the person leaves picked.
fn chosen<T: Terminal>(
    form: &Form<T>,
    clients: &Clients,
    without: &[Cli],
) -> Result<Vec<Cli>, SetupError> {
    match (clients, without) {
        (Clients::Named(_), _) | (Clients::Ask, []) => Ok(without.to_vec()),
        (Clients::Ask, _) => {
            let options = without
                .iter()
                .map(|cli| Choice {
                    label: cli.label().to_owned(),
                    hint: None,
                })
                .collect();
            let places = form.multiselect(
                "Connect which clients?",
                options,
                vec![true; without.len()],
                false,
            )?;
            Ok(places
                .into_iter()
                .filter_map(|place| without.get(place).copied())
                .collect())
        }
    }
}

fn add<T: Terminal>(
    programs: &impl Program,
    form: &Form<T>,
    binary: &str,
    cli: Cli,
) -> Result<(), SetupError> {
    let (label, program) = (cli.label(), cli.program());
    Ok(match programs.run(program, &cli.add(binary)) {
        Ok(ran) if ran.success => form.step(&format!("Connected {label}."), ""),
        Ok(ran) => form.step(&format!("Could not connect {label}"), ran.stderr.trim()),
        Err(err) => form.step(&format!("Could not run {program}"), &err.to_string()),
    }?)
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

#[cfg(test)] pub(super) mod tests;
