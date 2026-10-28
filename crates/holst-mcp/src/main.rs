//! MCP server for Holst over stdio, on the `holst` CLI's stored login.

#[cfg(test)] mod budget;
mod error;
mod login;
mod memory;
mod reply;
mod server;
mod tools;

use std::{io, io::Write as _, process::ExitCode};

use rmcp::{ServiceExt as _, service::ServerInitializeError, transport::stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error(
        "unknown argument {arg}: the server speaks MCP on stdio and takes no arguments"
    )]
    UnknownArg { arg: String },

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
    let outcome = match std::env::args().nth(1) {
        Some(arg) => print_usage(&arg),
        None => Box::pin(serve()).await,
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
             Connect: claude mcp add holst-rs -- holst-mcp\n\
             Environment: HOLST_HOME, HOLST_BASE_URL, HOLST_LOG, HOLST_MCP_RAW_WRITE=1"
        )),
        other => Err(StartError::UnknownArg {
            arg: other.to_owned(),
        }),
    }
}

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
        assert_eq!(
            usage("--port").err().map(|err| err.to_string()).as_deref(),
            Some(
                "unknown argument --port: the server speaks MCP on stdio and takes no arguments"
            )
        );
    }
}
