//! Comment threads of a board, read from and written to its `commentsStore`.

use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use yrs::{
    Any,
    ArrayPrelim,
    Doc,
    In,
    Map as _,
    MapPrelim,
    MapRef,
    Out,
    ReadTxn,
    Transact as _,
    TransactionMut,
    types::ToJson as _,
};

use crate::{
    domain::{
        board::{any, json, slate_text},
        change::build::{author, author_number},
        id_source::IdSource,
    },
    error::{
        Error,
        Result,
        lookup::{LookupError, LookupKind, LookupMiss},
    },
    protocol::document::{COMMENTS_STORE, USERS_IDS},
};

/// Where a thread is pinned.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadAnchor {
    /// A point of the board, in board units.
    Point { x: f64, y: f64 },
    /// A point of the object with this id, as fractions of its size.
    Object { id: String, x: f64, y: f64 },
    /// A rectangle of the board, as the web client pins a thread.
    Area {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
}

/// One message of a thread: the first one, or a reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadMessage {
    /// The short id of its entry in the store.
    pub id: String,
    /// The author's number, as [`author_number`] makes it from a user id.
    pub author: u32,
    /// Unix milliseconds, fractional as the web client writes them.
    pub created_at: f64,
    pub text: String,
}

/// A comment thread with its replies, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Thread {
    /// The short id the client makes, e.g. `B9ckv`.
    pub id: String,
    pub anchor: ThreadAnchor,
    /// A palette color; none until someone sets one.
    pub color: Option<String>,
    pub resolved: bool,
    /// The first message, then the replies.
    pub messages: Vec<ThreadMessage>,
}

/// A message to write.
#[derive(Debug, Clone, PartialEq)]
pub struct NewThreadMessage {
    pub id: String,
    /// The author's user id; [`build`] registers its number in the room.
    pub author: String,
    pub created_at: f64,
    pub text: String,
}

/// One change of the store, as the web client makes it.
#[derive(Debug, Clone, PartialEq)]
pub enum CommentChange {
    Start {
        message: NewThreadMessage,
        anchor: ThreadAnchor,
    },
    Reply {
        thread: String,
        message: NewThreadMessage,
    },
    /// A palette color, or none to clear it.
    Color {
        thread: String,
        color: Option<String>,
    },
    Resolve {
        thread: String,
        /// The resolver's user id.
        by: String,
        at: f64,
    },
    Reopen {
        thread: String,
    },
    /// The thread and its replies.
    Delete {
        thread: String,
    },
}

const ID: &str = "id";
const TARGET: &str = "t";
const PARENT: &str = "p";
const BODY: &str = "c";
const AUTHOR: &str = "a";
const CREATED: &str = "ca";
const RESOLVED: &str = "r";
const RESOLVED_AT: &str = "ra";
const RESOLVED_BY: &str = "rb";
const COLOR: &str = "co";
const MARKS: &str = "color";
const TARGET_OBJECT: u64 = 1;
const TARGET_POINT: u64 = 2;
const TARGET_RECT: u64 = 3;
const MENTIONS: &str = "m";
const ID_ALPHABET: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/// A five-character id from `seed`, drawn again until `taken` does not know it.
///
/// ```
/// use holst_core::domain::board::comment::short_id;
///
/// let id = short_id("seed", |_| false);
/// assert_eq!(id.chars().count(), 5);
/// assert_ne!(short_id("seed", |known| known == id), id);
/// ```
#[must_use]
pub fn short_id(seed: &str, taken: impl Fn(&str) -> bool) -> String {
    (0..u32::MAX)
        .map(|salt| {
            let number = author_number(seed, salt);
            (0..5_u32)
                .filter_map(|place| {
                    let at = usize::try_from((number >> (place * 6)) & 0x3f).ok()?;
                    ID_ALPHABET.chars().nth(at)
                })
                .collect::<String>()
        })
        .find(|id| !taken(id))
        .unwrap_or_default()
}

