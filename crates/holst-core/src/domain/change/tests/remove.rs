#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::collections::{BTreeSet, HashMap};

use rstest::rstest;
use serde_json::{Value, json};
use yrs::{
    ClientID,
    Doc,
    IdSet,
    Map as _,
    ReadTxn,
    Transact as _,
    Update,
    types::ToJson as _,
    updates::decoder::Decode as _,
};

use super::{add, edit, remove};
use crate::{
    domain::{
        board::any,
        change::{
            build::{author_number, build},
            check::{check, find},
            tests::{ME, board, copy, loaded_with_history},
            value::{
                BoardChange,
                ChangeSources,
                NewObject,
                ObjectChangeKind,
                ObjectPatch,
                RemoveRight,
            },
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        time::Timestamp,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        lookup,
        lookup::LookupError,
    },
};

#[test]
fn removing_takes_attached_arrows_along() -> Result<()> {
    let document = board()?;
    let ids = SequentialIds::new("r");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(3),
        ids: &ids,
    };
    build(
        &document,
        &add(NewObject::Arrow {
            from: "seed-2".into(),
            to: "seed-6".into(),
            color: None,
        }),
        ME,
        &context,
    )?;
    let built = build(&document, &remove("seed-2".into()), ME, &context)?;
    assert_eq!(
        built
            .objects
            .iter()
            .map(|t| (t.id.as_str(), t.change))
            .collect::<Vec<_>>(),
        [
            ("seed-2", ObjectChangeKind::Removed),
            ("r-1", ObjectChangeKind::Removed)
        ]
    );
    assert_eq!(board::objects(&document).len(), 2);
    Ok(())
}

fn clocks(set: &IdSet) -> BTreeSet<(ClientID, u32)> {
    set.iter()
        .flat_map(|(client, ranges)| {
            ranges
                .into_iter()
                .flat_map(|range| range.clone().map(|clock| (*client, clock)))
        })
        .collect()
}

#[test]
fn an_add_does_not_carry_earlier_deletions() -> Result<()> {
    let sizes = [1, 50, 500]
        .into_iter()
        .map(|deleted| {
            let document = loaded_with_history(deleted)?;
            let ids = SequentialIds::new("n");
            let built = build(
                &document,
                &add(NewObject::Sticker {
                    text: "zz".into(),
                    at: Point::new(5.0, 5.0)?,
                    size: None,
                    color: None,
                }),
                ME,
                &ChangeSources {
                    built_at: Timestamp::from_millis(2),
                    ids: &ids,
                },
            )?;
            let update = Update::decode_v2(&built.update)?;
            assert!(update.delete_set().is_empty(), "{deleted} deletions leaked");
            Ok(built.update.len())
        })
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(sizes.iter().collect::<BTreeSet<_>>().len(), 1, "{sizes:?}");
    assert!(sizes.iter().all(|&size| size < 1024), "{sizes:?}");
    Ok(())
}

#[test]
fn a_remove_deletes_exactly_what_it_removes() -> Result<()> {
    let document = loaded_with_history(200)?;
    let original = copy(&document)?;
    let before = document.transact().snapshot();
    let ids = SequentialIds::new("r");
    let built = build(
        &document,
        &remove("h-1".into()),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(3),
            ids: &ids,
        },
    )?;
    let after = document.transact().snapshot();
    let sent = clocks(Update::decode_v2(&built.update)?.delete_set());
    let context: BTreeSet<_> = clocks(&after.delete_set)
        .difference(&clocks(&before.delete_set))
        .copied()
        .collect();
    assert!(!sent.is_empty());
    assert_eq!(sent, context);
    original
        .transact_mut()
        .apply_update(Update::decode_v2(&built.update)?)?;
    let left = |d: &Doc| {
        board::objects(d)
            .into_iter()
            .map(|o| o.id)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(left(&original), left(&document));
    assert_eq!(
        left(&document),
        ["h-10", "h-6"].map(String::from).into_iter().collect()
    );
    assert_eq!(original.transact().snapshot(), after);
    Ok(())
}

#[rstest]
#[case::text_of_arrow(ObjectPatch { text: Some("x".into()), ..ObjectPatch::default() }, "arrow objects have no text to change")]
#[case::nothing(
    ObjectPatch::default(),
    "nothing to change: give a text, a position, a size, a color, an order or tags"
)]
fn arrows_refuse_text(#[case] patch: ObjectPatch, #[case] message: &str) -> Result<()> {
    let document = board()?;
    let ids = SequentialIds::new("x");
    let context = ChangeSources {
        built_at: Timestamp::from_millis(3),
        ids: &ids,
    };
    build(
        &document,
        &add(NewObject::Arrow {
            from: "seed-2".into(),
            to: "seed-6".into(),
            color: None,
        }),
        ME,
        &context,
    )?;
    let refused = build(&document, &edit("x-1".into(), patch), ME, &context);
    assert_eq!(
        refused.err().map(|error| error.to_string()).as_deref(),
        Some(message)
    );
    Ok(())
}

#[test]
fn objects_are_found_by_id_link_or_prefix() {
    let (first, second) = (
        "abcdef01-0000-4000-8000-000000000001",
        "abcdef02-0000-4000-8000-000000000002",
    );
    let states: HashMap<String, Value> = [(first, json!({})), (second, json!({}))]
        .into_iter()
        .map(|(id, v)| (id.to_owned(), v))
        .collect();
    assert_eq!(find(&states, "ABCDEF01").ok().as_deref(), Some(first));
    assert_eq!(
        find(&states, &format!("https://h/board/b?objectId={second}&x=1"))
            .ok()
            .as_deref(),
        Some(second)
    );
    assert!(matches!(
        find(&states, "abcdef0"),
        Err(Error::BoardChange(BoardChangeError::NoObject { .. }))
    ));
    assert!(matches!(
        find(&states, "abcdef0-"),
        Err(Error::BoardChange(BoardChangeError::NoObject { .. }))
    ));
    assert!(matches!(
        find(&states, "ffffffff"),
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Object,
            miss: lookup::LookupMiss::NotFound { .. },
            ..
        }))
    ));
    assert!(matches!(
        find(&states, ""),
        Err(Error::BoardChange(BoardChangeError::NoObject { .. }))
    ));
}

