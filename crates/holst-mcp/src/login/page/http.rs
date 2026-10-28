//! A minimal HTTP/1.1 exchange for the login page: one request, one reply.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// The largest head (request line and headers) the page reads.
const HEAD_MAX: usize = 8 * 1024;

/// The largest body the page reads.
const BODY_MAX: usize = 4 * 1024;

/// Why a request is not read: the status it is answered with, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The peer went away or sent nothing readable: no reply.
    Closed,
    Malformed,
    HeadTooLarge,
    BodyTooLarge,
}

impl Refusal {
    pub(super) fn reply(self) -> Option<Reply> {
        match self {
            Refusal::Closed => None,
            Refusal::Malformed => Some(Reply::text(400, "Bad Request", "bad request")),
            Refusal::HeadTooLarge => Some(Reply::text(
                431,
                "Request Header Fields Too Large",
                "request too large",
            )),
            Refusal::BodyTooLarge => {
                Some(Reply::text(413, "Content Too Large", "request too large"))
            }
        }
    }
}

/// What the page needs of a request.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) method: String,
    /// The target without its query.
    pub(super) path: String,
    pub(super) host: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) body: Vec<u8>,
}

/// The request line and the headers the page looks at.
#[derive(Debug, PartialEq, Eq)]
struct Head {
    method: String,
    path: String,
    host: Option<String>,
    content_type: Option<String>,
    length: usize,
}

impl Head {
    fn parse(text: &str) -> Result<Self, Refusal> {
        let mut lines = text.split("\r\n");
        let first = lines.next().unwrap_or_default();
        let (method, target) = match first.split(' ').collect::<Vec<_>>().as_slice() {
            [method, target, version]
                if version.starts_with("HTTP/1.") && !method.is_empty() =>
            {
                Ok((*method, *target))
            }
            _ => Err(Refusal::Malformed),
        }?;
        let headers: Vec<(String, &str)> = lines
            .map(|line| {
                line.split_once(':')
                    .map(|(name, value)| {
                        (name.trim().to_ascii_lowercase(), value.trim())
                    })
                    .ok_or(Refusal::Malformed)
            })
            .collect::<Result<_, _>>()?;
        let once = |wanted: &str| {
            let mut found = headers.iter().filter(|(name, _)| name == wanted);
            match (found.next(), found.next()) {
                (_, Some(_)) => Err(Refusal::Malformed),
                (value, None) => Ok(value.map(|(_, value)| (*value).to_owned())),
            }
        };
        // A body in chunks is not read: its length is unknown.
        let chunked = headers.iter().any(|(name, _)| name == "transfer-encoding");
        let length = match (chunked, once("content-length")?) {
            (true, _) => Err(Refusal::Malformed),
            (false, None) => Ok(0),
            (false, Some(digits)) => {
                digits.parse::<usize>().map_err(|_| Refusal::Malformed)
            }
        }?;
        Ok(Self {
            method: method.to_owned(),
            path: target
                .split_once('?')
                .map_or(target, |(path, _)| path)
                .to_owned(),
            host: once("host")?,
            content_type: once("content-type")?,
            length,
        })
    }
}

/// Reads the head and the body its length names; nothing is read past it.
pub(super) async fn read_request(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Request, Refusal> {
    let mut buffer = Vec::new();
    // imperative: the head is read in chunks up to the blank line
    let found = loop {
        match head_end(&buffer) {
            Some(end) => break Ok(end),
            None if buffer.len() > HEAD_MAX => break Err(Refusal::HeadTooLarge),
            None => fill(stream, &mut buffer).await?,
        }
    };
    let end = found?;
    let (raw, rest) = buffer.split_at(end);
    let text = std::str::from_utf8(raw).map_err(|_| Refusal::Malformed)?;
    let head = Head::parse(text)?;
    let mut body = rest.get(4..).unwrap_or_default().to_vec();
    match head.length {
        length if length > BODY_MAX => Err(Refusal::BodyTooLarge),
        length => {
            // imperative: the body is read in chunks up to its length
            while body.len() < length {
                fill(stream, &mut body).await?;
            }
            body.truncate(length);
            Ok(Request {
                method: head.method,
                path: head.path,
                host: head.host,
                content_type: head.content_type,
                body,
            })
        }
    }
}

fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn fill(
    stream: &mut (impl AsyncRead + Unpin),
    buffer: &mut Vec<u8>,
) -> Result<(), Refusal> {
    let mut chunk = [0_u8; 1024];
    let read = stream.read(&mut chunk).await.map_err(|_| Refusal::Closed)?;
    match chunk.get(..read) {
        Some(bytes) if !bytes.is_empty() => {
            buffer.extend_from_slice(bytes);
            Ok(())
        }
        Some(_) | None => Err(Refusal::Closed),
    }
}

/// A reply; the browser is told never to keep it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Reply {
    pub(super) status: u16,
    reason: &'static str,
    content_type: &'static str,
    pub(super) body: String,
}

impl Reply {
    pub(super) fn html(body: String) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: "text/html; charset=utf-8",
            body,
        }
    }

    pub(super) fn gone(body: String) -> Self {
        Self {
            status: 410,
            reason: "Gone",
            content_type: "text/html; charset=utf-8",
            body,
        }
    }

    pub(super) fn text(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: body.to_owned(),
        }
    }

    pub(super) async fn write(
        &self,
        stream: &mut (impl AsyncWrite + Unpin),
    ) -> std::io::Result<()> {
        let head = format!(
            "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\n\
             cache-control: no-store\r\nreferrer-policy: no-referrer\r\n\
             x-content-type-options: nosniff\r\n\
             content-security-policy: default-src 'none'; style-src 'unsafe-inline'; \
             form-action 'self'; frame-ancestors 'none'\r\n\
             connection: close\r\n\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(self.body.as_bytes()).await?;
        stream.flush().await
    }
}

/// The fields of a `application/x-www-form-urlencoded` body.
pub(super) struct Form(Vec<(String, String)>);

impl Form {
    pub(super) fn parse(body: &[u8]) -> Self {
        Self(
            String::from_utf8_lossy(body)
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(name, value)| (unescaped(name), unescaped(value)))
                .collect(),
        )
    }

    pub(super) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads `+` as a space and the `%XX` escapes.
fn unescaped(text: &str) -> String {
    let spaced = text.replace('+', " ");
    let mut parts = spaced.split('%');
    let first = parts.next().unwrap_or_default().as_bytes().to_vec();
    let bytes = parts.fold(first, |mut bytes, part| {
        let escaped = part
            .get(..2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        if let Some(byte) = escaped {
            bytes.push(byte);
            bytes.extend_from_slice(part.get(2..).unwrap_or_default().as_bytes());
        } else {
            bytes.push(b'%');
            bytes.extend_from_slice(part.as_bytes());
        }
        bytes
    });
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)] mod tests;