/// Every thread of the store, oldest first (equal times by id); replies are
/// grouped under their thread, and an entry that is neither a thread nor a
/// reply of one is left out.
#[must_use]
pub fn threads(doc: &Doc) -> Vec<Thread> {
    let all = entries(doc);
    all.iter()
        .filter_map(|entry| thread_of(entry, &all))
        .sorted_by(|left, right| {
            let created = |thread: &Thread| {
                thread
                    .messages
                    .first()
                    .map_or(0.0, |message| message.created_at)
            };
            created(left)
                .total_cmp(&created(right))
                .then_with(|| left.id.cmp(&right.id))
        })
        .collect()
}

fn entries(doc: &Doc) -> Vec<Value> {
    let store = doc.get_or_insert_map(COMMENTS_STORE);
    let txn = doc.transact();
    store
        .iter(&txn)
        .filter_map(|(_, entry)| json(&entry.to_json(&txn)))
        .collect()
}

fn text(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn thread_of(head: &Value, all: &[Value]) -> Option<Thread> {
    let id = text(head, ID)?;
    let anchor = anchor_of(head.get(TARGET)?)?;
    let first = message_of(head)?;
    let replies = all
        .iter()
        .filter(|entry| text(entry, PARENT).as_deref() == Some(id.as_str()))
        .filter_map(message_of)
        .sorted_by(|left, right| left.created_at.total_cmp(&right.created_at));
    Some(Thread {
        id,
        anchor,
        color: head
            .get(COLOR)
            .and_then(|color| text(color, MARKS))
            .filter(|color| !color.is_empty()),
        resolved: head
            .get(RESOLVED)
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        messages: std::iter::once(first).chain(replies).collect(),
    })
}

fn message_of(entry: &Value) -> Option<ThreadMessage> {
    Some(ThreadMessage {
        id: text(entry, ID)?,
        author: entry
            .get(AUTHOR)
            .and_then(Value::as_u64)
            .and_then(|author| u32::try_from(author).ok())
            .unwrap_or_default(),
        created_at: entry
            .get(CREATED)
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        text: entry.get(BODY).map(slate_text).unwrap_or_default(),
    })
}

fn coordinates(point: &Value) -> Option<(f64, f64)> {
    Some((point.get("x")?.as_f64()?, point.get("y")?.as_f64()?))
}

fn anchor_of(target: &Value) -> Option<ThreadAnchor> {
    match target.get("type")?.as_u64()? {
        TARGET_OBJECT => {
            let (x, y) = coordinates(target.get("point")?)?;
            Some(ThreadAnchor::Object {
                id: text(target, "targetId")?,
                x,
                y,
            })
        }
        TARGET_POINT => {
            let (x, y) = coordinates(target.get("point")?)?;
            Some(ThreadAnchor::Point { x, y })
        }
        TARGET_RECT => {
            let (start_x, start_y) = coordinates(target.get("startPoint")?)?;
            let (end_x, end_y) = coordinates(target.get("endPoint")?)?;
            Some(ThreadAnchor::Area {
                x: start_x.min(end_x),
                y: start_y.min(end_y),
                width: (end_x - start_x).abs(),
                height: (end_y - start_y).abs(),
            })
        }
        _ => None,
    }
}

/// Applies `change` to the store of `doc` in one transaction and encodes just
/// that transaction as a v2 update. `ids` numbers the blocks of a new text.
/// The number of an author or resolver is taken from the room's `usersIds`,
/// and registered there when the user has none.
///
/// # Errors
/// [`LookupError`] of a thread the store does not hold.
pub fn build(doc: &Doc, change: &CommentChange, ids: &dyn IdSource) -> Result<Vec<u8>> {
    let store = doc.get_or_insert_map(COMMENTS_STORE);
    let users = doc.get_or_insert_map(USERS_IDS);
    let mut txn = doc.transact_mut();
    write(&store, &users, &mut txn, change, ids)?;
    txn.commit();
    Ok(txn.encode_update_v2())
}

fn write(
    store: &MapRef,
    users: &MapRef,
    txn: &mut TransactionMut<'_>,
    change: &CommentChange,
    ids: &dyn IdSource,
) -> Result<()> {
    match change {
        CommentChange::Start { message, anchor } => {
            let number = author(txn, users, &message.author)?;
            let mut fields = message_fields(message, number, ids);
            fields.extend([
                (TARGET.to_owned(), target(anchor)),
                (RESOLVED.to_owned(), json!(false)),
                // Every new thread has it set; nothing reads it.
                ("ro".to_owned(), json!(true)),
            ]);
            put(store, txn, &message.id, &fields)
        }
        CommentChange::Reply { thread, message } => {
            let entry = thread_map(store, txn, thread)?;
            // The web client opens a resolved thread again before it replies.
            if let Some(Out::Any(Any::Bool(true))) = entry.get(&*txn, RESOLVED) {
                reopen(&entry, txn)?;
            }
            let number = author(txn, users, &message.author)?;
            let mut fields = message_fields(message, number, ids);
            fields.push((PARENT.to_owned(), json!(thread)));
            put(store, txn, &message.id, &fields)
        }
        CommentChange::Color { thread, color } => {
            let entry = thread_map(store, txn, thread)?;
            match (color, entry.get(&*txn, COLOR)) {
                (None, _) => {
                    entry.remove(txn, COLOR);
                    Ok(())
                }
                (Some(color), Some(Out::YMap(held))) => {
                    held.insert(txn, MARKS, any(&json!(color))?);
                    Ok(())
                }
                (Some(color), _) => {
                    let fields = [(MARKS, json!(color)), ("opacity", json!(1))]
                        .into_iter()
                        .map(|(name, value)| Ok((name.to_owned(), any(&value)?)))
                        .collect::<Result<Vec<_>>>()?;
                    entry.insert(txn, COLOR, MapPrelim::from_iter(fields));
                    Ok(())
                }
            }
        }
        CommentChange::Resolve { thread, by, at } => {
            let entry = thread_map(store, txn, thread)?;
            let resolver = author(txn, users, by)?;
            [
                (RESOLVED, json!(true)),
                (RESOLVED_AT, json!(at)),
                (RESOLVED_BY, json!(resolver)),
            ]
            .iter()
            .try_for_each(|(name, value)| {
                entry.insert(txn, *name, any(value)?);
                Ok(())
            })
        }
        CommentChange::Reopen { thread } => {
            reopen(&thread_map(store, txn, thread)?, txn)
        }
        CommentChange::Delete { thread } => {
            thread_map(store, txn, thread)?;
            let doomed: Vec<String> = store
                .iter(&*txn)
                .filter_map(|(key, entry)| {
                    entry
                        .cast::<MapRef>()
                        .ok()
                        .filter(|map| parent_of(&*txn, map).as_deref() == Some(thread))
                        .map(|_| key.to_owned())
                })
                .chain([thread.clone()])
                .collect();
            // imperative: each removal needs the transaction
            for key in &doomed {
                store.remove(txn, key);
            }
            Ok(())
        }
    }
}

fn reopen(entry: &MapRef, txn: &mut TransactionMut<'_>) -> Result<()> {
    entry.insert(txn, RESOLVED, any(&json!(false))?);
    entry.remove(txn, RESOLVED_AT);
    entry.remove(txn, RESOLVED_BY);
    Ok(())
}

fn message_fields(
    message: &NewThreadMessage,
    number: u32,
    ids: &dyn IdSource,
) -> Vec<(String, Value)> {
    let wrappers: Vec<Value> = message
        .text
        .split('\n')
        .map(|line| {
            json!({
                "type": "wrapper",
                "children": [{
                    "type": "paragraph",
                    "children": [{ "text": line }],
                    "key": ids.new_id(),
                }],
                "key": ids.new_id(),
            })
        })
        .collect();
    [
        (BODY, Value::Array(wrappers)),
        (AUTHOR, json!(number)),
        (CREATED, json!(message.created_at)),
        (ID, json!(message.id)),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

fn target(anchor: &ThreadAnchor) -> Value {
    match anchor {
        ThreadAnchor::Point { x, y } => {
            json!({ "type": TARGET_POINT, "point": { "x": x, "y": y } })
        }
        ThreadAnchor::Object { id, x, y } => json!({
            "type": TARGET_OBJECT,
            "targetId": id,
            "point": { "x": x, "y": y },
        }),
        ThreadAnchor::Area {
            x,
            y,
            width,
            height,
        } => json!({
            "type": TARGET_RECT,
            "startPoint": { "x": x, "y": y },
            "endPoint": { "x": x + width, "y": y + height },
        }),
    }
}

/// Inserts a message entry. Its mentions are a `Y.Array`, empty: the web
/// client's side stores keep every array of primitives as one.
fn put(
    store: &MapRef,
    txn: &mut TransactionMut<'_>,
    id: &str,
    fields: &[(String, Value)],
) -> Result<()> {
    let mut entry = fields
        .iter()
        .map(|(name, value)| Ok((name.clone(), In::from(any(value)?))))
        .collect::<Result<Vec<(String, In)>>>()?;
    entry.push((MENTIONS.to_owned(), In::from(ArrayPrelim::default())));
    store.insert(txn, id, MapPrelim::from_iter(entry));
    Ok(())
}

fn thread_map<T: ReadTxn>(store: &MapRef, txn: &T, id: &str) -> Result<MapRef> {
    match store.get(txn, id) {
        Some(Out::YMap(map)) if map.contains_key(txn, TARGET) => Ok(map),
        _ => Err(Error::Lookup(LookupError {
            kind: LookupKind::Thread,
            reference: id.to_owned(),
            miss: LookupMiss::not_found(),
        })),
    }
}

fn parent_of<T: ReadTxn>(txn: &T, entry: &MapRef) -> Option<String> {
    match entry.get(txn, PARENT) {
        Some(Out::Any(Any::String(parent))) => Some(parent.to_string()),
        _ => None,
    }
}

/// When one person read one thread, as `comment/thread_list` lists it. A mark
/// may point at a thread deleted since.
///
/// ```
/// use holst_core::domain::board::comment::ReadMark;
/// use serde_json::json;
///
/// let reply = json!([{
///     "channelId": "b1",
///     "readAt": "2026-10-27T12:27:28.401Z",
///     "threadId": "ZTaaJ",
///     "userId": "u1",
/// }]);
/// let marks: Vec<ReadMark> = serde_json::from_value(reply)?;
/// assert_eq!(marks[0].thread_id, "ZTaaJ");
///
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
pub struct ReadMark {
    pub thread_id: String,
    pub user_id: String,
    /// ISO 8601 with milliseconds, e.g. `2026-10-27T12:27:28.401Z`.
    pub read_at: String,
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{
        Array as _,
        ArrayRef,
        Doc,
        Map as _,
        MapRef,
        ReadTxn as _,
        StateVector,
        Transact as _,
        Update,
        types::ToJson as _,
        updates::decoder::Decode as _,
    };

    use crate::{
        Result,
        domain::{
            board::{
                any,
                comment::{
                    CommentChange,
                    NewThreadMessage,
                    ReadMark,
                    Thread,
                    ThreadAnchor,
                    ThreadMessage,
                    build,
                    threads,
                },
            },
            change::build::author_number,
            id_source::SequentialIds,
        },
        error::{Error, lookup::LookupKind},
    };

    const USER: &str = "00000000-0000-4000-8000-0000000000a1";

    fn message(id: &str, at: f64, text: &str) -> NewThreadMessage {
        NewThreadMessage {
            id: id.to_owned(),
            author: USER.to_owned(),
            created_at: at,
            text: text.to_owned(),
        }
    }

    fn start(doc: &Doc, id: &str, anchor: ThreadAnchor) -> Result<Vec<u8>> {
        build(
            doc,
            &CommentChange::Start {
                message: message(id, 100.5, "zz first"),
                anchor,
            },
            &SequentialIds::new("k"),
        )
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the call sites build the change in place"
    )]
    fn change(doc: &Doc, change: CommentChange) -> Result<Vec<u8>> {
        build(doc, &change, &SequentialIds::new("k"))
    }

    fn point() -> ThreadAnchor {
        ThreadAnchor::Point {
            x: 169.5,
            y: 698.25,
        }
    }

    #[test]
    fn marks_decode_and_drop_the_channel() {
        let reply = json!([
            { "channelId": "b1", "readAt": "2026-10-27T12:27:28.401Z", "threadId": "ZTaaJ", "userId": "u1" },
            { "channelId": "b1", "readAt": "2026-10-27T12:28:00.000Z", "threadId": "9YSRw", "userId": "u2" },
        ]);
        let marks = serde_json::from_value::<Vec<ReadMark>>(reply)
            .ok()
            .map(|marks| {
                marks
                    .into_iter()
                    .map(|mark| (mark.thread_id, mark.user_id, mark.read_at))
                    .collect::<Vec<_>>()
            });
        assert_eq!(
            marks,
            Some(vec![
                (
                    "ZTaaJ".to_owned(),
                    "u1".to_owned(),
                    "2026-10-27T12:27:28.401Z".to_owned()
                ),
                (
                    "9YSRw".to_owned(),
                    "u2".to_owned(),
                    "2026-10-27T12:28:00.000Z".to_owned()
                ),
            ])
        );
    }

    #[test]
    fn a_mark_without_a_time_is_refused() {
        let reply = json!([{ "channelId": "b1", "threadId": "t", "userId": "u" }]);
        assert!(serde_json::from_value::<Vec<ReadMark>>(reply).is_err());
    }

    #[test]
    fn a_thread_serializes_with_its_anchor() {
        let thread = Thread {
            id: "ZTaaJ".to_owned(),
            anchor: ThreadAnchor::Point { x: 10.0, y: 20.0 },
            color: None,
            resolved: false,
            messages: vec![ThreadMessage {
                id: "ZTaaJ".to_owned(),
                author: 7,
                created_at: 1.5,
                text: "Why?".to_owned(),
            }],
        };
        let object = Thread {
            anchor: ThreadAnchor::Object {
                id: "o1".to_owned(),
                x: 0.5,
                y: 0.25,
            },
            ..thread.clone()
        };
        assert_eq!(
            serde_json::to_value(thread).ok(),
            Some(json!({
                "id": "ZTaaJ",
                "anchor": { "point": { "x": 10.0, "y": 20.0 } },
                "color": null,
                "resolved": false,
                "messages": [
                    { "id": "ZTaaJ", "author": 7, "created_at": 1.5, "text": "Why?" }
                ],
            }))
        );
        assert_eq!(
            serde_json::to_value(object)
                .ok()
                .and_then(|value| value.get("anchor").cloned()),
            Some(json!({ "object": { "id": "o1", "x": 0.5, "y": 0.25 } }))
        );
    }

    #[test]
    fn a_thread_and_its_replies_read_back_in_order() -> Result<()> {
        let doc = Doc::new();
        start(&doc, "B9ckv", point())?;
        change(
            &doc,
            CommentChange::Reply {
                thread: "B9ckv".to_owned(),
                message: message("3fBIc", 300.0, "zz reply\ntwo lines"),
            },
        )?;
        change(
            &doc,
            CommentChange::Reply {
                thread: "B9ckv".to_owned(),
                message: message("a1b2c", 200.0, "zz earlier"),
            },
        )?;
        let found = threads(&doc);
        assert_eq!(found.len(), 1);
        let thread =
            found
                .first()
                .ok_or(Error::Lookup(crate::error::lookup::LookupError {
                    kind: LookupKind::Thread,
                    reference: "B9ckv".to_owned(),
                    miss: crate::error::lookup::LookupMiss::not_found(),
                }))?;
        assert_eq!(thread.anchor, point());
        assert_eq!(
            thread
                .messages
                .iter()
                .map(|message| (message.id.as_str(), message.text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("B9ckv", "zz first"),
                ("a1b2c", "zz earlier"),
                ("3fBIc", "zz reply\ntwo lines"),
            ]
        );
        assert_eq!(
            threads(&doc)
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            ["B9ckv"]
        );
        Ok(())
    }

    #[test]
    fn threads_with_equal_times_sort_the_same_way() -> Result<()> {
        let ids = ["Zz9aB", "Aa1bC", "Mm5dE", "Qq3fG", "Bb7hI", "Kk2jL"];
        let order_of = |inserted: Vec<&str>| -> Result<Vec<String>> {
            let doc = Doc::new();
            inserted
                .into_iter()
                .try_for_each(|id| start(&doc, id, point()).map(drop))?;
            Ok(threads(&doc).into_iter().map(|thread| thread.id).collect())
        };
        let forward = order_of(ids.to_vec())?;
        let backward = order_of(ids.iter().rev().copied().collect())?;
        assert_eq!(forward, backward);
        assert_eq!(
            forward,
            ["Aa1bC", "Bb7hI", "Kk2jL", "Mm5dE", "Qq3fG", "Zz9aB"]
        );
        Ok(())
    }

    #[test]
    fn colour_and_resolution_come_and_go() -> Result<()> {
        let doc = Doc::new();
        start(&doc, "B9ckv", point())?;
        let colored = |held: &Doc| {
            threads(held)
                .first()
                .map(|thread| (thread.color.clone(), thread.resolved))
        };
        change(
            &doc,
            CommentChange::Color {
                thread: "B9ckv".to_owned(),
                color: Some("red10".to_owned()),
            },
        )?;
        change(
            &doc,
            CommentChange::Color {
                thread: "B9ckv".to_owned(),
                color: Some("indigo6".to_owned()),
            },
        )?;
        change(
            &doc,
            CommentChange::Resolve {
                thread: "B9ckv".to_owned(),
                by: USER.to_owned(),
                at: 5.5,
            },
        )?;
        assert_eq!(colored(&doc), Some((Some("indigo6".to_owned()), true)));
        change(
            &doc,
            CommentChange::Reopen {
                thread: "B9ckv".to_owned(),
            },
        )?;
        change(
            &doc,
            CommentChange::Color {
                thread: "B9ckv".to_owned(),
                color: None,
            },
        )?;
        assert_eq!(colored(&doc), Some((None, false)));
        Ok(())
    }

    #[test]
    fn deleting_a_thread_takes_its_replies_and_nothing_else() -> Result<()> {
        let doc = Doc::new();
        start(
            &doc,
            "x_Ctu",
            ThreadAnchor::Object {
                id: "o1".to_owned(),
                x: 0.25,
                y: 0.5,
            },
        )?;
        start(&doc, "B9ckv", point())?;
        change(
            &doc,
            CommentChange::Reply {
                thread: "B9ckv".to_owned(),
                message: message("3fBIc", 300.0, "zz reply"),
            },
        )?;
        change(
            &doc,
            CommentChange::Delete {
                thread: "B9ckv".to_owned(),
            },
        )?;
        assert_eq!(
            threads(&doc)
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            ["x_Ctu"]
        );
        assert_eq!(threads(&doc).len(), 1);
        Ok(())
    }

    #[test]
    fn a_change_to_an_unknown_thread_is_not_found() {
        let doc = Doc::new();
        let outcome = change(
            &doc,
            CommentChange::Reopen {
                thread: "nope".to_owned(),
            },
        );
        assert_eq!(
            outcome.err().map(|error| error.to_string()),
            Some("comment thread \"nope\" not found on this board".to_owned())
        );
    }

    #[test]
    fn an_update_carries_the_change_to_another_document() -> Result<()> {
        let mine = Doc::new();
        let update = start(&mine, "B9ckv", point())?;
        let theirs = Doc::new();
        theirs
            .transact_mut()
            .apply_update(Update::decode_v2(&update)?)?;
        assert_eq!(threads(&theirs), threads(&mine));
        assert_eq!(
            theirs.transact().state_vector(),
            mine.transact().state_vector()
        );
        assert_ne!(mine.transact().state_vector(), StateVector::default());
        Ok(())
    }

    fn applied(update: &[u8]) -> Result<Doc> {
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v2(update)?)?;
        Ok(doc)
    }

    fn entry(doc: &Doc, id: &str) -> Option<Value> {
        let store = doc.get_or_insert_map("commentsStore");
        let txn = doc.transact();
        crate::domain::board::json(&store.get(&txn, id)?.to_json(&txn))
    }

    fn mentions_len(doc: &Doc, id: &str) -> Option<u32> {
        let store = doc.get_or_insert_map("commentsStore");
        let txn = doc.transact();
        let held = store.get(&txn, id)?.cast::<MapRef>().ok()?;
        let mentions = held.get(&txn, "m")?.cast::<ArrayRef>().ok()?;
        Some(mentions.len(&txn))
    }

    fn retarget(doc: &Doc, id: &str, target: &Value) -> Result<()> {
        let store = doc.get_or_insert_map("commentsStore");
        let mut txn = doc.transact_mut();
        let held = store
            .get(&txn, id)
            .and_then(|out| out.cast::<MapRef>().ok());
        held.iter().try_for_each(|held| -> Result<()> {
            held.insert(&mut txn, "t", any(target)?);
            Ok(())
        })
    }

    fn registered(doc: &Doc, user: &str) -> Option<u64> {
        let users = doc.get_or_insert_map("usersIds");
        let txn = doc.transact();
        crate::domain::board::json(&users.get(&txn, user)?.to_json(&txn))?.as_u64()
    }

    #[test]
    fn a_new_thread_registers_its_author_number_in_the_room() -> Result<()> {
        let mine = Doc::new();
        let seen = applied(&start(&mine, "B9ckv", point())?)?;
        let number = u64::from(author_number(USER, 0));
        assert_eq!(registered(&seen, USER), Some(number));
        assert_eq!(
            entry(&seen, "B9ckv")
                .and_then(|held| held.get("a").and_then(Value::as_u64)),
            Some(number)
        );
        Ok(())
    }

    #[test]
    fn a_taken_number_moves_the_author_to_the_next_salt() -> Result<()> {
        let mine = Doc::new();
        {
            let users = mine.get_or_insert_map("usersIds");
            let mut txn = mine.transact_mut();
            users.insert(
                &mut txn,
                "someone-else",
                any(&json!(author_number(USER, 0)))?,
            );
        }
        let seen = applied(&start(&mine, "B9ckv", point())?)?;
        let next = u64::from(author_number(USER, 1));
        assert_eq!(registered(&seen, USER), Some(next));
        assert_eq!(
            entry(&seen, "B9ckv")
                .and_then(|held| held.get("a").and_then(Value::as_u64)),
            Some(next)
        );
        Ok(())
    }

    #[test]
    fn a_resolve_writes_the_resolver_from_the_same_map() -> Result<()> {
        let mine = Doc::new();
        start(&mine, "B9ckv", point())?;
        let update = change(
            &mine,
            CommentChange::Resolve {
                thread: "B9ckv".to_owned(),
                by: "other-user".to_owned(),
                at: 5.5,
            },
        )?;
        let number = registered(&mine, "other-user");
        assert!(number.is_some());
        assert_ne!(number, registered(&mine, USER));
        assert_eq!(
            entry(&mine, "B9ckv")
                .and_then(|held| held.get("rb").and_then(Value::as_u64)),
            number
        );
        assert!(!update.is_empty());
        Ok(())
    }

    fn reply_by(doc: &Doc, user: &str) -> Result<Vec<u8>> {
        change(
            doc,
            CommentChange::Reply {
                thread: "B9ckv".to_owned(),
                message: NewThreadMessage {
                    author: user.to_owned(),
                    ..message("3fBIc", 200.5, "zz reply")
                },
            },
        )
    }

    #[test]
    fn a_reply_registers_an_author_the_room_lacks() -> Result<()> {
        let mine = Doc::new();
        start(&mine, "B9ckv", point())?;
        reply_by(&mine, "other-user")?;
        let number = registered(&mine, "other-user");
        assert!(number.is_some());
        assert_ne!(number, registered(&mine, USER));
        assert_eq!(
            entry(&mine, "3fBIc")
                .and_then(|held| held.get("a").and_then(Value::as_u64)),
            number
        );
        Ok(())
    }

    #[test]
    fn a_reply_opens_a_resolved_thread_again() -> Result<()> {
        let mine = Doc::new();
        start(&mine, "B9ckv", point())?;
        change(
            &mine,
            CommentChange::Resolve {
                thread: "B9ckv".to_owned(),
                by: USER.to_owned(),
                at: 5.5,
            },
        )?;
        reply_by(&mine, USER)?;
        let held = entry(&mine, "B9ckv");
        let field = |name: &str| held.as_ref().and_then(|held| held.get(name));
        assert_eq!(field("r"), Some(&json!(false)));
        assert_eq!(field("ra"), None);
        assert_eq!(field("rb"), None);
        assert_eq!(
            threads(&mine).first().map(|thread| thread.resolved),
            Some(false)
        );
        Ok(())
    }

    #[test]
    fn mentions_are_an_empty_y_array() -> Result<()> {
        let seen = applied(&start(&Doc::new(), "B9ckv", point())?)?;
        assert_eq!(mentions_len(&seen, "B9ckv"), Some(0));
        Ok(())
    }

    #[test]
    fn every_line_is_a_wrapper_with_one_paragraph() -> Result<()> {
        let mine = Doc::new();
        let update = change(
            &mine,
            CommentChange::Start {
                message: message("B9ckv", 1.0, "one\ntwo"),
                anchor: point(),
            },
        )?;
        let seen = applied(&update)?;
        let body = entry(&seen, "B9ckv").and_then(|held| held.get("c").cloned());
        let shape = body.as_ref().and_then(Value::as_array).map(|wrappers| {
            wrappers
                .iter()
                .map(|wrapper| {
                    let children = wrapper.get("children").and_then(Value::as_array);
                    (
                        wrapper.get("type").and_then(Value::as_str),
                        children.map(Vec::len),
                        children
                            .and_then(|inner| inner.first())
                            .and_then(|paragraph| paragraph.get("type"))
                            .and_then(Value::as_str),
                    )
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(
            shape,
            Some(vec![
                (Some("wrapper"), Some(1), Some("paragraph")),
                (Some("wrapper"), Some(1), Some("paragraph")),
            ])
        );
        assert_eq!(
            threads(&seen)
                .first()
                .and_then(|found| found.messages.first())
                .map(|first| first.text.as_str()),
            Some("one\ntwo")
        );
        Ok(())
    }

    #[test]
    fn a_rectangle_target_lists_as_an_area() -> Result<()> {
        let doc = Doc::new();
        start(&doc, "B9ckv", point())?;
        retarget(
            &doc,
            "B9ckv",
            &json!({
                "type": 3,
                "startPoint": { "x": 30.0, "y": 50.0 },
                "endPoint": { "x": 10.0, "y": 80.0 },
            }),
        )?;
        let area = ThreadAnchor::Area {
            x: 10.0,
            y: 50.0,
            width: 20.0,
            height: 30.0,
        };
        assert_eq!(
            threads(&doc).first().map(|found| found.anchor.clone()),
            Some(area.clone())
        );
        assert_eq!(
            serde_json::to_value(&area).ok(),
            Some(
                json!({ "area": { "x": 10.0, "y": 50.0, "width": 20.0, "height": 30.0 } })
            )
        );
        Ok(())
    }
}
