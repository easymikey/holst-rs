//! Comment threads of the socket room and the read marks.

use serde_json::{Value, json};
use yrs::{Doc, Map as _, MapPrelim, Out, Transact as _, types::ToJson as _};

use crate::{
    FixtureError,
    board::{any_fields, object_id},
    fixture::unknown,
    http::{Parts, Request, Response, found, object_body, refused},
};

const STORE: &str = "commentsStore";
const CREATED: f64 = 1_793_000_000_000.5;

/// Where a thread is pinned.
#[derive(Debug, Clone, PartialEq)]
pub enum ThreadAnchor {
    /// A point of the board.
    Point(f64, f64),
    /// The object with this number on the board, counting from 1.
    Object(u32),
}

/// One message of a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessage {
    /// The author's number, as the clients hash it from a user id.
    pub author: u32,
    pub text: String,
}

/// A comment thread: its first message and the replies after it.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureThread {
    pub id: String,
    pub anchor: ThreadAnchor,
    pub color: Option<String>,
    pub resolved: bool,
    pub messages: Vec<ThreadMessage>,
    read_at: Option<String>,
}

impl FixtureThread {
    /// A thread with id `id` that the person numbered `author` started with
    /// `text`, pinned at the origin, unresolved, uncolored and unread.
    #[must_use]
    pub fn new(id: &str, author: u32, text: &str) -> Self {
        Self {
            id: id.to_owned(),
            anchor: ThreadAnchor::Point(0.0, 0.0),
            color: None,
            resolved: false,
            messages: vec![message(author, text)],
            read_at: None,
        }
    }

    /// Pins the thread at a point of the board.
    #[must_use]
    pub fn at(self, x: f64, y: f64) -> Self {
        Self {
            anchor: ThreadAnchor::Point(x, y),
            ..self
        }
    }

    /// Pins the thread to the object with this number on the board.
    #[must_use]
    pub fn on_object(self, number: u32) -> Self {
        Self {
            anchor: ThreadAnchor::Object(number),
            ..self
        }
    }

    /// Gives the thread the color `color`.
    #[must_use]
    pub fn colored(self, color: &str) -> Self {
        Self {
            color: Some(color.to_owned()),
            ..self
        }
    }

    /// Marks the thread resolved.
    #[must_use]
    pub fn resolved(self) -> Self {
        Self {
            resolved: true,
            ..self
        }
    }

    /// Adds a reply after the messages already there.
    #[must_use]
    pub fn reply(self, author: u32, text: &str) -> Self {
        Self {
            messages: self
                .messages
                .into_iter()
                .chain([message(author, text)])
                .collect(),
            ..self
        }
    }

    /// Says the signed-in user read the thread at `time` (ISO 8601).
    #[must_use]
    pub fn read_at(self, time: &str) -> Self {
        Self {
            read_at: Some(time.to_owned()),
            ..self
        }
    }
}

fn message(author: u32, text: &str) -> ThreadMessage {
    ThreadMessage {
        author,
        text: text.to_owned(),
    }
}

/// When one person read one thread.
#[derive(Debug, Clone)]
struct Mark {
    board: String,
    thread: String,
    read_at: String,
}

#[derive(Debug, Default)]
pub(crate) struct Comments {
    /// The board id and the document of its comments room.
    rooms: Vec<(String, Doc)>,
    marks: Vec<Mark>,
    /// The id of the signed-in user.
    user: String,
}

