//! The board socket through the proxy from `HTTPS_PROXY`, as REST requests go.

use hyper_util::client::proxy::matcher::Matcher;
use reqwest::Url;
use secrecy::SecretString;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};
use tokio_tungstenite::tungstenite::http::Uri;

use crate::{
    domain::text::printable,
    error::{Error, Result, board_socket::BoardSocketError},
};

/// How the board socket reaches its host.
#[derive(Debug, Clone)]
pub enum ProxyRoute {
    Direct,
    Tunnel {
        proxy: String,
        target: String,
        authorization: Option<SecretString>,
    },
}

/// The route to `socket` under `matcher` (`ws` as `http`, `wss` as `https`).
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::ProxyScheme`] for a proxy other than `http://`.
pub fn route(matcher: &Matcher, socket: &Url) -> Result<ProxyRoute> {
    let host = socket.host_str().unwrap_or_default();
    let port = socket.port_or_known_default().unwrap_or(443);
    let scheme = match socket.scheme() {
        "ws" => "http",
        _ => "https",
    };
    let target = format!("{host}:{port}");
    let destination = format!("{scheme}://{target}/").parse::<Uri>().ok();
    let intercept = destination.and_then(|uri| matcher.intercept(&uri));
    intercept.map_or(Ok(ProxyRoute::Direct), |proxy| {
        let uri = proxy.uri();
        let proxy_host = uri.host().unwrap_or_default();
        match uri.scheme_str() {
            Some("http") => Ok(ProxyRoute::Tunnel {
                proxy: format!("{proxy_host}:{}", uri.port_u16().unwrap_or(80)),
                target,
                authorization: proxy
                    .basic_auth()
                    .and_then(|authorization| authorization.to_str().ok())
                    .map(SecretString::from),
            }),
            other => Err(Error::BoardSocket(BoardSocketError::ProxyScheme {
                scheme: other.unwrap_or_default().to_owned(),
                host: proxy_host.to_owned(),
            })),
        }
    })
}

/// The `CONNECT` request for `target`, with the proxy login if any.
#[must_use]
pub fn connect_request(target: &str, authorization: Option<&str>) -> String {
    let login = authorization
        .map(|authorization| format!("Proxy-Authorization: {authorization}\r\n"))
        .unwrap_or_default();
    format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{login}\r\n")
}

/// Whether the proxy's answer opens the tunnel: a 2xx status line.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::ProxyDenied`] with the
/// status line otherwise.
pub fn opened(proxy: &str, head: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(head);
    let status = text.lines().next().unwrap_or_default();
    let code = status.split(' ').nth(1).unwrap_or_default();
    if status.starts_with("HTTP/1.") && code.starts_with('2') && code.len() == 3 {
        Ok(())
    } else {
        Err(Error::BoardSocket(BoardSocketError::ProxyDenied {
            proxy: proxy.to_owned(),
            detail: printable(status).into_owned(),
        }))
    }
}

const MAX_HEAD: usize = 16 * 1024;

/// A TCP stream to `target` through the proxy, ready for TLS and the socket.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::Proxy`] when the proxy
/// cannot be reached or hangs up, and
/// [`crate::error::board_socket::BoardSocketError::ProxyDenied`] when it says
/// no.
pub async fn tunnel(
    proxy: &str,
    target: &str,
    authorization: Option<&str>,
) -> Result<TcpStream> {
    let failed = |source| {
        Error::BoardSocket(BoardSocketError::Proxy {
            proxy: proxy.to_owned(),
            error: source,
        })
    };
    let mut stream = TcpStream::connect(proxy).await.map_err(failed)?;
    stream
        .write_all(connect_request(target, authorization).as_bytes())
        .await
        .map_err(failed)?;
    let head = read_head(&mut stream).await.map_err(failed)?;
    opened(proxy, &head).map(|()| stream)
}

