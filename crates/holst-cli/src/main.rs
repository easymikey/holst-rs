//! `holst`: a command-line client for the Holst REST API.

mod args;
mod cli;
mod commands;
mod error;
mod exit;
mod form;
mod help;
mod output;
mod run;
mod table;
mod terminal;

use std::process::ExitCode;

use holst_core::effect::settings::LogSettings;

use self::{
    error::Error,
    run::{Prompting, RunSettings},
};
use crate::{cli::Cli, exit::report, terminal::detect::TerminalType};

/// holst's thread stack: 8 MiB as on Unix; Windows gives main 1 MiB.
const STACK: usize = 8 << 20;

fn main() -> ExitCode {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map(|runtime| runtime.block_on(start()))
        })
        .ok()
        .and_then(|thread| thread.join().ok())
        .and_then(Result::ok)
        .unwrap_or(ExitCode::FAILURE)
}

#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the process arguments"
)]
async fn start() -> ExitCode {
    // First of all: from here on a Ctrl-C is counted, not fatal.
    terminal::signals::note_arrivals();
    match Cli::parse_from_args(std::env::args_os()) {
        Ok(cli) => run(cli).await,
        Err(err) => exit::usage(&err),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
async fn run(cli: Cli) -> ExitCode {
    // The one place the run's settings are built: flags, else the environment.
    let read_variable = |name: &str| std::env::var_os(name);
    init_tracing(
        &LogSettings::from_variables(read_variable),
        cli.verbose.then_some(["holst", "holst_core"]),
    );
    let dumb = read_variable("TERM").is_some_and(|name| name == "dumb");
    terminal::detect::mute_color_when(TerminalType::dumb_if(dumb));
    let home = std::env::home_dir().filter(|home| !home.as_os_str().is_empty());
    let (settings, cache_error) = cli.settings.into_settings(read_variable);
    // A bad cache variable leaves the cache off: logout and storage still work.
    cache_error
        .map(|err| format!("{err}; the board cache is off for this run"))
        .into_iter()
        .for_each(output::warn);
    let result = match terminal::signals::watch_apart() {
        // Ctrl-C fallback for commands that have no stop of their own.
        Ok(()) => {
            let term_type = TerminalType::dumb_if(dumb);
            let disabled =
                read_variable("HOLST_NO_PROMPT").is_some_and(|value| !value.is_empty());
            let prompting = Prompting::of(
                terminal::detect::standard_streams(),
                term_type,
                disabled,
                cli.output,
            );
            let env = RunSettings::new(settings, home, term_type, prompting);
            // Boxed: every command group's future in one would be large on the stack.
            let work = Box::pin(cli::run(cli.table, cli.output, &env));
            terminal::signals::or_stopped(work, terminal::signals::stopped_fallback())
                .await
        }
        Err(err) => Err(Error::Terminal(err)),
    };
    result.map_or_else(|err| report(&err), |()| ExitCode::SUCCESS)
}

fn init_tracing(log: &LogSettings, verbose_targets: Option<[&str; 2]>) {
    use holst_core::system::logging::{LevelFilter, Targets};

    let own = verbose_targets
        .map(|targets| targets.map(|target| (target, LevelFilter::DEBUG)));
    let default = Targets::new()
        .with_default(LevelFilter::WARN)
        .with_targets(own.into_iter().flatten());
    holst_core::system::logging::init(log, default);
}
