//! MCP server for Holst on stdio or HTTP, on the login of `setup`.

#[cfg(test)] mod budget;
mod error;
mod http;
mod login;
mod memory;
mod reply;
mod server;
mod setup;
mod tools;

use std::{
    io::{self, IsTerminal, Write as _},
    net::SocketAddr,
    process::ExitCode,
};

use holst_core::effect::settings::Settings;
use holst_form::{detect::TerminalType, form::Form, terminal::Console};
use rmcp::{ServiceExt as _, service::ServerInitializeError, transport::stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error(
        "unknown argument {arg}: the server speaks MCP on stdio; its commands are setup and --http"
    )]
    UnknownArg { arg: String },

    #[error(
        "unknown setup flag {flag}: setup takes --no-connect, --connect-only or --clients <list>"
    )]
    SetupFlag { flag: String },

    #[error("--clients needs a list such as claude,codex")]
    ClientsList,

    #[error("--clients names clients to connect, which --no-connect skips")]
    ClientsWithoutConnect,

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

    #[error(transparent)]
    Http(#[from] http::HttpError),

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
#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the process arguments"
)]
async fn main() -> ExitCode {
    // Else `--version` would wait for the protocol; stdout is still free.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        Some(("setup", flags)) => Box::pin(set_up(flags)).await,
        Some(("--http", rest)) => Box::pin(serve_http(rest)).await,
        Some((arg, _)) => print_usage(arg),
        None => Box::pin(serve()).await,
    };
    outcome.map_or_else(|err| report(&err), |()| ExitCode::SUCCESS)
}

/// The steps and the clients the flags of `holst-mcp setup` ask for.
fn setup_options(
    flags: &[String],
) -> Result<(setup::Steps, setup::Clients), StartError> {
    let (steps, clients, awaiting) = flags.iter().try_fold(
        (Vec::new(), setup::Clients::Ask, false),
        |(mut steps, clients, awaiting), flag| match (awaiting, flag.as_str()) {
            (true, list) => Ok((
                steps,
                setup::Clients::parse(list).map_err(StartError::Setup)?,
                false,
            )),
            (false, "--no-connect") => {
                steps.push(setup::Steps::LoginOnly);
                Ok((steps, clients, false))
            }
            (false, "--connect-only") => {
                steps.push(setup::Steps::ConnectOnly);
                Ok((steps, clients, false))
            }
            (false, "--clients") => Ok((steps, clients, true)),
            (false, _) => Err(StartError::SetupFlag { flag: flag.clone() }),
        },
    )?;
    awaiting
        .then_some(())
        .map_or(Ok(()), |()| Err(StartError::ClientsList))?;
    let steps = match steps.as_slice() {
        [] => Ok(setup::Steps::Both),
        [one] => Ok(*one),
        _ => Err(StartError::OneFlag),
    }?;
    match (steps, &clients) {
        (setup::Steps::LoginOnly, setup::Clients::Named(_)) => {
            Err(StartError::ClientsWithoutConnect)
        }
        _ => Ok((steps, clients)),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "`--help` and `--version` answer on stdout before the protocol starts"
)]
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
             Log in and connect Claude Code and Codex: holst-mcp setup [--no-connect | --connect-only] [--clients claude,codex]\n\
             (in a terminal; in a container: docker exec -it <container> holst-mcp setup)\n\
             \x20 --clients connects those clients without asking, for scripts (without a terminal: connects only, needs a saved login)\n\
             Other clients: setup prints the entry to put in their config\n\
             HTTP, for agents in containers: holst-mcp --http 127.0.0.1:<port> [--print-config]\n\
             \x20 serves http://<addr>/mcp for the bearer token of HOLST_MCP_TOKEN or <holst home>/mcp-token;\n\
             \x20 --print-config prints the claude, Codex and JSON settings with that token;\n\
             \x20 a container reaches the Mac's loopback as host.docker.internal (Docker Desktop, Colima)\n\
             \x20 or host.lima.internal (Colima); a non-loopback address needs an existing token\n\
             Log in for HTTP: holst-mcp setup in a Mac terminal when holst-mcp runs on the Mac;\n\
             \x20 docker exec -it <container> holst-mcp setup when it runs in a container (HOLST_HOME on a volume)\n\
             Environment: HOLST_HOME, HOLST_BASE_URL, HOLST_LOG"
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
fn read_settings() -> Result<(Settings, Option<holst_core::Error>), StartError> {
    Settings::checked(|name| std::env::var_os(name)).map_err(StartError::Settings)
}