#[test]
fn a_sticker_is_square_or_wide() {
    let sticker = |width, height| {
        build(
            &Doc::new(),
            &add(NewObject::Sticker {
                text: String::new(),
                at: Point::ORIGIN,
                size: Some(Size::new(width, height)?),
                color: None,
            }),
            ME,
            &ChangeSources {
                built_at: Timestamp::from_millis(0),
                ids: &SequentialIds::new("s"),
            },
        )
        .map(drop)
    };
    assert!(sticker(192.0, 192.0).is_ok() && sticker(384.0, 192.0).is_ok());
    assert!(matches!(
        sticker(300.0, 192.0),
        Err(Error::BoardChange(BoardChangeError::StickerSize { .. }))
    ));
}

#[test]
fn a_taken_author_number_is_salted() -> Result<()> {
    let document = Doc::new();
    {
        let users = document.get_or_insert_map("usersIds");
        // imperative: yrs writes take `&mut TransactionMut`
        let mut transaction = document.transact_mut();
        users.insert(
            &mut transaction,
            "someone-else",
            any(&json!(author_number("me", 0)))?,
        );
    }
    let ids = SequentialIds::new("u");
    build(
        &document,
        &add(NewObject::Text {
            text: "t".into(),
            at: Point::ORIGIN,
        }),
        "me",
        &ChangeSources {
            built_at: Timestamp::from_millis(0),
            ids: &ids,
        },
    )?;
    let transaction = document.transact();
    let mine = transaction
        .get_map("usersIds")
        .and_then(|u| u.get(&transaction, "me"))
        .map(|number| number.to_json(&transaction));
    assert_eq!(
        mine.as_ref().and_then(board::json),
        Some(json!(author_number("me", 1)))
    );
    Ok(())
}

const MINE: u32 = 7;

/// Objects made by me (`mine-*`) and by someone else (`theirs-*`).
fn made_by_two() -> HashMap<String, Value> {
    [
        json!({"id": "mine-sticker", "type": "sticker", "created": {"a": MINE, "t": 1}}),
        json!({"id": "theirs-sticker", "type": "sticker", "created": {"a": 8, "t": 1}}),
        json!({"id": "unsigned", "type": "sticker"}),
        json!({"id": "mine-frame", "type": "frame", "created": {"a": MINE, "t": 1}}),
        json!({"id": "theirs-in-mine", "type": "sticker", "parentId": "mine-frame",
               "created": {"a": 8, "t": 1}}),
        json!({"id": "theirs-group", "type": "group", "created": {"a": 8, "t": 1}}),
        json!({"id": "mine-in-theirs", "type": "sticker", "parentId": "theirs-group",
               "created": {"a": MINE, "t": 1}}),
        json!({"id": "pinned", "type": "sticker", "created": {"a": 8, "t": 1},
               "stickyPosition": {"parentId": "mine-sticker"}}),
    ]
    .into_iter()
    .filter_map(|object| Some((object.get("id")?.as_str()?.to_owned(), object)))
    .collect()
}

#[rstest]
#[case::own(RemoveRight::OwnOnly, Some(MINE), &["mine-sticker"], None)]
#[case::theirs(RemoveRight::OwnOnly, Some(MINE), &["theirs-sticker"], Some("theirs-sticker"))]
#[case::unsigned(RemoveRight::OwnOnly, Some(MINE), &["unsigned"], Some("unsigned"))]
#[case::unknown_to_the_board(RemoveRight::OwnOnly, None, &["mine-sticker"], Some("mine-sticker"))]
#[case::one_of_two(RemoveRight::OwnOnly, Some(MINE), &["mine-sticker", "theirs-sticker"], Some("theirs-sticker"))]
#[case::theirs_inside_mine(RemoveRight::OwnOnly, Some(MINE), &["mine-frame"], Some("theirs-in-mine"))]
#[case::mine_inside_a_group(RemoveRight::OwnOnly, Some(MINE), &["theirs-group"], None)]
#[case::pinned_to_what_goes(RemoveRight::OwnOnly, Some(MINE), &["mine-sticker", "pinned"], None)]
#[case::pinned_alone(RemoveRight::OwnOnly, Some(MINE), &["pinned"], Some("pinned"))]
#[case::anyone(RemoveRight::Anyone, None, &["theirs-sticker", "theirs-in-mine"], None)]
fn another_users_object_is_removed_only_where_the_board_allows(
    #[case] right: RemoveRight,
    #[case] mine: Option<u32>,
    #[case] named: &[&str],
    #[case] refused: Option<&str>,
) {
    let change =
        BoardChange::RemoveAll(named.iter().map(|id| (*id).to_owned()).collect());
    assert_eq!(not_yours(&change, right, mine).as_deref(), refused);
}

fn not_yours(
    change: &BoardChange,
    right: RemoveRight,
    mine: Option<u32>,
) -> Option<String> {
    match check(change, &made_by_two(), right, mine) {
        Err(Error::BoardChange(BoardChangeError::NotYours { reference })) => {
            Some(reference)
        }
        Err(_) | Ok(_) => None,
    }
}
