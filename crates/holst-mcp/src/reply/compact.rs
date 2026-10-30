//! API replies, compacted to save the model's tokens.

use std::{fmt, fmt::Write as _};

use holst_core::{
    domain::{
        id::is_uuid,
        text::{EMPTY, hide_tokens, joined, one_line},
    },
    protocol::json::is_noise,
};
use serde_json::{Map, Value};

use crate::error::Error;

const MAX_OUTPUT: usize = 16_000;

#[must_use]
pub(crate) fn render(value: Value, alias: &impl Fn(&str) -> String) -> String {
    truncate(compact(hide_tokens(value), alias))
}

/// A reply as the whole JSON, with ids: what `output: json` gives.
pub(crate) fn render_json(reply: &impl serde::Serialize) -> Result<String, Error> {
    let value = serde_json::to_value(reply).map_err(holst_core::Error::from)?;
    let text = hide_tokens(value).to_string();
    // Cut JSON does not parse: refuse, and let the model ask for less.
    if text.len() > MAX_OUTPUT {
        Err(Error::JsonTooLong {
            bytes: text.len(),
            max: MAX_OUTPUT,
        })
    } else {
        Ok(text)
    }
}

fn compact(mut value: Value, alias: &impl Fn(&str) -> String) -> String {
    prune(&mut value);
    alias_ids(&mut value, alias);
    // An empty list must not be an empty reply: the model reads a failure.
    if is_empty(&value) {
        format!("{EMPTY}\n")
    } else {
        write_value(&value)
    }
}

fn prune(value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        map.retain(|key, item| {
            prune(item);
            !is_noise(key) && !is_empty(item)
        });
        let copies: Vec<String> = map
            .iter()
            .filter(|(key, item)| is_copy(map, key, item))
            .map(|(key, _)| key.clone())
            .collect();
        map.retain(|key, _| !copies.contains(key));
    }
    value.as_array_mut().into_iter().flatten().for_each(prune);
    if let Some(text) = string_mut(value) {
        whole_seconds(text);
    }
}

fn is_copy(map: &Map<String, Value>, key: &str, item: &Value) -> bool {
    let id = item.as_object().and_then(|nested| nested.get("id"));
    id.is_some() && map.get(&format!("{key}_id")) == id
}

fn whole_seconds(text: &mut String) {
    let bytes = text.as_bytes();
    let digit = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
    let stamp = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
        .into_iter()
        .all(digit)
        && bytes.get(10) == Some(&b'T')
        && bytes.get(19) == Some(&b'.');
    let end = if stamp {
        (20..bytes.len()).find(|&at| !digit(at))
    } else {
        None
    };
    if let Some(end) = end.filter(|&end| end > 20) {
        text.replace_range(19..end, "");
    }
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

fn alias_ids(value: &mut Value, alias: &impl Fn(&str) -> String) {
    if let Some(text) = string_mut(value).filter(|text| is_uuid(text)) {
        *text = alias(text);
    }
    value
        .as_array_mut()
        .into_iter()
        .flatten()
        .for_each(|item| alias_ids(item, alias));
    if let Some(map) = value.as_object_mut() {
        alias_object(map, alias);
    }
}

fn string_mut(value: &mut Value) -> Option<&mut String> {
    match value {
        Value::String(s) => Some(s),
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::Array(_)
        | Value::Object(_) => None,
    }
}

fn alias_object(map: &mut Map<String, Value>, alias: &impl Fn(&str) -> String) {
    if map.keys().any(|key| is_uuid(key)) {
        // `extend`, not `collect`: `from_iter` builds an extra `Vec`.
        let old = std::mem::take(map);
        map.extend(
            old.into_iter()
                .map(|(key, item)| (alias_key(key, alias), item)),
        );
    }
    map.values_mut().for_each(|item| alias_ids(item, alias));
}

fn alias_key(key: String, alias: &impl Fn(&str) -> String) -> String {
    if is_uuid(&key) { alias(&key) } else { key }
}

fn write_value(value: &Value) -> String {
    match value {
        Value::Object(map) => fmt::from_fn(|out| write_object(out, map)).to_string(),
        Value::Array(items) if is_table(items) => {
            fmt::from_fn(|out| write_table(out, items)).to_string()
        }
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_) => format!("{}\n", scalar(value)),
    }
}

fn write_object(out: &mut impl fmt::Write, map: &Map<String, Value>) -> fmt::Result {
    map.iter().try_for_each(|(key, item)| match item {
        Value::Array(items) if is_table(items) => {
            writeln!(out, "{key} [{}]:", items.len())?;
            write_table(out, items)
        }
        Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Object(_) => writeln!(out, "{key}: {}", scalar(item)),
    })
}

fn is_table(items: &[Value]) -> bool {
    items.len() >= 2 && items.iter().all(Value::is_object)
}

fn write_table(out: &mut impl fmt::Write, items: &[Value]) -> fmt::Result {
    // Columns in the order they first appear.
    let columns = items
        .iter()
        .filter_map(Value::as_object)
        .flat_map(Map::keys)
        .fold(Vec::<&str>::new(), |mut columns, key| {
            if !columns.contains(&key.as_str()) {
                columns.push(key.as_str());
            }
            columns
        });
    writeln!(out, "{}", joined(&columns, "|"))?;
    items.iter().try_for_each(|item| {
        let line = columns.iter().map(|column| {
            fmt::from_fn(move |f| {
                item.get(*column)
                    .map_or(Ok(()), |value| write!(f, "{}", cell(value)))
            })
        });
        writeln!(out, "{}", joined(line, "|"))
    })
}

