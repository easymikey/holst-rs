//! MCP server for Holst over stdio, on the `holst` CLI's stored login.

#[cfg(test)] mod budget;
mod error;
mod login;
mod memory;
mod reply;
mod server;
mod setup;
mod tools;

use std::{
    io::{self, IsTerminal as _, Write as _},
    process::ExitCode,
};

use rmcp::{ServiceExt as _, service::ServerInitializeError, transport::stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error(
        "unknown argument {arg}: the server speaks MCP on stdio; its only command is setup"
    )]
    UnknownArg { arg: String },

    #[error(
        "stdin is not a terminal: run holst-mcp setup in a terminal \
         (in a container: docker exec -it <container> holst-mcp setup)"
    )]
    NotTerminal,

    #[error("give setup one of --no-connect and --connect-only, once")]
    OneFlag,

    #[error(
        "could not find the path of the running holst-mcp (it must be valid UTF-8)"
    )]
    Binary,

    #[error("setup did not finish")]
    Setup(#[source] setup::SetupError),

    #[error("could not write to stdout")]
    Stdout(#[source] io::Error),

    #[error("the server settings in the environment are not valid")]
    Settings(#[source] holst_core::Error),

    #[error("could not start the server on stdio")]
    Serve(#[source] Box<ServerInitializeError>),

    #[error("the server stopped with an error")]
    Stopped(#[source] tokio::task::JoinError),
}

#[tokio::main]
async fn main() -> ExitCode {
    // Else `--version` would wait for the protocol; stdout is still free.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.split_first() {
        Some((command, flags)) if command == "setup" => match setup_steps(flags) {
            Ok(steps) => Box::pin(set_up(steps)).await,
            Err(err) => Err(err),
        },
        Some((arg, _)) => print_usage(arg),
        None => Box::pin(serve()).await,
    };
    outcome.map_or_else(|err| report(&err), |()| ExitCode::SUCCESS)
}

/// The steps the flags of `holst-mcp setup` ask for.
fn setup_steps(flags: &[String]) -> Result<setup::Steps, StartError> {
    let asked = flags
        .iter()
        .map(|flag| match flag.as_str() {
            "--no-connect" => Ok(setup::Steps::LoginOnly),
            "--connect-only" => Ok(setup::Steps::ConnectOnly),
            _ => Err(StartError::UnknownArg { arg: flag.clone() }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    match asked.as_slice() {
        [] => Ok(setup::Steps::Both),
        [one] => Ok(*one),
        _ => Err(StartError::OneFlag),
    }
}

fn print_usage(arg: &str) -> Result<(), StartError> {
    usage(arg).and_then(|text| {
        writeln!(io::stdout().lock(), "{text}").map_err(StartError::Stdout)
    })
}

fn usage(arg: &str) -> Result<String, StartError> {
    match arg {
        "--version" | "-V" => Ok(format!("holst-mcp {VERSION}")),
        "--help" | "-h" => Ok(format!(
            "holst-mcp {VERSION} — the Holst MCP server on stdio.\n\
             Log in and connect Claude Code and Codex: holst-mcp setup [--no-connect | --connect-only]\n\
             (in a terminal; in a container: docker exec -it <container> holst-mcp setup)\n\
             Other clients: setup prints the entry to put in their config\n\
             Environment: HOLST_HOME, HOLST_BASE_URL, HOLST_LOG, HOLST_MCP_RAW_WRITE=1"
        )),
        other => Err(StartError::UnknownArg {
            arg: other.to_owned(),
        }),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
async fn serve() -> Result<(), StartError> {
    let (settings, refused) =
        holst_core::effect::settings::Settings::checked(|name| std::env::var_os(name))
            .map_err(StartError::Settings)?;
    init_tracing(&settings.log);
    // A bad cache variable leaves the board cache off; the server still starts.
    refused.iter().for_each(warn_cache_off);
    let raw_write = tools::admin::RawWrite::from_setting(
        std::env::var("HOLST_MCP_RAW_WRITE").ok().as_deref(),
    );
    let secrets = tools::files::SecretPaths::new(
        settings.location.file().as_deref(),
        std::env::home_dir().as_deref(),
    );
    let mut holst = server::McpServer::new(settings, raw_write, secrets);
    holst.downloads = std::env::home_dir().map(|home| home.join("Downloads"));
    tracing::info!(tools = holst.tool_count(), "starting holst-mcp");
    let service = holst
        .serve(stdio())
        .await
        .map_err(|err| StartError::Serve(Box::new(err)))?;
    service
        .waiting()
        .await
        .map(drop)
        .map_err(StartError::Stopped)
}

/// `holst-mcp setup`: the login the server uses, then the MCP clients.
#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
async fn set_up(steps: setup::Steps) -> Result<(), StartError> {
    io::stdin()
        .is_terminal()
        .then_some(())
        .ok_or(StartError::NotTerminal)?;
    let (settings, _) =
        holst_core::effect::settings::Settings::checked(|name| std::env::var_os(name))
            .map_err(StartError::Settings)?;
    init_tracing(&settings.log);
    let backend = login::store::SystemBackend::new(settings);
    let binary = steps
        .connects()
        .then(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.to_str().map(str::to_owned))
                .ok_or(StartError::Binary)
        })
        .transpose()?
        .unwrap_or_default();
    let mut talk = setup::Talk::new(io::stdin().lock(), io::stderr().lock(), true);
    setup::set_up(&backend, &mut talk, &setup::System, &binary, steps)
        .await
        .map_err(StartError::Setup)
}

fn warn_cache_off(err: &holst_core::Error) {
    tracing::warn!(error = %reply::chained(err), "the board cache is off");
}

fn report(err: &StartError) -> ExitCode {
    let text = reply::chained(err);
    writeln!(io::stderr().lock(), "holst-mcp: {text}")
        .inspect_err(
            |failure| tracing::debug!(%failure, "could not write the start error"),
        )
        .map_or(ExitCode::FAILURE, |()| ExitCode::FAILURE)
}

fn init_tracing(log: &holst_core::effect::settings::LogSettings) {
    use holst_core::system::logging::{LevelFilter, Targets};
    holst_core::system::logging::init(
        log,
        Targets::new().with_default(LevelFilter::WARN),
    );
}

#[cfg(test)]
mod tests {
    use crate::{VERSION, setup::Steps, setup_steps, usage};

    fn flags(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn setup_flags_pick_the_steps() {
        assert_eq!(setup_steps(&flags(&[])).ok(), Some(Steps::Both));
        assert_eq!(
            setup_steps(&flags(&["--no-connect"])).ok(),
            Some(Steps::LoginOnly)
        );
        assert_eq!(
            setup_steps(&flags(&["--connect-only"])).ok(),
            Some(Steps::ConnectOnly)
        );
    }

    #[test]
    fn setup_flags_reject_the_unknown_and_the_pair() {
        let shown =
            |words: &[&str]| setup_steps(&flags(words)).err().map(|e| e.to_string());
        assert_eq!(
            shown(&["--port"]).as_deref(),
            Some(
                "unknown argument --port: the server speaks MCP on stdio; its only command is setup"
            )
        );
        assert_eq!(
            shown(&["--no-connect", "--connect-only"]).as_deref(),
            Some("give setup one of --no-connect and --connect-only, once")
        );
    }

    #[test]
    fn usage_knows_version_and_help_and_rejects_the_rest() {
        assert_eq!(usage("-V").ok(), Some(format!("holst-mcp {VERSION}")));
        assert!(usage("--help").is_ok_and(|text| text.contains("HOLST_MCP_RAW_WRITE")));
        assert!(usage("--help").is_ok_and(|text| text.contains("holst-mcp setup")));
        assert_eq!(
            usage("--port").err().map(|err| err.to_string()).as_deref(),
            Some(
                "unknown argument --port: the server speaks MCP on stdio; its only command is setup"
            )
        );
    }
}