impl Comments {
    /// Fills the threads of the boards called as `filled` says; `board_id`
    /// maps a board name to its id. A read time on a thread becomes the
    /// user's mark.
    ///
    /// # Errors
    /// [`FixtureError`] when a thread names a board the fixture does not hold.
    pub(crate) fn fixture(
        filled: &[(String, FixtureThread)],
        user: &str,
        board_id: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, FixtureError> {
        let threads = filled
            .iter()
            .map(|(board, thread)| {
                board_id(board)
                    .map(|id| (id, thread.clone()))
                    .ok_or_else(|| unknown("board", board))
            })
            .collect::<Result<Vec<_>, FixtureError>>()?;
        let marks = threads
            .iter()
            .filter_map(|(board, thread)| {
                thread.read_at.as_ref().map(|read_at| Mark {
                    board: board.clone(),
                    thread: thread.id.clone(),
                    read_at: read_at.clone(),
                })
            })
            .collect();
        let rooms = threads
            .iter()
            .map(|(board, _)| board.clone())
            .fold(Vec::<String>::new(), |mut boards, board| {
                if !boards.contains(&board) {
                    boards.push(board);
                }
                boards
            })
            .into_iter()
            .map(|board| {
                let own: Vec<&FixtureThread> = threads
                    .iter()
                    .filter(|(id, _)| *id == board)
                    .map(|(_, thread)| thread)
                    .collect();
                let room = filled_room(&board, &own);
                (board, room)
            })
            .collect();
        Ok(Self {
            rooms,
            marks,
            user: user.to_owned(),
        })
    }

    /// The id of the signed-in user.
    pub(crate) fn user(&self) -> &str {
        &self.user
    }

    /// The document of the comments room of a board, empty at first.
    pub(crate) fn room(&mut self, board: &str) -> Doc {
        let held = self
            .rooms
            .iter()
            .find(|(id, _)| id == board)
            .map(|(_, doc)| doc.clone());
        held.unwrap_or_else(|| {
            let doc = Doc::new();
            self.rooms.push((board.to_owned(), doc.clone()));
            doc
        })
    }

    /// The threads of a board, by its id, oldest first.
    pub(crate) fn threads_of(&self, board: &str) -> Vec<FixtureThread> {
        self.rooms
            .iter()
            .find(|(id, _)| id == board)
            .map(|(_, doc)| read_room(doc))
            .unwrap_or_default()
    }

    /// When the signed-in user last read the thread `thread` of the board
    /// `board`.
    pub(crate) fn read_at(&self, board: &str, thread: &str) -> Option<String> {
        self.marks
            .iter()
            .find(|mark| mark.board == board && mark.thread == thread)
            .map(|mark| mark.read_at.clone())
    }

    /// Answers the read-mark endpoints of the boards `exists` knows. A mark
    /// outlives its thread, as on the real server.
    pub(crate) fn respond(
        &mut self,
        request: &Request,
        exists: impl Fn(&str) -> bool,
    ) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "comment", "thread_list", board]) => {
                Some(if exists(board) {
                    found(&Value::Array(self.listed(board)))
                } else {
                    refused(404, "not-found")
                })
            }
            ("POST", ["", "gapi", "comment", "set_read_at"]) => {
                Some(self.mark(request, exists))
            }
            _ => None,
        }
    }

    /// The marks of the signed-in user on a board, as the server lists them.
    fn listed(&self, board: &str) -> Vec<Value> {
        self.marks
            .iter()
            .filter(|mark| mark.board == board)
            .map(|mark| {
                json!({
                    "channelId": mark.board,
                    "readAt": mark.read_at,
                    "threadId": mark.thread,
                    "userId": self.user,
                })
            })
            .collect()
    }

    fn mark(&mut self, request: &Request, exists: impl Fn(&str) -> bool) -> Response {
        let body = object_body(request);
        let field = |name: &str| {
            body.as_ref()
                .and_then(|body| body.get(name))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        match (field("channelId"), field("threadId"), field("readAt")) {
            (Some(board), ..) if !exists(&board) => refused(404, "not-found"),
            (Some(board), Some(thread), Some(read_at)) => {
                self.marks
                    .retain(|mark| !(mark.board == board && mark.thread == thread));
                self.marks.push(Mark {
                    board,
                    thread,
                    read_at,
                });
                // The real server answers with an empty body.
                Response {
                    status: 200,
                    body: String::new(),
                    cookie: None,
                }
            }
            _ => refused(400, "bad-request"),
        }
    }
}

/// The room of a board holding `threads`, with the entries the web client
/// writes: a thread carries its first message, a reply points to its thread.
fn filled_room(board: &str, threads: &[&FixtureThread]) -> Doc {
    let doc = Doc::new();
    let store = doc.get_or_insert_map(STORE);
    {
        let mut txn = doc.transact_mut();
        threads.iter().zip(0_u32..).for_each(|(thread, order)| {
            let created = CREATED + f64::from(order) * 1000.0;
            let target = match &thread.anchor {
                ThreadAnchor::Point(x, y) => {
                    json!({ "type": 2, "point": { "x": x, "y": y } })
                }
                ThreadAnchor::Object(number) => json!({
                    "type": 1,
                    "targetId": object_id(board, *number),
                    "point": { "x": 0.5, "y": 0.5 },
                }),
            };
            let first = thread.messages.first();
            let mut head = entry_fields(
                &thread.id,
                first.map_or(0, |first| first.author),
                first.map_or("", |first| first.text.as_str()),
                created,
            );
            head.extend([
                ("t".to_owned(), target),
                ("r".to_owned(), json!(thread.resolved)),
                ("ro".to_owned(), json!(true)),
            ]);
            if thread.resolved {
                head.extend([
                    ("ra".to_owned(), json!(created + 500.0)),
                    (
                        "rb".to_owned(),
                        json!(first.map_or(0, |first| first.author)),
                    ),
                ]);
            }
            let entry = store.insert(
                &mut txn,
                thread.id.as_str(),
                MapPrelim::from_iter(any_fields(head)),
            );
            if let Some(color) = &thread.color {
                entry.insert(
                    &mut txn,
                    "co",
                    MapPrelim::from_iter(any_fields([
                        ("color".to_owned(), json!(color)),
                        ("opacity".to_owned(), json!(1)),
                    ])),
                );
            }
            thread
                .messages
                .iter()
                .skip(1)
                .zip(1_u32..)
                .for_each(|(reply, number)| {
                    let id = format!("{}r{number}", thread.id);
                    let mut fields = entry_fields(
                        &id,
                        reply.author,
                        &reply.text,
                        created + f64::from(number) * 10.0,
                    );
                    fields.push(("p".to_owned(), json!(thread.id)));
                    store.insert(
                        &mut txn,
                        id.as_str(),
                        MapPrelim::from_iter(any_fields(fields)),
                    );
                });
        });
    }
    doc
}

