//! `holst-mcp --http <addr>`: MCP over streamable HTTP, behind a bearer token.

#[cfg(test)] mod tests;

use std::{
    convert::Infallible,
    io,
    net::{AddrParseError, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use base64::Engine as _;
use holst_core::{
    effect::clock::Clock as _,
    error::local_data::LocalDataError,
    system::clock::SystemClock,
};
use http_body_util::{BodyExt as _, Full, combinators::BoxBody};
use hyper::{
    Request,
    Response,
    StatusCode,
    body::{Bytes, Incoming},
    header::{AUTHORIZATION, HeaderValue, WWW_AUTHENTICATE},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig,
    StreamableHttpService,
    session::never::NeverSessionManager,
};
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use tokio::net::TcpListener;

use crate::server::McpServer;

/// The variable that hands the token to the server instead of the file.
pub(crate) const TOKEN_VAR: &str = "HOLST_MCP_TOKEN";

/// The token file's name, beside the login file in the holst home.
pub(crate) const TOKEN_FILE: &str = "mcp-token";

/// Names a container uses for the loopback of its host; Colima knows both.
const HOST_NAMES: [&str; 2] = ["host.docker.internal", "host.lima.internal"];

const PATH: &str = "/mcp";

/// The pause after a failed accept, such as a process out of file handles.
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpError {
    #[error("--http needs an address to listen on, like 127.0.0.1:8808")]
    MissingAddr,

    #[error("{text} is not an address to listen on, like 127.0.0.1:8808")]
    BadAddr {
        text: String,
        #[source]
        source: AddrParseError,
    },

    #[error("unknown argument {arg}: after --http come an address and --print-config")]
    UnknownArg { arg: String },

    #[error(
        "{addr} is not a loopback address and there is no token yet: set \
         {TOKEN_VAR}, or run holst-mcp --http 127.0.0.1:<port> --print-config \
         once to create {TOKEN_FILE}"
    )]
    NoToken { addr: SocketAddr },

    #[error("there is no place for the token file: set HOLST_HOME")]
    NoTokenPlace,

    #[error("could not read the token file")]
    TokenRead(#[source] holst_core::Error),

    #[error("could not write the token file")]
    TokenWrite(#[source] holst_core::Error),

    #[error("the system has no random bytes for a token")]
    Random(#[source] getrandom::Error),

    #[error("could not listen on {addr}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
}

/// What follows `--http` on the command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub addr: SocketAddr,
    pub print_config: bool,
}

impl Options {
    pub(crate) fn parse(args: &[String]) -> Result<Self, HttpError> {
        let print_config = args.iter().any(|arg| arg == "--print-config");
        let words: Vec<&String> =
            args.iter().filter(|arg| *arg != "--print-config").collect();
        match words.as_slice() {
            [] => Err(HttpError::MissingAddr),
            [text] => text
                .parse()
                .map(|addr| Self { addr, print_config })
                .map_err(|source| HttpError::BadAddr {
                    text: (*text).clone(),
                    source,
                }),
            [_, extra, ..] => Err(HttpError::UnknownArg {
                arg: (*extra).clone(),
            }),
        }
    }
}

/// The secret every request has to send as `Authorization: Bearer <token>`.
#[derive(Debug, Clone)]
pub(crate) struct Token(SecretString);

impl Token {
    pub(crate) fn new(text: &str) -> Self {
        Self(SecretString::from(text))
    }

    fn generate() -> Result<Self, HttpError> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(HttpError::Random)?;
        Ok(Self::new(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        ))
    }

    /// Whether an `Authorization` header carries this token; the compare
    /// takes the same time wherever the texts differ.
    fn admits(&self, header: Option<&str>) -> bool {
        header
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .is_some_and(|(_, sent)| {
                bool::from(
                    sent.trim()
                        .as_bytes()
                        .ct_eq(self.0.expose_secret().as_bytes()),
                )
            })
    }

    /// The token text, for the snippets that `--print-config` prints.
    fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

/// The token from the variable, else from the file; a missing or empty file
/// is created when `create` says so.
pub(crate) fn load_token(
    variable: Option<&str>,
    file: Option<&Path>,
    create: bool,
) -> Result<Option<Token>, HttpError> {
    variable
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map_or_else(
            || token_from_file(file, create),
            |text| Ok(Some(Token::new(text))),
        )
}

fn token_from_file(
    file: Option<&Path>,
    create: bool,
) -> Result<Option<Token>, HttpError> {
    let path = file.ok_or(HttpError::NoTokenPlace)?;
    match (read_token(path)?, create) {
        (Some(token), _) => Ok(Some(token)),
        (None, true) => make_token(path).map(Some),
        (None, false) => Ok(None),
    }
}

fn read_token(path: &Path) -> Result<Option<Token>, HttpError> {
    match holst_core::storage::file::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            Ok(Some(text.trim())
                .filter(|text| !text.is_empty())
                .map(Token::new))
        }
        Err(holst_core::Error::LocalData(LocalDataError::FileRead {
            error, ..
        })) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(HttpError::TokenRead(err)),
    }
}

