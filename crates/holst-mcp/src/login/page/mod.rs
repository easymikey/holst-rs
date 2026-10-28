//! The local login page: a one-shot HTTP server on 127.0.0.1.

mod flow;
mod html;
mod http;

use std::{sync::Arc, time::Duration};

use holst_core::{domain::id_source::IdSource as _, system::id::RandomIds};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify},
    task::JoinHandle,
};

use crate::{
    error::Result,
    login::{
        SharedLogin,
        machine::Milliseconds,
        page::{
            flow::Flow,
            http::{Form, Reply, Request, read_request},
        },
    },
};

/// How long a page lives, and with it the link.
const LIFETIME: Duration = Duration::from_mins(10);

/// How long one connection may take to send its request and get its reply.
const CONNECTION: Duration = Duration::from_secs(10);

/// A page that is serving.
#[derive(Debug)]
pub(crate) struct OpenPage {
    link: String,
    expires_at: Milliseconds,
    task: JoinHandle<()>,
}

impl OpenPage {
    fn is_open(&self, now: Milliseconds) -> bool {
        now < self.expires_at && !self.task.is_finished()
    }
}

/// The link of the page that is open, or of a new one.
///
/// # Errors
/// The I/O error of binding the port.
pub(crate) async fn open(login: &Arc<SharedLogin>) -> Result<String> {
    let mut slot = login.page.lock().await;
    let now = login.now();
    if let Some(page) = slot.as_ref().filter(|page| page.is_open(now)) {
        Ok(page.link.clone())
    } else {
        let started = start(login).await?;
        let link = started.link.clone();
        *slot = Some(started);
        Ok(link)
    }
}

async fn start(login: &Arc<SharedLogin>) -> Result<OpenPage> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let host = listener.local_addr()?.to_string();
    // 244 random bits, hex: safe in a path.
    let token = [RandomIds.new_id(), RandomIds.new_id()]
        .iter()
        .map(|id| id.replace('-', ""))
        .collect::<String>();
    let expires_at = login.now().saturating_add(millis(LIFETIME));
    let link = format!("http://{host}/login/{token}");
    let page = Arc::new(Page {
        path: format!("/login/{token}"),
        host,
        expires_at,
        flow: Mutex::new(Flow::new()),
        stop: Notify::new(),
        login: Arc::clone(login),
    });
    Ok(OpenPage {
        link,
        expires_at,
        task: tokio::spawn(serve(listener, page)),
    })
}

fn millis(span: Duration) -> Milliseconds {
    Milliseconds::try_from(span.as_millis()).unwrap_or(Milliseconds::MAX)
}

struct Page {
    /// The only path that answers.
    path: String,
    /// The `Host` every request carries: the address of the listener.
    host: String,
    expires_at: Milliseconds,
    flow: Mutex<Flow>,
    stop: Notify,
    login: Arc<SharedLogin>,
}

/// Accepts until the login succeeds, the page expires or its time is up.
async fn serve(listener: TcpListener, page: Arc<Page>) {
    let accepting = async {
        // imperative: one connection after another until the page is done
        loop {
            tokio::select! {
                () = page.stop.notified() => break,
                accepted = listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        drop(tokio::spawn(handle(stream, Arc::clone(&page))));
                    }
                }
            }
        }
    };
    drop(tokio::time::timeout(LIFETIME, accepting).await);
}

async fn handle(mut stream: TcpStream, page: Arc<Page>) {
    let exchange = async {
        let reply = match read_request(&mut stream).await {
            Ok(request) => Some(page.route(request).await),
            Err(refusal) => refusal.reply(),
        };
        match reply {
            Some(reply) => reply.write(&mut stream).await,
            None => Ok(()),
        }
    };
    drop(tokio::time::timeout(CONNECTION, exchange).await);
}

impl Page {
    /// Answers a request: the host, then the path, then the time, then the
    /// page.
    async fn route(&self, request: Request) -> Reply {
        let expired = self.login.now() >= self.expires_at;
        let host_is_ours = request
            .host
            .as_deref()
            .is_some_and(|host| host.eq_ignore_ascii_case(&self.host));
        match (host_is_ours, same(&request.path, &self.path), expired) {
            (false, _, _) => Reply::text(403, "Forbidden", "wrong host"),
            (true, false, _) => Reply::text(404, "Not Found", "not found"),
            (true, true, true) => {
                self.stop.notify_one();
                Reply::gone(html::gone())
            }
            (true, true, false) => self.answer(&request).await,
        }
    }

    async fn answer(&self, request: &Request) -> Reply {
        match request.method.as_str() {
            "GET" => Reply::html(html::page(&self.flow.lock().await.shown(None))),
            "POST" => self.submit(request).await,
            _ => Reply::text(405, "Method Not Allowed", "method not allowed"),
        }
    }

    async fn submit(&self, request: &Request) -> Reply {
        let media = request
            .content_type
            .as_deref()
            .and_then(|kind| kind.split(';').next())
            .map(str::trim);
        match media {
            Some(kind)
                if kind.eq_ignore_ascii_case("application/x-www-form-urlencoded") =>
            {
                let form = Form::parse(&request.body);
                let shown = self.flow.lock().await.submit(&form, &self.login).await;
                if matches!(shown, flow::Shown::LoggedIn { .. }) {
                    self.stop.notify_one();
                }
                Reply::html(html::page(&shown))
            }
            Some(_) | None => Reply::text(415, "Unsupported Media Type", "send a form"),
        }
    }
}

/// Equal text, compared without stopping at the first difference.
fn same(given: &str, wanted: &str) -> bool {
    given.len() == wanted.len()
        && given
            .bytes()
            .zip(wanted.bytes())
            .fold(0_u8, |differ, (left, right)| differ | (left ^ right))
            == 0
}
