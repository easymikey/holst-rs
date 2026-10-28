#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests use real temp files and real sockets on loopback"
)]

use std::{fs, net::SocketAddr};

use rmcp::{
    ServiceExt as _,
    transport::{
        StreamableHttpClientTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use super::{HttpError, Options, Token, bind, load_token, serve};
use crate::server::McpServer;

/// What can go wrong between a test and the server it asks.
#[derive(Debug, thiserror::Error)]
enum TestError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ask(#[from] reqwest::Error),
    #[error(transparent)]
    Start(Box<rmcp::service::ClientInitializeError>),
    #[error(transparent)]
    Service(#[from] rmcp::ServiceError),
}

/// A server on a free loopback port; the task ends with the test.
async fn running(token: &str) -> Result<SocketAddr, TestError> {
    let listener = bind("127.0.0.1:0".parse().map_err(std::io::Error::other)?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(serve(listener, McpServer::offline(), Token::new(token)));
    Ok(addr)
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

#[tokio::test]
async fn a_client_with_the_token_lists_the_tools() -> Result<(), TestError> {
    let addr = running("sesame").await?;
    let config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"))
            .auth_header("sesame");
    let client =
        ().serve(StreamableHttpClientTransport::from_config(config))
            .await
            .map_err(|err| TestError::Start(Box::new(err)))?;
    let listed = client.list_all_tools().await?;
    assert!(listed.iter().any(|tool| tool.name == "workspace"));
    Ok(())
}

#[tokio::test]
async fn a_request_without_the_token_gets_401() -> Result<(), TestError> {
    let addr = running("sesame").await?;
    let url = format!("http://{addr}/mcp");
    let http = reqwest::Client::new();
    let bare = http.post(&url).send().await?;
    let wrong = http.post(&url).bearer_auth("sesamf").send().await?;
    let other_scheme = http
        .post(&url)
        .header("Authorization", "sesame")
        .send()
        .await?;
    assert_eq!(bare.status(), 401);
    assert_eq!(wrong.status(), 401);
    assert_eq!(other_scheme.status(), 401);
    assert_eq!(
        bare.headers()
            .get("www-authenticate")
            .map(reqwest::header::HeaderValue::as_bytes),
        Some(b"Bearer".as_slice())
    );
    Ok(())
}

#[tokio::test]
async fn a_client_without_the_token_cannot_start() -> Result<(), TestError> {
    let addr = running("sesame").await?;
    let config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"));
    let started = ().serve(StreamableHttpClientTransport::from_config(config)).await;
    assert!(started.is_err());
    Ok(())
}

#[tokio::test]
async fn another_path_is_not_found_with_the_token() -> Result<(), TestError> {
    let addr = running("sesame").await?;
    let reply = reqwest::Client::new()
        .post(format!("http://{addr}/elsewhere"))
        .bearer_auth("sesame")
        .send()
        .await?;
    assert_eq!(reply.status(), 404);
    Ok(())
}

#[tokio::test]
async fn the_host_check_stops_a_stranger_and_admits_the_docker_name()
-> Result<(), TestError> {
    let addr = running("sesame").await?;
    let post = |host: String| {
        reqwest::Client::new()
            .post(format!("http://{addr}/mcp"))
            .bearer_auth("sesame")
            .header("Host", host)
            .send()
    };
    let stranger = post("evil.example".to_owned()).await?;
    let docker = post(format!("host.docker.internal:{}", addr.port())).await?;
    assert_eq!(stranger.status(), 403);
    assert_ne!(docker.status(), 403);
    Ok(())
}

#[test]
fn the_token_comes_from_the_variable_before_the_file() -> Result<(), HttpError> {
    let token = load_token(Some(" from-env \n"), None, false)?;
    assert!(token.is_some_and(|token| token.expose() == "from-env"));
    Ok(())
}

#[test]
fn a_missing_file_is_created_private_once() -> Result<(), TestError> {
    let dir =
        std::env::temp_dir().join(format!("holst-mcp-token-{}", std::process::id()));
    let file = dir.join("mcp-token");
    let absent = load_token(None, Some(&file), false)?;
    let made = load_token(None, Some(&file), true)?;
    let again = load_token(None, Some(&file), true)?;
    let text = fs::read_to_string(&file)?;
    fs::remove_dir_all(&dir)?;
    assert!(absent.is_none());
    assert_eq!(
        made.map(|token| token.expose().to_owned()),
        Some(text.clone())
    );
    assert_eq!(
        again.map(|token| token.expose().to_owned()),
        Some(text.clone())
    );
    assert_eq!(text.len(), 43);
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_token_file_is_readable_by_its_owner_only() -> Result<(), TestError> {
    use std::os::unix::fs::PermissionsExt as _;
    let dir =
        std::env::temp_dir().join(format!("holst-mcp-mode-{}", std::process::id()));
    let file = dir.join("mcp-token");
    load_token(None, Some(&file), true)?;
    let mode = fs::metadata(&file)?.permissions().mode() & 0o777;
    fs::remove_dir_all(&dir)?;
    assert_eq!(mode, 0o600);
    Ok(())
}

#[test]
fn the_options_are_an_address_and_a_flag() {
    let plain = Options::parse(&strings(&["127.0.0.1:8808"]));
    let printing = Options::parse(&strings(&["--print-config", "127.0.0.1:8808"]));
    let none = Options::parse(&[]);
    let bad = Options::parse(&strings(&["8808"]));
    let extra = Options::parse(&strings(&["127.0.0.1:8808", "--port"]));
    assert_eq!(
        plain.ok(),
        Some(Options {
            addr: SocketAddr::from(([127, 0, 0, 1], 8808)),
            print_config: false
        })
    );
    assert!(printing.is_ok_and(|options| options.print_config));
    assert!(matches!(none, Err(HttpError::MissingAddr)));
    assert!(matches!(bad, Err(HttpError::BadAddr { .. })));
    assert!(matches!(extra, Err(HttpError::UnknownArg { .. })));
}