fn make_token(path: &Path) -> Result<Token, HttpError> {
    let token = Token::generate()?;
    path.parent()
        .map_or(Ok(()), holst_core::storage::file::create_dir_all)
        .and_then(|()| {
            holst_core::storage::file::replace_private(
                path,
                &[token.expose().as_bytes()],
            )
        })
        .map_err(HttpError::TokenWrite)?;
    Ok(token)
}

/// Binds the address; a non-loopback one is announced to the log.
pub(crate) async fn bind(addr: SocketAddr) -> Result<TcpListener, HttpError> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| HttpError::Bind { addr, source })?;
    if !addr.ip().is_loopback() {
        tracing::warn!(
            %addr,
            "not a loopback address: every host that reaches it and sends the \
             token can use the Holst login of this account"
        );
    }
    Ok(listener)
}

type Reply = Response<BoxBody<Bytes, Infallible>>;

type Mcp = StreamableHttpService<McpServer, NeverSessionManager>;

/// Answers MCP over HTTP on `listener`; a failed accept is logged and tried
/// again. No session is kept: every request is complete in itself.
pub(crate) async fn serve(
    listener: TcpListener,
    holst: McpServer,
    token: Token,
) -> Infallible {
    let loopback = listener
        .local_addr()
        .is_ok_and(|addr| addr.ip().is_loopback());
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_allowed_hosts(
            ["localhost", "127.0.0.1", "[::1]"]
                .into_iter()
                .chain(HOST_NAMES),
        );
    // Beyond loopback the names clients use are not known; the token guards.
    let config = if loopback {
        config
    } else {
        config.disable_allowed_hosts()
    };
    let mcp = StreamableHttpService::new(
        move || Ok(holst.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    let token = Arc::new(token);
    // imperative: the accept loop of a server
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let (mcp, token) = (mcp.clone(), Arc::clone(&token));
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let (mcp, token) = (mcp.clone(), Arc::clone(&token));
                        async move {
                            Ok::<_, Infallible>(answer(&mcp, &token, request).await)
                        }
                    });
                    if let Err(err) = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await
                    {
                        tracing::debug!(%err, "a connection ended with an error");
                    }
                });
            }
            Err(err) => {
                tracing::warn!(%err, "accept failed");
                SystemClock.sleep(ACCEPT_PAUSE).await;
            }
        }
    }
}

async fn answer(mcp: &Mcp, token: &Token, request: Request<Incoming>) -> Reply {
    let sent = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    match (token.admits(sent), request.uri().path() == PATH) {
        (false, _) => unauthorized(),
        (true, false) => refusal(StatusCode::NOT_FOUND, "the MCP endpoint is /mcp"),
        (true, true) => mcp.handle(request).await,
    }
}

fn refusal(status: StatusCode, text: &'static str) -> Reply {
    let mut reply =
        Response::new(Full::new(Bytes::from_static(text.as_bytes())).boxed());
    *reply.status_mut() = status;
    reply
}

fn unauthorized() -> Reply {
    let mut reply = refusal(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    reply
        .headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    reply
}

/// The commands that connect a client in a container to the server at `addr`.
pub(crate) fn config_snippets(addr: SocketAddr, token: &Token) -> String {
    let [docker, lima] = HOST_NAMES;
    let port = addr.port();
    let url = format!("http://{docker}:{port}{PATH}");
    let token = token.expose();
    format!(
        "# Agents in Docker reach the Mac's loopback as {docker} (Colima also {lima}).\n\
         # Agents on the Mac itself: use 127.0.0.1 instead.\n\
         \n\
         # Claude Code\n\
         claude mcp add --transport http holst-rs {url} --header \"Authorization: Bearer {token}\"\n\
         \n\
         # Codex, ~/.codex/config.toml\n\
         [mcp_servers.holst-rs]\n\
         url = \"{url}\"\n\
         http_headers = {{ Authorization = \"Bearer {token}\" }}\n\
         \n\
         # Any other client\n\
         {{\"mcpServers\": {{\"holst-rs\": {{\"type\": \"http\", \"url\": \"{url}\", \
         \"headers\": {{\"Authorization\": \"Bearer {token}\"}}}}}}}}\n\
         \n\
         # Log in once, never through the chat:\n\
         #   holst-mcp running on the Mac: holst-mcp setup, in a Mac terminal\n\
         #   holst-mcp running in a container: docker exec -it <container> holst-mcp setup --no-connect\n\
         #     (keep HOLST_HOME on a volume so the login survives a restart)\n"
    )
}
