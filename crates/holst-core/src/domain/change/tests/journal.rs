#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::collections::HashMap;

use itertools::Itertools as _;
use rstest::rstest;
use serde_json::{Value, json};
use yrs::{
    Doc,
    Map as _,
    ReadTxn,
    Transact as _,
    Update,
    types::ToJson as _,
    updates::decoder::Decode as _,
};

use super::{add, remove};
use crate::{
    domain::{
        board::any,
        change::{
            build::{author_number, build},
            tests::{ME, board, copy, fields_of, loaded_with_history},
            value::{ArrowLook, ChangeSources, NewObject},
        },
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::Result,
};

const WEB_ENTRY: &str =
    r#"{"id": "kj0b8", "a": 392241187, "t": 3, "cnt": 1, "type": "sticker", "s": []}"#;

fn journal_entry(document: &Doc, id: &str) -> (Value, bool) {
    let transaction = document.transact();
    let entry = transaction
        .get_map(crate::domain::change::journal::JOURNAL)
        .and_then(|journal| journal.get(&transaction, id));
    let plain = entry
        .clone()
        .and_then(|out| out.cast::<yrs::MapRef>().ok())
        .is_some_and(|fields| {
            fields
                .iter(&transaction)
                .all(|(_, value)| matches!(value, yrs::Out::Any(_)))
        });
    let json = entry
        .and_then(|entry| board::json(&entry.to_json(&transaction)))
        .unwrap_or_default();
    (json, plain)
}

fn restore(document: &Doc, id: &str) -> Result<()> {
    let (entry, _) = journal_entry(document, id);
    let layer = |state: &Value| {
        state
            .get("zIndex")
            .and_then(Value::as_f64)
            .unwrap_or_default()
    };
    let objects = document.get_or_insert_map("objects");
    // imperative: yrs writes take `&mut TransactionMut`
    let mut transaction = document.transact_mut();
    entry
        .get("s")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .sorted_by(|one, other| layer(one).total_cmp(&layer(other)))
        .try_for_each(|state| {
            let fields = state
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, value)| any(value).map(|value| (name.clone(), value)))
                .collect::<Result<Vec<_>>>()?;
            let object = state.get("id").and_then(Value::as_str).unwrap_or_default();
            objects.insert(&mut transaction, object, yrs::MapPrelim::from_iter(fields));
            Ok(())
        })
}

#[rstest]
#[case::none(&[], "0")]
#[case::short(&["a"], "2p")]
#[case::one(&["seed-2"], "kj0b8")]
#[case::unsorted(&["seed-2", "seed-1", "r-1"], "uzeem")]
#[case::uuid(&["00000000-0000-4000-8000-0000000000a5"], "byemt")]
#[case::repeated(
        &["aaaaaaaa-0000-4000-8000-000000000002", "aaaaaaaa-0000-4000-8000-000000000001", "aaaaaaaa-0000-4000-8000-000000000001"],
        "5otpe"
    )]
fn journal_ids_match_the_web_client(#[case] ids: &[&str], #[case] expected: &str) {
    assert_eq!(
        crate::domain::change::journal::entry_id(ids.iter().copied()),
        expected
    );
}

#[test]
fn a_locked_group_is_journaled_for_unlocking() {
    let states: HashMap<String, Value> = [
        ("g", json!({"id": "g", "type": "group", "locked": true})),
        ("h", json!({"id": "h", "type": "group"})),
    ]
    .into_iter()
    .map(|(id, state)| (id.to_owned(), state))
    .collect();
    let stamp = crate::domain::change::write::AuthorStamp {
        author: 1,
        at: Timestamp::from_millis(2),
    };
    let entry = crate::domain::change::journal::entry(&states, &["g", "h"], stamp);
    let field = |name: &str| entry.fields.get(name).cloned();
    assert_eq!(field("type"), Some(json!("group")));
    assert_eq!(field("unlock"), Some(json!(true)));
    let plain = crate::domain::change::journal::entry(&states, &["h"], stamp);
    assert_eq!(plain.fields.get("unlock"), None);
}