async fn read_head(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(128);
    // imperative: a byte per await to the blank line; a buffer reads past it
    while !head.ends_with(b"\r\n\r\n") && head.len() < MAX_HEAD {
        head.push(stream.read_u8().await?);
    }
    Ok(head)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use hyper_util::client::proxy::matcher::Matcher;
    use reqwest::Url;
    use rstest::rstest;
    use secrecy::ExposeSecret as _;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    use crate::{
        error::{Error, Result, board_socket::BoardSocketError},
        remote::socket::proxy::{ProxyRoute, connect_request, opened, route, tunnel},
    };

    fn socket(raw: &str) -> Option<Url> {
        Url::parse(raw).ok()
    }

    fn matcher(all: &str, no: &str) -> Matcher {
        Matcher::builder().all(all).no(no).build()
    }

    /// A route with its login exposed, to compare.
    #[derive(Debug, PartialEq, Eq)]
    enum Reached {
        Direct,
        Tunnel {
            proxy: String,
            target: String,
            authorization: Option<String>,
        },
    }

    fn reached(route: ProxyRoute) -> Reached {
        match route {
            ProxyRoute::Direct => Reached::Direct,
            ProxyRoute::Tunnel {
                proxy,
                target,
                authorization,
            } => Reached::Tunnel {
                proxy,
                target,
                authorization: authorization
                    .map(|login| login.expose_secret().to_owned()),
            },
        }
    }

    #[rstest]
    #[case::no_proxy_set("", "", Some(Reached::Direct))]
    #[case::through_the_proxy(
        "http://proxy.test:3128",
        "",
        Some(Reached::Tunnel {
            proxy: "proxy.test:3128".into(),
            target: "app.holst.so:443".into(),
            authorization: None,
        })
    )]
    #[case::default_proxy_port(
        "proxy.test",
        "",
        Some(Reached::Tunnel {
            proxy: "proxy.test:80".into(),
            target: "app.holst.so:443".into(),
            authorization: None,
        })
    )]
    #[case::with_a_login(
        "http://Aladdin:opensesame@proxy.test:8080",
        "",
        Some(Reached::Tunnel {
            proxy: "proxy.test:8080".into(),
            target: "app.holst.so:443".into(),
            authorization: Some("Basic QWxhZGRpbjpvcGVuc2VzYW1l".into()),
        })
    )]
    #[case::host_bypassed(
        "http://proxy.test:3128",
        "localhost, .holst.so",
        Some(Reached::Direct)
    )]
    #[case::socks_is_not_spoken("socks5://proxy.test:1080", "", None)]
    fn the_socket_goes_where_rest_goes(
        #[case] all: &str,
        #[case] no: &str,
        #[case] expected: Option<Reached>,
    ) {
        let got = socket("wss://app.holst.so/hud/ws")
            .map(|url| route(&matcher(all, no), &url));
        assert_eq!(got.and_then(Result::ok).map(reached), expected);
    }

    #[test]
    fn an_unusable_proxy_is_named_without_its_login() {
        let err = socket("wss://app.holst.so/hud/ws")
            .map(|url| route(&matcher("https://user:secret@proxy.test:443", ""), &url));
        let shown = err.and_then(Result::err).map(|err| err.to_string());
        assert_eq!(
            shown.as_deref(),
            Some(
                "the board connection goes only through an http:// proxy, not https://proxy.test"
            )
        );
    }

    #[test]
    fn debug_of_a_route_hides_the_login() {
        let route = ProxyRoute::Tunnel {
            proxy: "proxy.test:80".into(),
            target: "app.holst.so:443".into(),
            authorization: Some("Basic QWxhZGRpbjpvcGVuc2VzYW1l".into()),
        };
        let shown = format!("{route:?}");
        assert!(shown.contains("proxy.test:80"));
        assert!(!shown.contains("QWxhZGRpbjpvcGVuc2VzYW1l"));
    }

    #[test]
    fn connect_request_names_the_target_and_the_login() {
        assert_eq!(
            connect_request("app.holst.so:443", Some("Basic eA==")),
            "CONNECT app.holst.so:443 HTTP/1.1\r\nHost: app.holst.so:443\r\nProxy-Authorization: Basic eA==\r\n\r\n"
        );
        assert_eq!(
            connect_request("h:1", None),
            "CONNECT h:1 HTTP/1.1\r\nHost: h:1\r\n\r\n"
        );
    }

    #[rstest]
    #[case::established("HTTP/1.1 200 Connection established\r\n\r\n", Tunnel::Opens)]
    #[case::bare_ok("HTTP/1.0 200\r\n\r\n", Tunnel::Opens)]
    #[case::login_needed(
        "HTTP/1.1 407 Proxy Authentication Required\r\n\r\n",
        Tunnel::Stays
    )]
    #[case::forbidden("HTTP/1.1 403 Forbidden\r\n\r\n", Tunnel::Stays)]
    #[case::not_http("SSH-2.0-OpenSSH\r\n", Tunnel::Stays)]
    #[case::nothing("", Tunnel::Stays)]
    fn only_a_2xx_opens_the_tunnel(#[case] head: &str, #[case] expected: Tunnel) {
        let got =
            opened("p:1", head.as_bytes()).map_or(Tunnel::Stays, |()| Tunnel::Opens);
        assert_eq!(got, expected);
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Tunnel {
        Opens,
        Stays,
    }

    fn refusal(err: Option<Error>) -> Option<String> {
        match err {
            Some(Error::BoardSocket(BoardSocketError::ProxyDenied {
                detail: answer,
                ..
            })) => Some(answer),
            _ => None,
        }
    }

    fn named_proxy(err: Option<Error>) -> Option<String> {
        match err {
            Some(Error::BoardSocket(BoardSocketError::Proxy { proxy, .. })) => {
                Some(proxy)
            }
            _ => None,
        }
    }

    async fn fake_proxy(
        answer: &'static str,
        after: &'static [u8],
    ) -> std::io::Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?.to_string();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut seen = Vec::new();
            // imperative: a byte per await to the blank line
            while !seen.ends_with(b"\r\n\r\n") {
                seen.push(stream.read_u8().await?);
            }
            stream.write_all(answer.as_bytes()).await?;
            stream.write_all(after).await?;
            std::io::Result::Ok(seen)
        });
        Ok(address)
    }

    #[tokio::test]
    async fn the_tunnel_keeps_what_follows_the_answer() -> Result<()> {
        let proxy = fake_proxy("HTTP/1.1 200 OK\r\nVia: test\r\n\r\n", b"\x16\x03\x01")
            .await
            .map_err(|error| {
                Error::BoardSocket(BoardSocketError::Proxy {
                    proxy: "fake".into(),
                    error,
                })
            })?;
        let mut stream = tunnel(&proxy, "app.holst.so:443", None).await?;
        let mut first = [0u8; 3];
        stream.read_exact(&mut first).await.map_err(|error| {
            Error::BoardSocket(BoardSocketError::Proxy {
                proxy: proxy.clone(),
                error,
            })
        })?;
        assert_eq!(&first, b"\x16\x03\x01");
        Ok(())
    }

    #[tokio::test]
    async fn a_refusal_is_reported_with_its_status() -> Result<()> {
        let proxy =
            fake_proxy("HTTP/1.1 407 Proxy Authentication Required\r\n\r\n", b"")
                .await
                .map_err(|error| {
                    Error::BoardSocket(BoardSocketError::Proxy {
                        proxy: "fake".into(),
                        error,
                    })
                })?;
        let err = tunnel(&proxy, "app.holst.so:443", None).await.err();
        assert_eq!(
            refusal(err).as_deref(),
            Some("HTTP/1.1 407 Proxy Authentication Required")
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_unreachable_proxy_is_named() {
        let err = tunnel("127.0.0.1:1", "app.holst.so:443", None).await.err();
        assert_eq!(named_proxy(err).as_deref(), Some("127.0.0.1:1"));
    }
}
