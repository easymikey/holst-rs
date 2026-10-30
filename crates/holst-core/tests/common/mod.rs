//! A local HTTP mock for the tests and the `resolve` bench (no Holst).
#![expect(dead_code, reason = "every test crate takes its own part of the mock")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests measure and wait on real time"
)]

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use holst_core::{
    domain::session::Session,
    remote::{client::Client, http::ReqwestTransport},
};
use rstest::fixture;
use secrecy::SecretString;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream, tcp::OwnedReadHalf},
};

pub(crate) struct ReceivedRequest {
    method: String,
    pub(crate) path: String,
    authorization: String,
    cookie: String,
    n: usize,
}

type CannedResponse = (u16, String, Arc<str>);

pub(crate) const DROP: u16 = 0;

pub(crate) const CLOSE_DELIMITED: &str = "connection: close\r\n";

pub(crate) const SESSION_COOKIE: &str = "sid=1";

pub(crate) struct CannedServer {
    pub(crate) base: String,
    hits: Arc<AtomicUsize>,
}

impl CannedServer {
    pub(crate) fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub(crate) fn client(&self, token: &str) -> holst_core::Result<Client> {
        self.client_with(token, SESSION_COOKIE)
    }

    pub(crate) fn client_with(
        &self,
        token: &str,
        cookies: &str,
    ) -> holst_core::Result<Client> {
        let session = Session::with_cookies(
            SecretString::from(token),
            cookies.parse().unwrap_or_default(),
        );
        holst_core::command::login::connect(
            Some(session),
            self.base.parse()?,
            holst_core::system::clock::SystemClock,
            holst_core::system::id::RandomIds,
        )
    }

    pub(crate) fn limited_client(
        &self,
        timeout: Duration,
        body_limit: usize,
    ) -> holst_core::Result<Client> {
        let transport =
            ReqwestTransport::with_limits("holst-rs-test", timeout, body_limit)?;
        Ok(self.client("tok")?.with_transport(transport))
    }
}

pub(crate) async fn serve(
    latency: Duration,
    handler: impl Fn(ReceivedRequest) -> CannedResponse + Send + Sync + 'static,
) -> io::Result<CannedServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let hits = Arc::new(AtomicUsize::new(0));
    let (counter, handler) = (Arc::clone(&hits), Arc::new(handler));
    tokio::spawn(async move {
        // imperative: accept for the test's life; recursion would grow.
        while let Ok((stream, _)) = listener.accept().await {
            let (counter, handler) = (Arc::clone(&counter), Arc::clone(&handler));
            tokio::spawn(async move {
                // An I/O error: the client hung up; the test sees it.
                drop(answer(stream, &counter, latency, &*handler).await);
            });
        }
    });
    Ok(CannedServer { base, hits })
}

#[derive(Clone, Copy)]
enum Framing {
    Length,
    Close,
}

impl Framing {
    fn of(headers: &str) -> Self {
        headers
            .find(CLOSE_DELIMITED)
            .map_or(Framing::Length, |_| Framing::Close)
    }

    fn length_header(self, len: usize) -> String {
        match self {
            Framing::Length => format!("content-length: {len}\r\n"),
            Framing::Close => String::new(),
        }
    }

    fn closes(self) -> bool {
        matches!(self, Framing::Close)
    }
}

#[derive(Default)]
struct Head {
    length: usize,
    authorization: String,
    cookie: String,
}

impl Head {
    fn with(self, line: &str) -> io::Result<Self> {
        let (name, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => Ok(Self {
                length: value.parse().map_err(io::Error::other)?,
                ..self
            }),
            "authorization" => Ok(Self {
                authorization: value.to_owned(),
                ..self
            }),
            "cookie" => Ok(Self {
                cookie: value.to_owned(),
                ..self
            }),
            _ => Ok(self),
        }
    }
}

async fn read_head(
    read: &mut BufReader<OwnedReadHalf>,
) -> io::Result<Option<Vec<String>>> {
    let mut lines = Vec::new();
    // imperative: lines come off the socket until an empty one.
    Ok(loop {
        let mut line = String::new();
        match read.read_line(&mut line).await? {
            0 => break None,
            _ if line == "\r\n" => break Some(lines),
            _ => lines.push(line),
        }
    })
}

async fn answer(
    stream: TcpStream,
    counter: &AtomicUsize,
    latency: Duration,
    handler: &impl Fn(ReceivedRequest) -> CannedResponse,
) -> io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    // imperative: a keep-alive connection serves every request sent.
    while let Some(lines) = read_head(&mut read).await? {
        let mut words = lines
            .first()
            .map(|line| line.split(' '))
            .into_iter()
            .flatten();
        let method = words.next().unwrap_or("").to_owned();
        let path = words.next().unwrap_or("").to_owned();
        let head = lines
            .iter()
            .skip(1)
            .try_fold(Head::default(), |head, line| head.with(line))?;
        let mut request_body = vec![0; head.length];
        read.read_exact(&mut request_body).await?;
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let (status, headers, body) = handler(ReceivedRequest {
            method,
            path,
            authorization: head.authorization,
            cookie: head.cookie,
            n,
        });
        tokio::time::sleep(latency).await;
        match status {
            DROP => break,
            status => {
                let framing = Framing::of(&headers);
                let response_head = format!(
                    "HTTP/1.1 {status} X\r\n{}{headers}\r\n",
                    framing.length_header(body.len())
                );
                write.write_all(response_head.as_bytes()).await?;
                write.write_all(body.as_bytes()).await?;
                // A close-delimited body ends with the connection.
                if framing.closes() {
                    write.shutdown().await?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn script(
    script: Vec<(u16, &'static str, &'static str)>,
) -> io::Result<CannedServer> {
    scripted(Duration::ZERO, script).await
}

pub(crate) async fn scripted(
    latency: Duration,
    script: Vec<(u16, &'static str, &'static str)>,
) -> io::Result<CannedServer> {
    serve(latency, move |req| {
        script.get(req.n).or_else(|| script.last()).map_or_else(
            || (500, String::new(), "the mock script has no replies".into()),
            |&(status, headers, body)| {
                (
                    status,
                    headers.into(),
                    body.replace("{auth}", &req.authorization)
                        .replace("{cookie}", &req.cookie)
                        .into(),
                )
            },
        )
    })
    .await
}

pub(crate) async fn routes(
    routes: Vec<(&'static str, u16, String)>,
) -> io::Result<CannedServer> {
    serve(Duration::ZERO, move |req| {
        routes
            .iter()
            .find(|(path, ..)| *path == req.path)
            .map_or_else(
                || (404, String::new(), "{}".into()),
                |(_, s, b)| (*s, String::new(), b.as_str().into()),
            )
    })
    .await
}

#[fixture]
pub(crate) fn current_user() -> (&'static str, u16, String) {
    (
        "/gapi/current_user",
        200,
        serde_json::json!({ "id": "me", "workspaces": [
            { "id": "w1", "name": "Team" },
            { "id": "w2", "name": "easy" },
        ]})
        .to_string(),
    )
}
