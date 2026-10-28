//! The login page end to end: a raw client plays the browser.

#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use holst_core::{
    domain::session::Session,
    effect::clock::Clock,
    fake::{Fake, Fixture, LOGIN_CODE},
    remote::client::Client,
    storage::session_store::{LoginSaveReport, SessionStoreKind},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

use crate::{
    login::{fake::Manual, store::Backend},
    server::{McpServer, tests::table_call},
};

/// A login store in memory that signs in to the fake server.
struct Kept {
    base_url: String,
    saved: Mutex<Option<Session>>,
}

impl Kept {
    fn put(&self, session: &Session) {
        *self.saved.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(session.clone());
    }

    fn saved(&self) -> Option<Session> {
        self.saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn client(&self, session: Option<Session>) -> holst_core::Result<Client> {
        holst_core::command::login::connect(
            session,
            self.base_url.parse()?,
            holst_core::system::clock::SystemClock,
            holst_core::system::id::RandomIds,
        )
    }
}

impl Backend for Kept {
    fn load(&self) -> holst_core::Result<Option<(Session, SessionStoreKind)>> {
        Ok(self
            .saved()
            .map(|session| (session, SessionStoreKind::File)))
    }

    fn save(
        &self,
        session: &Session,
        from: SessionStoreKind,
    ) -> holst_core::Result<LoginSaveReport> {
        self.put(session);
        Ok(LoginSaveReport {
            store: from,
            stale: None,
        })
    }

    fn connect(&self, session: Session) -> holst_core::Result<Client> {
        self.client(Some(session))
    }

    fn save_new(&self, session: &Session) -> holst_core::Result<LoginSaveReport> {
        self.save(session, SessionStoreKind::File)
    }

    fn anonymous(&self) -> holst_core::Result<Client> {
        self.client(None)
    }
}

#[derive(Debug, thiserror::Error)]
enum Trouble {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error("the reply has no link")]
    NoLink,

    #[error("the reply has no status")]
    NoStatus,
}

struct Setup {
    _fake: Fake,
    holst: McpServer,
    kept: Arc<Kept>,
    clock: Arc<Manual>,
}

async fn setup() -> Result<Setup, Trouble> {
    let fake = Fake::start(Fixture::default().workspace("Team")).await?;
    let kept = Arc::new(Kept {
        base_url: fake.base_url().to_owned(),
        saved: Mutex::new(None),
    });
    let clock = Arc::new(Manual::default());
    let backend: Arc<dyn Backend> = Arc::<Kept>::clone(&kept);
    let ticks: Arc<dyn Clock> = Arc::<Manual>::clone(&clock);
    Ok(Setup {
        _fake: fake,
        holst: McpServer::with(backend, ticks),
        kept,
        clock,
    })
}

/// Where the page is: the `Host` its requests carry and the path of its token.
struct Link {
    host: String,
    path: String,
}

async fn login_link(holst: &McpServer) -> Result<(Link, String), Trouble> {
    let (failed, text) =
        table_call(holst, "auth", serde_json::json!({ "action": "login" })).await;
    assert!(!failed, "{text}");
    let link = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("http://"))
        .and_then(|rest| rest.split_once('/'))
        .map(|(host, path)| Link {
            host: host.to_owned(),
            path: format!("/{path}"),
        })
        .ok_or(Trouble::NoLink)?;
    Ok((link, text))
}

struct Reply {
    status: u16,
    head: String,
    body: String,
}