fn scalar(value: &Value) -> impl fmt::Display {
    escaped(value, PipeEscape::Keep)
}

fn cell(value: &Value) -> impl fmt::Display {
    escaped(value, PipeEscape::Escape)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PipeEscape {
    Keep,
    Escape,
}

fn escaped(value: &Value, pipe: PipeEscape) -> impl fmt::Display {
    fmt::from_fn(move |f| match (value, pipe) {
        (Value::String(s), PipeEscape::Keep) => f.write_str(&one_line(s)),
        (Value::String(s), PipeEscape::Escape) => {
            PipeEscaper { f }.write_str(&one_line(s))
        }
        // No raw line breaks in compact JSON, but nested strings may hold `|`.
        (other, PipeEscape::Escape) => write!(PipeEscaper { f }, "{other}"),
        (other, PipeEscape::Keep) => write!(f, "{other}"),
    })
}

struct PipeEscaper<'a, 'f> {
    f: &'a mut fmt::Formatter<'f>,
}

impl fmt::Write for PipeEscaper<'_, '_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let tail = s.bytes().enumerate().filter(|&(_, b)| b == b'|').try_fold(
            0,
            |from, (at, _)| {
                self.f.write_str(s.get(from..at).unwrap_or_default())?;
                self.f.write_str("\\|")?;
                Ok(at.saturating_add(1))
            },
        )?;
        self.f.write_str(s.get(tail..).unwrap_or_default())
    }
}

fn truncate(text: String) -> String {
    match text.len() {
        len if len <= MAX_OUTPUT => text,
        len => {
            let boundary = text.floor_char_boundary(MAX_OUTPUT);
            let head = text.get(..boundary).unwrap_or_default();
            let cut = head.rfind('\n').unwrap_or(boundary);
            format!(
                "{}\n… {} more bytes cut: narrow the request (limit, target)",
                text.get(..cut).unwrap_or_default(),
                len.saturating_sub(cut)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::{
        memory::McpMemory,
        reply::compact::{MAX_OUTPUT, render, render_json, truncate},
    };

    const ID: &str = "ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b";

    fn compact_with(state: &McpMemory, value: Value) -> String {
        render(value, &|id| state.alias(id))
    }

    #[rstest::rstest]
    #[case::prunes_noise_and_aliases_ids(
        "prune",
        json!({ "id": ID, "name": "Board", "thumbnail": "x", "public_access": null, "tags": [] })
    )]
    #[case::lists_of_objects_become_tables(
        "table",
        json!([{ "id": ID, "name": "A|B" }, { "name": "C", "role": "editor" }])
    )]
    fn render_compact(#[case] name: &str, #[case] input: Value) {
        insta::assert_snapshot!(name, compact_with(&McpMemory::default(), input));
    }

    #[test]
    fn nested_copies_and_fractions_go() {
        let state = McpMemory::default();
        let text = compact_with(
            &state,
            json!({
                "workspace_id": ID,
                "workspace": { "id": ID, "name": "W" },
                "owner": { "id": ID },
                "created_at": "2026-02-18T06:38:51.999455Z",
                "note": "2026-02-18T06:38:51.x",
            }),
        );
        assert_eq!(
            text,
            "created_at: 2026-02-18T06:38:51Z\nnote: 2026-02-18T06:38:51.x\n\
             owner: {\"id\":\"@1\"}\nworkspace_id: @1\n"
        );
    }

    #[test]
    fn line_breaks_read_as_slashes() {
        let state = McpMemory::default();
        let text = compact_with(&state, json!({ "text": "Цель:\nfirewall\u{1b}[2J" }));
        assert_eq!(text, "text: Цель: / firewall\u{fffd}[2J\n");
        let table = compact_with(&state, json!([{ "a": "x\ny|z" }, { "a": "b" }]));
        assert_eq!(table, "a\nx / y\\|z\nb\n");
    }

    #[test]
    fn json_keeps_everything_but_the_tokens() {
        let text = render_json(&json!({ "id": ID, "x": null, "t": "holst_pat_a" }))
            .unwrap_or_default();
        assert_eq!(
            serde_json::from_str::<Value>(&text).ok(),
            Some(json!({ "id": ID, "x": null, "t": "***" }))
        );
    }

    #[test]
    fn uuid_keys_and_nested_values_become_aliases() {
        let state = McpMemory::default();
        let text = compact_with(&state, json!({ ID: { "owner": ID } }));
        assert_eq!(text, "@1: {\"owner\":\"@1\"}\n");
    }

    #[test]
    fn long_output_is_cut_on_a_line_with_a_note() {
        let lines: Vec<String> = (0..5000).map(|i| format!("line {i}")).collect();
        let long = lines.join("\n") + "\n";
        let out = truncate(long);
        assert!(out.len() < MAX_OUTPUT + 200);
        assert!(out.contains("more bytes cut"));
        assert!(!out.contains("line 4999"));
    }

    #[test]
    fn long_json_output_is_an_error_never_cut_json() {
        let big: Vec<Value> = (0..2_000)
            .map(|number| json!({ "n": number, "pad": "x".repeat(20) }))
            .collect();
        let err = render_json(&big).err().map(|err| err.to_string());
        assert!(
            err.as_deref()
                .is_some_and(|text| text.contains("pass a lower `limit`")),
            "{err:?}"
        );
        let small =
            render_json(&big.iter().take(3).collect::<Vec<_>>()).unwrap_or_default();
        assert!(serde_json::from_str::<Value>(&small).is_ok(), "{small}");
    }
}
