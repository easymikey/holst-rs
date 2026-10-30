//! Reads one HTTP request and writes one reply.

use std::io;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::TcpStream,
};

pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) cookie: String,
    pub(crate) body: String,
    /// The client asked for the connection to end after this reply.
    pub(crate) closes: bool,
    /// The body as it came, byte for byte.
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) body: String,
    /// A `set-cookie` value, when the reply starts a session.
    pub(crate) cookie: Option<String>,
}

/// Reads the request line, the headers and the body the length header names.
pub(crate) async fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let first = head_line(&mut reader).await?;
    // A hang-up between two requests of a kept-alive connection reads as an
    // empty line.
    if first.is_empty() {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    // imperative: the head is read line by line up to the blank line
    let mut head = Vec::new();
    // imperative: each line waits for the socket
    while let Some(line) =
        Some(head_line(&mut reader).await?).filter(|line| !line.is_empty())
    {
        head.push(line);
    }
    let words: Vec<&str> = first.split(' ').collect();
    let word = |position: usize| words.get(position).copied().unwrap_or_default();
    let header = |wanted: &str| {
        head.iter()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.trim().to_owned())
            .unwrap_or_default()
    };
    let mut body = vec![0; header("content-length").parse().unwrap_or(0)];
    reader.read_exact(&mut body).await?;
    Ok(Request {
        method: word(0).to_owned(),
        path: word(1).to_owned(),
        cookie: header("cookie"),
        closes: header("connection").eq_ignore_ascii_case("close"),
        body: String::from_utf8_lossy(&body).into_owned(),
        bytes: body,
    })
}

async fn head_line(reader: &mut BufReader<&mut TcpStream>) -> io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    Ok(line.trim_end().to_owned())
}

pub(crate) async fn write_response(
    stream: &mut TcpStream,
    reply: &Response,
) -> io::Result<()> {
    let cookie = reply
        .cookie
        .as_ref()
        .map(|cookie| format!("set-cookie: {cookie}\r\n"))
        .unwrap_or_default();
    write_body(
        stream,
        (reply.status, "application/json"),
        &cookie,
        reply.body.as_bytes(),
    )
    .await
}

/// Writes a reply of `status` and the content type whose body is not text.
pub(crate) async fn write_body(
    stream: &mut TcpStream,
    (status, content_type): (u16, &str),
    cookie: &str,
    body: &[u8],
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n{cookie}\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await
}

pub(crate) fn found(body: &Value) -> Response {
    Response {
        status: 200,
        body: body.to_string(),
        cookie: None,
    }
}

pub(crate) fn refused(status: u16, code: &str) -> Response {
    Response {
        status,
        body: json!({ "errCode": code }).to_string(),
        cookie: None,
    }
}

/// A request split into its method, path segments and query pairs.
pub(crate) struct Parts<'a> {
    pub(crate) method: &'a str,
    pub(crate) segments: Vec<&'a str>,
    pub(crate) query: Vec<(String, String)>,
}

impl<'a> Parts<'a> {
    pub(crate) fn of(request: &'a Request) -> Self {
        let (path, query) = request
            .path
            .split_once('?')
            .unwrap_or((request.path.as_str(), ""));
        Self {
            method: request.method.as_str(),
            segments: path.split('/').collect(),
            query: query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(key, value)| (decoded(key), decoded(value)))
                .collect(),
        }
    }

    pub(crate) fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads the `%XX` escapes of a query value.
fn decoded(text: &str) -> String {
    String::from_utf8_lossy(&unescaped(text.as_bytes())).into_owned()
}

fn unescaped(bytes: &[u8]) -> Vec<u8> {
    match bytes {
        [] => Vec::new(),
        [b'%', high, low, rest @ ..] if hex(*high, *low).is_some() => {
            [hex(*high, *low).into_iter().collect(), unescaped(rest)].concat()
        }
        [other, rest @ ..] => [vec![*other], unescaped(rest)].concat(),
    }
}

fn hex(high: u8, low: u8) -> Option<u8> {
    let digit = |byte: u8| char::from(byte).to_digit(16);
    digit(high)
        .zip(digit(low))
        .and_then(|(high, low)| u8::try_from(high * 16 + low).ok())
}

pub(crate) fn object_body(request: &Request) -> Option<serde_json::Map<String, Value>> {
    match serde_json::from_str::<Value>(&request.body) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) | Err(_) => None,
    }
}
