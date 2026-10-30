//! A change built in one transaction: read, checked, written.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Value, json};
use yrs::{
    Doc,
    Map as _,
    MapRef,
    ReadTxn,
    Transact as _,
    TransactionMut,
    types::ToJson as _,
};

use crate::{
    domain::{
        board::{any, note_root},
        change::{
            check::check,
            journal,
            value::{BoardChange, ChangeSources, ChangeUpdate, RemoveRight},
            write::{AuthorStamp, DocumentRoots, WriteTransaction, write_change},
        },
    },
    error::Result,
    protocol::document::{DOCUMENTS, OBJECTS, TAGS_STORE, USERS_IDS},
};

/// Apply `op` to `document` as `user` in one transaction and encode just that
/// transaction as a v2 update.
///
/// # Errors
/// A [`crate::error::lookup::LookupError`] of an object,
/// [`crate::error::board_change::BoardChangeError::NotChangeable`],
/// [`crate::error::board_change::BoardChangeError::NothingToChange`],
/// [`crate::error::board_change::BoardChangeError::StickerSize`], or
/// [`crate::error::http::HttpError::Json`].
pub fn build(
    document: &Doc,
    change: &BoardChange,
    user: &str,
    context: &ChangeSources<'_>,
) -> Result<ChangeUpdate> {
    build_with(document, change, user, context, RemoveRight::Anyone)
}

/// As [`build`], with what `right` lets a removal take.
///
/// # Errors
/// As [`build`], and [`crate::error::board_change::BoardChangeError::NotYours`]
/// for a removal of what the user did not make under [`RemoveRight::OwnOnly`].
pub fn build_with(
    document: &Doc,
    change: &BoardChange,
    user: &str,
    context: &ChangeSources<'_>,
    right: RemoveRight,
) -> Result<ChangeUpdate> {
    let roots = DocumentRoots {
        objects: document.get_or_insert_map(OBJECTS),
        documents: document.get_or_insert_map(DOCUMENTS),
        users: document.get_or_insert_map(USERS_IDS),
        journal: document.get_or_insert_map(journal::JOURNAL),
        tags: document.get_or_insert_map(TAGS_STORE),
        note: note_root(document),
    };
    let mut transaction = document.transact_mut();
    let states = states(&transaction, &roots.objects);
    let mine = known_author(&transaction, &roots.users, user);
    let checked = check(change, &states, right, mine)?;
    let stamp = AuthorStamp {
        author: author(&mut transaction, &roots.users, user)?,
        at: context.built_at,
    };
    let scope = WriteTransaction {
        roots: &roots,
        states: &states,
        stamp,
        context,
    };
    let kept = checked.kept();
    let touched = write_change(&mut transaction, &scope, checked)?;
    transaction.commit();
    Ok(ChangeUpdate {
        update: transaction.encode_update_v2(),
        objects: touched,
        kept,
    })
}

fn states<T: ReadTxn>(transaction: &T, objects: &MapRef) -> HashMap<String, Value> {
    objects
        .iter(transaction)
        .filter_map(|(id, value)| {
            crate::domain::board::json(&value.to_json(transaction))
                .map(|v| (id.to_owned(), v))
        })
        .collect()
}

/// The author number the board's users hold for `user`, if it holds one.
fn known_author<T: ReadTxn>(
    transaction: &T,
    users: &MapRef,
    user: &str,
) -> Option<u32> {
    users
        .get(transaction, user)
        .and_then(|number| crate::domain::board::json(&number.to_json(transaction)))
        .and_then(|number| number.as_u64())
        .and_then(|number| u32::try_from(number).ok())
}

pub(crate) fn author(
    transaction: &mut TransactionMut<'_>,
    users: &MapRef,
    user: &str,
) -> Result<u32> {
    let mine = known_author(&*transaction, users, user);
    let taken: BTreeSet<u64> = users
        .iter(&*transaction)
        .filter_map(|(_, number)| {
            crate::domain::board::json(&number.to_json(&*transaction))
                .and_then(|number| number.as_u64())
        })
        .collect();
    mine.map_or_else(
        || {
            let number = (0..u32::MAX)
                .map(|salt| author_number(user, salt))
                .find(|number| !taken.contains(&u64::from(*number)))
                .unwrap_or_default();
            users.insert(transaction, user, any(&json!(number))?);
            Ok(number)
        },
        Ok,
    )
}

/// The web client's author number of a user id: a 31-bit cyrb53-style hash.
///
/// ```
/// use holst_core::domain::change::build::author_number;
///
/// assert_eq!(
///     author_number("00000000-0000-4000-8000-0000000000a1", 0),
///     392_241_187
/// );
/// ```
#[must_use]
pub fn author_number(user: &str, salt: u32) -> u32 {
    let (h1, h2) = user.encode_utf16().fold(
        (0xdead_beef ^ salt, 0x41c6_ce57 ^ salt),
        |(h1, h2), unit| {
            let unit = u32::from(unit);
            (
                (h1 ^ unit).wrapping_mul(0x9e37_79b1),
                (h2 ^ unit).wrapping_mul(0x5f35_6495),
            )
        },
    );
    let h1 = (h1 ^ (h1 >> 16)).wrapping_mul(0x85eb_ca6b)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(0xc2b2_ae35);
    let h2 = (h2 ^ (h2 >> 16)).wrapping_mul(0x85eb_ca6b)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(0xc2b2_ae35);
    (h1 ^ h2) & 0x7fff_ffff
}

/// A 31-bit Yjs client id drawn from `seed` until no client of the board has
/// it.
#[must_use]
pub fn client_id(seed: &str, taken: impl Fn(u64) -> bool) -> u64 {
    (3..u32::MAX)
        .map(|salt| u64::from(author_number(seed, salt)))
        .find(|id| !taken(*id))
        .unwrap_or_default()
}

/// An update id below 2^52 from a fresh id, as the web client draws one.
#[must_use]
pub fn update_id(seed: &str) -> u64 {
    (u64::from(author_number(seed, 0)) << 21)
        | u64::from(author_number(seed, 1) & 0x1f_ffff)
}