fn entry_fields(
    id: &str,
    author: u32,
    text: &str,
    created: f64,
) -> Vec<(String, Value)> {
    let paragraphs: Vec<Value> = text
        .split('\n')
        .map(|line| {
            json!({ "type": "paragraph", "children": [{ "text": line }], "key": format!("{id}-p") })
        })
        .collect();
    vec![
        (
            "c".to_owned(),
            json!([{ "type": "wrapper", "children": paragraphs, "key": format!("{id}-w") }]),
        ),
        ("m".to_owned(), json!([])),
        ("a".to_owned(), json!(author)),
        ("ca".to_owned(), json!(created)),
        ("id".to_owned(), json!(id)),
    ]
}

/// The threads of a room, oldest first, each with its replies.
fn read_room(doc: &Doc) -> Vec<FixtureThread> {
    let store = doc.get_or_insert_map(STORE);
    let txn = doc.transact();
    let mut all: Vec<Value> = store
        .iter(&txn)
        .filter_map(|(_, entry)| match entry {
            Out::YMap(_) => serde_json::to_value(entry.to_json(&txn)).ok(),
            Out::Any(_)
            | Out::YText(_)
            | Out::YArray(_)
            | Out::YXmlElement(_)
            | Out::YXmlFragment(_)
            | Out::YXmlText(_)
            | Out::YDoc(_)
            | Out::UndefinedRef(_) => None,
        })
        .collect();
    let created =
        |entry: &Value| entry.get("ca").and_then(Value::as_f64).unwrap_or_default();
    all.sort_by(|left, right| created(left).total_cmp(&created(right)));
    let text = |entry: &Value, key: &str| {
        entry.get(key).and_then(Value::as_str).map(str::to_owned)
    };
    let author = |entry: &Value| {
        entry
            .get("a")
            .and_then(Value::as_u64)
            .and_then(|author| u32::try_from(author).ok())
            .unwrap_or_default()
    };
    all.iter()
        .filter_map(|head| {
            let id = text(head, "id")?;
            let target = head.get("t")?;
            let point = target.get("point")?;
            let anchor = match target.get("type").and_then(Value::as_u64) {
                Some(1) => ThreadAnchor::Object(
                    text(target, "targetId")?
                        .rsplit('-')
                        .next()
                        .and_then(|tail| tail.parse::<u64>().ok())
                        .and_then(|ordinal| u32::try_from(ordinal % 1000).ok())?,
                ),
                _ => ThreadAnchor::Point(
                    point.get("x")?.as_f64()?,
                    point.get("y")?.as_f64()?,
                ),
            };
            let messages =
                std::iter::once(head)
                    .chain(all.iter().filter(|reply| {
                        text(reply, "p").as_deref() == Some(id.as_str())
                    }))
                    .map(|entry| ThreadMessage {
                        author: author(entry),
                        text: entry.get("c").map(plain).unwrap_or_default(),
                    })
                    .collect();
            Some(FixtureThread {
                anchor,
                color: head.get("co").and_then(|color| text(color, "color")),
                resolved: head.get("r").and_then(Value::as_bool).unwrap_or_default(),
                messages,
                read_at: None,
                id,
            })
        })
        .collect()
}

fn plain(node: &Value) -> String {
    match node {
        Value::Array(items) => items.iter().map(plain).collect::<Vec<_>>().join("\n"),
        Value::Object(map) => map.get("text").and_then(Value::as_str).map_or_else(
            || {
                let children = map
                    .get("children")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let separator =
                    if children.iter().any(|child| child.get("children").is_some()) {
                        "\n"
                    } else {
                        ""
                    };
                children
                    .iter()
                    .map(plain)
                    .collect::<Vec<_>>()
                    .join(separator)
            },
            str::to_owned,
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            String::new()
        }
    }
}