/// Sends the raw request text to the page and reads the reply to its end.
async fn send(link: &Link, request: String) -> Result<Reply, Trouble> {
    let mut stream = TcpStream::connect(&link.host).await?;
    stream.write_all(request.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(Trouble::NoStatus)?;
    Ok(Reply {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    })
}

fn get(host: &str, path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

fn post_as(link: &Link, content_type: &str, body: &str) -> String {
    format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        link.path,
        link.host,
        body.len()
    )
}

fn post(link: &Link, body: &str) -> String {
    post_as(link, "application/x-www-form-urlencoded", body)
}

async fn open_page(link: &Link) -> Result<Reply, Trouble> {
    send(link, get(&link.host, &link.path)).await
}

async fn send_email(link: &Link) -> Result<Reply, Trouble> {
    send(link, post(link, "email=me%40example.com")).await
}

#[tokio::test]
async fn the_user_logs_in_through_the_page() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (link, text) = login_link(&setup.holst).await?;
    assert!(text.contains("then call auth"), "{text}");

    let page = open_page(&link).await?;
    assert_eq!(
        (page.status, page.body.contains("type=\"email\"")),
        (200, true)
    );
    assert!(
        page.head.contains("cache-control: no-store"),
        "{}",
        page.head
    );

    let asked = send_email(&link).await?;
    assert!(
        asked.body.contains("A code was sent to me@example.com"),
        "{}",
        asked.body
    );
    assert!(setup.kept.saved().is_none());

    let done = send(&link, post(&link, &format!("secret={LOGIN_CODE}"))).await?;
    assert!(
        done.body
            .contains("Logged in as me@example.com. You can close this page."),
        "{}",
        done.body
    );
    assert!(setup.kept.saved().is_some());

    let (failed, status) = table_call(
        &setup.holst,
        "auth",
        serde_json::json!({ "action": "status" }),
    )
    .await;
    assert!(!failed, "{status}");
    assert!(status.contains("logged in:"), "{status}");
    Ok(())
}

#[tokio::test]
async fn an_open_page_gives_the_same_link_again() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (_, first) = login_link(&setup.holst).await?;
    let (_, second) = login_link(&setup.holst).await?;
    assert_eq!(first, second);
    Ok(())
}

#[tokio::test]
async fn a_path_without_the_token_is_not_found() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (link, _) = login_link(&setup.holst).await?;
    let wrong = format!("{}x", link.path);
    let statuses = [
        send(&link, get(&link.host, &wrong)).await?.status,
        send(&link, get(&link.host, "/")).await?.status,
        send(&link, get(&link.host, "/login/")).await?.status,
    ];
    assert_eq!(statuses, [404, 404, 404]);
    Ok(())
}

#[tokio::test]
async fn another_host_is_refused() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (link, _) = login_link(&setup.holst).await?;
    let rebound = send(&link, get("evil.example:80", &link.path)).await?;
    assert_eq!(rebound.status, 403);
    assert!(!rebound.body.contains("email"), "{}", rebound.body);
    Ok(())
}

#[tokio::test]
async fn an_expired_page_is_gone() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (link, _) = login_link(&setup.holst).await?;
    setup.clock.advance(Duration::from_mins(10));
    assert_eq!(open_page(&link).await?.status, 410);
    Ok(())
}

#[tokio::test]
async fn a_wrong_code_asks_again() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (link, _) = login_link(&setup.holst).await?;
    send_email(&link).await?;

    let wrong = send(&link, post(&link, "secret=000000")).await?;
    assert!(wrong.body.contains("was not accepted"), "{}", wrong.body);
    assert!(wrong.body.contains("name=\"secret\""), "{}", wrong.body);
    assert!(setup.kept.saved().is_none());

    let right = send(&link, post(&link, &format!("secret={LOGIN_CODE}"))).await?;
    assert!(right.body.contains("Logged in as"), "{}", right.body);
    Ok(())
}

#[tokio::test]
async fn a_post_must_be_a_small_form() -> Result<(), Trouble> {
    let setup = setup().await?;
    let (link, _) = login_link(&setup.holst).await?;
    let json = send(&link, post_as(&link, "application/json", "{}")).await?;
    let claim = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: 5000\r\nConnection: close\r\n\r\n",
        link.path, link.host
    );
    let big = send(&link, claim).await?;
    assert_eq!((json.status, big.status), (415, 413));
    Ok(())
}