#[test]
fn a_delete_alone_does_not_advance_the_state_vector() -> Result<()> {
    let document = board()?;
    let before = document.transact().state_vector();
    let objects = document.get_or_insert_map("objects");
    objects.remove(&mut document.transact_mut(), "seed-6");
    assert_eq!(document.transact().state_vector(), before);
    Ok(())
}

#[test]
fn a_removal_advances_the_state_vector() -> Result<()> {
    let document = loaded_with_history(20)?;
    let cached = copy(&document)?;
    let before = cached.transact().state_vector();
    let ids = SequentialIds::new("r");
    let built = build(
        &document,
        &remove("h-10".into()),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(3),
            ids: &ids,
        },
    )?;
    let sent = Update::decode_v2(&built.update)?;
    let announced = sent.state_vector();
    let ahead = announced
        .iter()
        .any(|(client, clock)| before.get(client) < *clock);
    assert!(ahead, "the update announces nothing new: {announced:?}");
    cached.transact_mut().apply_update(sent)?;
    assert_eq!(
        cached.transact().state_vector(),
        document.transact().state_vector()
    );
    assert!(
        board::objects(&cached)
            .iter()
            .all(|object| object.id != "h-10")
    );
    Ok(())
}

#[test]
fn a_removal_journal_entry_has_the_web_clients_shape() -> Result<()> {
    let document = board()?;
    let sticker = fields_of(&document, "seed-2");
    let ids = SequentialIds::new("r");
    build(
        &document,
        &remove("seed-2".into()),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(3),
            ids: &ids,
        },
    )?;
    let expected = Value::Object(
        serde_json::from_str::<serde_json::Map<String, Value>>(WEB_ENTRY)?
            .into_iter()
            .chain([("s".to_owned(), json!([sticker]))])
            .collect(),
    );
    let (entry, plain) = journal_entry(&document, "kj0b8");
    assert_eq!(entry, expected);
    assert!(plain, "every field of an entry is a plain value, `s` too");
    let users = fields_of_root(&document, "usersIds");
    assert_eq!(users.get(ME), Some(&json!(author_number(ME, 0))));
    Ok(())
}

fn fields_of_root(document: &Doc, name: &str) -> Value {
    let transaction = document.transact();
    transaction
        .get_map(name)
        .and_then(|root| board::json(&root.to_json(&transaction)))
        .unwrap_or_default()
}

#[test]
fn a_cascade_is_one_entry_of_mixed_kinds() -> Result<()> {
    let document = board()?;
    let ids = SequentialIds::new("r");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(3),
        ids: &ids,
    };
    let arrow = NewObject::Arrow {
        from: "seed-2".into(),
        to: "seed-6".into(),
        color: None,
        look: ArrowLook::default(),
    };
    build(&document, &add(arrow), ME, &context)?;
    let built = build(&document, &remove("seed-1".into()), ME, &context)?;
    let gone: Vec<&str> = built.objects.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(gone, ["seed-1", "r-1", "seed-2"]);
    let id = crate::domain::change::journal::entry_id(gone.iter().copied());
    let (entry, _) = journal_entry(&document, &id);
    assert_eq!(entry.get("cnt"), Some(&json!(3)));
    assert_eq!(entry.get("type"), None, "kinds differ: no `type`");
    let saved: Vec<&str> = entry
        .get("s")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|state| state.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(saved, gone);
    Ok(())
}

#[test]
fn a_journaled_removal_restores_as_the_web_client_does() -> Result<()> {
    let document = board()?;
    let ids = SequentialIds::new("r");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(3),
        ids: &ids,
    };
    let arrow = NewObject::Arrow {
        from: "seed-2".into(),
        to: "seed-6".into(),
        color: None,
        look: ArrowLook::default(),
    };
    build(&document, &add(arrow), ME, &context)?;
    let sorted = |target: &Doc| {
        board::objects(target)
            .into_iter()
            .sorted_by(|one, other| one.id.cmp(&other.id))
            .collect::<Vec<_>>()
    };
    let before = sorted(&document);
    let built = build(&document, &remove("seed-1".into()), ME, &context)?;
    assert_eq!(board::objects(&document).len(), 1);
    let gone = built.objects.iter().map(|t| t.id.as_str());
    restore(&document, &crate::domain::change::journal::entry_id(gone))?;
    assert_eq!(sorted(&document), before, "fields and texts come back");
    Ok(())
}
