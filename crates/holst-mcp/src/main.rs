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

    #[error("the login did not finish")]
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
    let mut args = std::env::args().skip(1);
    let outcome = match (args.next(), args.next()) {
        (Some(command), None) if command == "setup" => Box::pin(log_in()).await,
        (Some(command), Some(extra)) if command == "setup" => {
            Err(StartError::UnknownArg { arg: extra })
        }
        (Some(arg), _) => print_usage(&arg),
        (None, _) => Box::pin(serve()).await,
    };
    outcome.map_or_else(|err| report(&err), |()| ExitCode::SUCCESS)
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
             Log in: holst-mcp setup (in a terminal; in a container: docker exec -it <container> holst-mcp setup)\n\
             Connect: claude mcp add holst-rs -- holst-mcp\n\
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

/// `holst-mcp setup`: the login the server uses, typed in a terminal.
#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
async fn log_in() -> Result<(), StartError> {
    io::stdin()
        .is_terminal()
        .then_some(())
        .ok_or(StartError::NotTerminal)?;
    let (settings, _) =
        holst_core::effect::settings::Settings::checked(|name| std::env::var_os(name))
            .map_err(StartError::Settings)?;
    init_tracing(&settings.log);
    let backend = login::store::SystemBackend::new(settings);
    let mut talk = setup::Talk::new(io::stdin().lock(), io::stderr().lock(), true);
    setup::run(&backend, &mut talk)
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
    use crate::{VERSION, usage};

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