async fn serve() -> Result<(), StartError> {
    let (settings, refused) = read_settings()?;
    let holst = start_server(settings, refused.as_ref());
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

/// `holst-mcp --http <addr>`: the same server on streamable HTTP.
#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
async fn serve_http(args: &[String]) -> Result<(), StartError> {
    let options = http::Options::parse(args)?;
    let (settings, refused) = read_settings()?;
    let file = settings
        .location
        .file()
        .map(|file| file.with_file_name(http::TOKEN_FILE));
    let variable = std::env::var(http::TOKEN_VAR).ok();
    let loopback = options.addr.ip().is_loopback();
    let create = loopback || options.print_config;
    let token = http::load_token(variable.as_deref(), file.as_deref(), create)?
        .ok_or(http::HttpError::NoToken { addr: options.addr })?;
    if options.print_config {
        let snippets = http::config_snippets(options.addr, &token);
        write!(io::stdout().lock(), "{snippets}").map_err(StartError::Stdout)
    } else {
        listen(options.addr, token, settings, refused.as_ref()).await
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the listening address goes to stderr, as stdout carries the protocol"
)]
async fn listen(
    addr: SocketAddr,
    token: http::Token,
    settings: Settings,
    refused: Option<&holst_core::Error>,
) -> Result<(), StartError> {
    let holst = start_server(settings, refused);
    let listener = http::bind(addr).await?;
    tracing::info!(tools = holst.tool_count(), %addr, "starting holst-mcp on HTTP");
    writeln!(io::stderr().lock(), "holst-mcp: serving http://{addr}/mcp")
        .map_err(StartError::Stdout)?;
    match http::serve(listener, holst, token).await {}
}

#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
fn start_server(
    settings: Settings,
    refused: Option<&holst_core::Error>,
) -> server::McpServer {
    init_tracing(&settings.log);
    // A bad cache variable leaves the board cache off; the server still starts.
    refused.into_iter().for_each(warn_cache_off);
    let secrets = tools::files::SecretPaths::new(
        settings.location.file().as_deref(),
        std::env::home_dir().as_deref(),
    );
    let mut holst = server::McpServer::new(settings, secrets);
    holst.downloads = std::env::home_dir().map(|home| home.join("Downloads"));
    holst
}

/// `holst-mcp setup`: the login the server uses, then the MCP clients.
#[expect(
    clippy::disallowed_methods,
    reason = "main.rs is the one place that reads the environment"
)]
async fn set_up(flags: &[String]) -> Result<(), StartError> {
    let (steps, clients) = setup_options(flags)?;
    // A script names its clients; no terminal needed.
    (io::stdin().is_terminal() || clients != setup::Clients::Ask)
        .then_some(())
        .ok_or(StartError::NotTerminal)?;
    let (settings, _) = read_settings()?;
    init_tracing(&settings.log);
    let backend = login::store::SystemBackend::new(settings);
    let binary = steps
        .connects()
        .then(|| {
            std::env::current_exe()
                .ok()
                .map(|exe| setup::stable_binary(&exe, std::path::Path::is_file))
                .and_then(|path| path.to_str().map(str::to_owned))
                .ok_or(StartError::Binary)
        })
        .transpose()?
        .unwrap_or_default();
    let dumb = std::env::var("TERM").is_ok_and(|name| name == "dumb");
    holst_form::detect::mute_color_when(TerminalType::dumb_if(dumb));
    let form = Form::intro(
        Console::detect(TerminalType::dumb_if(dumb)),
        "holst-mcp setup",
    )
    .map_err(|err| StartError::Setup(err.into()))?;
    setup::set_up(&backend, &form, &setup::System, &binary, steps, &clients)
        .await
        .map_err(StartError::Setup)
}

fn warn_cache_off(err: &holst_core::Error) {
    tracing::warn!(error = %reply::chained(err), "the board cache is off");
}

#[expect(
    clippy::disallowed_methods,
    reason = "a start error goes to stderr, as stdout carries the protocol"
)]
fn report(err: &StartError) -> ExitCode {
    let text = reply::chained(err);
    drop(
        writeln!(io::stderr().lock(), "holst-mcp: {text}").inspect_err(
            |failure| tracing::debug!(%failure, "could not write the start error"),
        ),
    );
    ExitCode::FAILURE
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
    use crate::{
        VERSION,
        setup::{Clients, Steps},
        setup_options,
        usage,
    };

    fn flags(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn setup_flags_pick_the_steps() {
        let steps = |words: &[&str]| setup_options(&flags(words)).ok().map(|(s, _)| s);
        assert_eq!(steps(&[]), Some(Steps::Both));
        assert_eq!(steps(&["--no-connect"]), Some(Steps::LoginOnly));
        assert_eq!(steps(&["--connect-only"]), Some(Steps::ConnectOnly));
    }

    #[test]
    fn the_clients_flag_names_the_clients() {
        let clients =
            |words: &[&str]| setup_options(&flags(words)).ok().map(|(_, c)| c);
        assert_eq!(clients(&[]), Some(Clients::Ask));
        assert_eq!(
            clients(&["--connect-only", "--clients", "claude,codex"]),
            Clients::parse("claude,codex").ok()
        );
    }

    #[test]
    fn setup_flags_reject_the_unknown_and_the_pair() {
        let shown =
            |words: &[&str]| setup_options(&flags(words)).err().map(|e| e.to_string());
        assert_eq!(
            shown(&["--port"]).as_deref(),
            Some(
                "unknown setup flag --port: setup takes --no-connect, --connect-only or --clients <list>"
            )
        );
        assert_eq!(
            shown(&["--no-connect", "--connect-only"]).as_deref(),
            Some("give setup one of --no-connect and --connect-only, once")
        );
        assert_eq!(
            shown(&["--clients"]).as_deref(),
            Some("--clients needs a list such as claude,codex")
        );
        assert_eq!(
            shown(&["--no-connect", "--clients", "codex"]).as_deref(),
            Some("--clients names clients to connect, which --no-connect skips")
        );
    }

    #[test]
    fn usage_knows_version_and_help_and_rejects_the_rest() {
        assert_eq!(usage("-V").ok(), Some(format!("holst-mcp {VERSION}")));
        assert!(usage("--help").is_ok_and(|text| text.contains("holst-mcp setup")));
        assert_eq!(
            usage("--port").err().map(|err| err.to_string()).as_deref(),
            Some(
                "unknown argument --port: the server speaks MCP on stdio; its commands are setup and --http"
            )
        );
    }
}
