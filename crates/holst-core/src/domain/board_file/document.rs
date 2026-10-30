//! A whole board document as one Yjs v2 update and the plan to import it; pure.

use std::collections::BTreeSet;

use yrs::{
    Doc,
    Map as _,
    ReadTxn as _,
    StateVector,
    Transact as _,
    Update,
    types::ToJson as _,
    undo::UndoManager,
    updates::decoder::Decode as _,
};

use crate::{
    domain::board::{is_listed, json},
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::OBJECTS,
};

/// Whether an import may go into a board that already has objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    EmptyOnly,
    Merge,
}

/// The whole document as a Yjs v2 update, with pending parts and deletions.
#[must_use]
pub fn encoded(doc: &Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v2(&StateVector::default())
}

/// A fresh document holding a saved state.
///
/// # Errors
/// [`crate::error::board_socket::BoardSocketError::Decode`] or
/// [`crate::error::board_socket::BoardSocketError::UpdateNotMerged`] for a
/// broken update.
pub fn decoded(board_file: &[u8]) -> Result<Doc> {
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v2(board_file)?)?;
    Ok(doc)
}

/// A saved state applied to a loaded board: the update and object counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    pub update: Vec<u8>,
    pub saved: usize,
    pub before: usize,
    pub after: usize,
}

/// How many objects a saved state holds, after checking that it decodes.
///
/// # Errors
/// As [`decoded`].
pub fn saved_objects(board_file: &[u8]) -> Result<usize> {
    decoded(board_file).map(|doc| object_count(&doc))
}

/// Counts what `board::objects` would list, without sorting or routing.
fn object_count(doc: &Doc) -> usize {
    let objects = doc.get_or_insert_map(OBJECTS);
    let txn = doc.transact();
    objects
        .iter(&txn)
        .filter_map(|(_, value)| json(&value.to_json(&txn)))
        .filter(is_listed)
        .count()
}

/// Applies `board_file`, holding `saved` objects, to `target`; encodes what the
/// server lacks.
///
/// # Errors
/// [`crate::error::board_change::BoardChangeError::NotEmpty`] under
/// [`Overwrite::EmptyOnly`], or as [`decoded`].
pub fn import_plan(
    target: &Doc,
    board_file: &[u8],
    saved: usize,
    overwrite: Overwrite,
) -> Result<ImportPlan> {
    let before = object_count(target);
    match (overwrite, before) {
        (Overwrite::EmptyOnly, 1..) => {
            Err(Error::BoardChange(BoardChangeError::NotEmpty {
                objects: before,
            }))
        }
        (Overwrite::EmptyOnly, 0) | (Overwrite::Merge, _) => {
            applied(target, board_file, saved, before)
        }
    }
}

/// Like [`import_plan`] with no limit, but `target` becomes exactly the saved
/// state.
///
/// # Errors
/// As [`decoded`].
pub fn replace_plan(
    target: &Doc,
    board_file: &[u8],
    saved: usize,
) -> Result<ImportPlan> {
    let before = object_count(target);
    applied(target, &reverted(target, board_file)?, saved, before)
}

/// Applies `board_file` to `target`; encodes what the server lacks.
fn applied(
    target: &Doc,
    board_file: &[u8],
    saved: usize,
    before: usize,
) -> Result<ImportPlan> {
    let seen = target.transact().state_vector();
    target
        .transact_mut()
        .apply_update(Update::decode_v2(board_file)?)?;
    let update = target.transact().encode_state_as_update_v2(&seen);
    Ok(ImportPlan {
        update,
        saved,
        before,
        after: object_count(target),
    })
}

/// The update that takes `target` back to the saved state, as a version
/// restore.
fn reverted(target: &Doc, board_file: &[u8]) -> Result<Vec<u8>> {
    let saved = decoded(board_file)?;
    let current = target.transact().state_vector();
    let newer = target
        .transact()
        .encode_state_as_update_v2(&saved.transact().state_vector());
    let roots: BTreeSet<String> = [target, &saved]
        .into_iter()
        .flat_map(|doc| {
            doc.transact()
                .root_refs()
                .map(|(name, _)| name.to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let mut undo = UndoManager::<()>::new();
    roots
        .into_iter()
        .map(|name| undo.expand_scope(&saved, &saved.get_or_insert_map(name)))
        .for_each(drop);
    saved
        .transact_mut()
        .apply_update(Update::decode_v2(&newer)?)?;
    undo.undo_blocking();
    Ok(saved.transact().encode_state_as_update_v2(&current))
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use yrs::{
        Any,
        Doc,
        GetString as _,
        Map as _,
        MapPrelim,
        MapRef,
        Out,
        ReadTxn as _,
        Text as _,
        Transact as _,
        XmlTextPrelim,
        types::ToJson as _,
        updates::decoder::Decode as _,
    };

    use crate::{
        domain::board_file::document::{
            Overwrite,
            decoded,
            encoded,
            import_plan,
            object_count,
            replace_plan,
            saved_objects,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    fn put(doc: &Doc, id: &str, fields: &serde_json::Value) {
        let objects: MapRef = doc.get_or_insert_map("objects");
        let value = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
        objects.insert(&mut doc.transact_mut(), id, value);
    }

    fn planned(
        target: &Doc,
        board_file: &[u8],
        overwrite: Overwrite,
    ) -> Result<crate::domain::board_file::document::ImportPlan> {
        import_plan(target, board_file, saved_objects(board_file)?, overwrite)
    }

    fn source() -> Doc {
        let doc = Doc::with_client_id(11);
        put(
            &doc,
            "s1",
            &json!({"id": "s1", "type": "sticker", "position": {"x": 1, "y": 2}}),
        );
        put(
            &doc,
            "s2",
            &json!({"id": "s2", "type": "shape", "position": {"x": 5, "y": 6}}),
        );
        doc.get_or_insert_map("objects")
            .remove(&mut doc.transact_mut(), "s2");
        doc.get_or_insert_map("task-card-statusesStore").insert(
            &mut doc.transact_mut(),
            "todo",
            "To do",
        );
        doc
    }

    #[test]
    fn a_replace_leaves_exactly_the_saved_objects() -> Result<()> {
        let target = Doc::with_client_id(22);
        put(
            &target,
            "old",
            &json!({"id": "old", "type": "sticker", "position": {"x": 0, "y": 0}}),
        );
        let server = decoded(&encoded(&target))?;
        let plan = replace_plan(&target, &encoded(&source()), 1)?;
        server
            .transact_mut()
            .apply_update(yrs::Update::decode_v2(&plan.update)?)?;
        assert_eq!(
            (plan.saved, plan.before, plan.after, object_count(&server)),
            (1, 1, 1, 1)
        );
        assert!(
            crate::domain::board::objects(&server)
                .iter()
                .all(|object| object.id != "old")
        );
        Ok(())
    }

    /// The roots a restore must make equal, one of them absent from a backup.
    type View = (Any, Any, Any, Any, String);

    fn view(doc: &Doc) -> View {
        let maps = [
            "objects",
            "documents",
            "task-card-statusesStore",
            "extraStore",
        ]
        .map(|name| doc.get_or_insert_map(name));
        let note = doc.get_or_insert_text("note");
        let txn = doc.transact();
        let [objects, documents, statuses, extra] = maps.map(|map| map.to_json(&txn));
        (objects, documents, statuses, extra, note.get_string(&txn))
    }

    /// A board of two stickers, the text of the first, a store entry and a
    /// note.
    fn board(text: &str) -> Doc {
        let doc = Doc::with_client_id(11);
        let objects = doc.get_or_insert_map("objects");
        let documents = doc.get_or_insert_map("documents");
        let statuses = doc.get_or_insert_map("task-card-statusesStore");
        let note = doc.get_or_insert_text("note");
        let mut txn = doc.transact_mut();
        let sticker = |id: &str| {
            [
                ("id".to_owned(), Any::from(id)),
                ("type".to_owned(), Any::from("sticker")),
            ]
            .into_iter()
            .collect::<MapPrelim>()
        };
        objects.insert(&mut txn, "s1", sticker("s1"));
        objects.insert(&mut txn, "s2", sticker("s2"));
        documents.insert(&mut txn, "d1", XmlTextPrelim::new(text));
        statuses.insert(&mut txn, "todo", "To do");
        note.insert(&mut txn, 0, "saved note");
        drop(txn);
        doc
    }

    /// The server's view after `live`, a copy of the backup that then changed
    /// by `edit`, is restored to the backup.
    fn restored(edit: impl FnOnce(&Doc)) -> Result<(View, View)> {
        let saved = encoded(&board("old text"));
        let live = decoded(&saved)?;
        edit(&live);
        let server = decoded(&encoded(&live))?;
        let plan = replace_plan(&live, &saved, 2)?;
        server
            .transact_mut()
            .apply_update(yrs::Update::decode_v2(&plan.update)?)?;
        Ok((view(&server), view(&decoded(&saved)?)))
    }

    /// Replaces the eight characters of the text `d1` with `text`.
    fn retext(doc: &Doc, text: &str) {
        let documents = doc.get_or_insert_map("documents");
        let mut txn = doc.transact_mut();
        if let Some(Out::YXmlText(old)) = documents.get(&txn, "d1") {
            old.remove_range(&mut txn, 0, 8);
            old.insert(&mut txn, 0, text);
        }
    }

    #[test]
    fn a_restore_returns_the_text_of_the_backup() -> Result<()> {
        let (restored, saved) = restored(|live| retext(live, "new text"))?;
        assert_eq!(restored, saved);
        Ok(())
    }

    #[test]
    fn a_restore_brings_back_a_removed_object_and_drops_a_new_one() -> Result<()> {
        let (restored, saved) = restored(|live| {
            let objects = live.get_or_insert_map("objects");
            let mut txn = live.transact_mut();
            objects.remove(&mut txn, "s1");
            objects.insert(
                &mut txn,
                "s3",
                [("id".to_owned(), Any::from("s3"))]
                    .into_iter()
                    .collect::<MapPrelim>(),
            );
        })?;
        assert_eq!(restored, saved);
        Ok(())
    }

    #[test]
    fn a_restore_drops_entries_added_to_the_stores_and_the_note() -> Result<()> {
        let (restored, saved) = restored(|live| {
            let statuses = live.get_or_insert_map("task-card-statusesStore");
            let extra = live.get_or_insert_map("extraStore");
            let note = live.get_or_insert_text("note");
            let mut txn = live.transact_mut();
            statuses.insert(&mut txn, "done", "Done");
            statuses.insert(&mut txn, "todo", "Later");
            extra.insert(&mut txn, "k", "v");
            note.insert(&mut txn, 0, "added ");
        })?;
        assert_eq!(restored, saved);
        Ok(())
    }

    #[test]
    fn an_empty_board_gets_everything() -> Result<()> {
        let doc = source();
        let target = Doc::with_client_id(22);
        let plan = planned(&target, &encoded(&doc), Overwrite::EmptyOnly)?;
        assert_eq!((plan.saved, plan.before, plan.after), (1, 0, 1));
        let server = decoded(&plan.update)?;
        assert_eq!(
            crate::domain::board::objects(&server),
            crate::domain::board::objects(&doc)
        );
        assert_eq!(
            server.transact().state_vector(),
            doc.transact().state_vector()
        );
        Ok(())
    }

    #[test]
    fn a_board_with_objects_is_refused_unless_merged() -> Result<()> {
        let saved = encoded(&source());
        let target = Doc::with_client_id(22);
        put(&target, "t1", &json!({"id": "t1", "type": "sticker"}));
        assert!(matches!(
            planned(&target, &saved, Overwrite::EmptyOnly),
            Err(Error::BoardChange(BoardChangeError::NotEmpty {
                objects: 1
            }))
        ));
        let merged = planned(&target, &saved, Overwrite::Merge)?;
        assert_eq!((merged.saved, merged.before, merged.after), (1, 1, 2));
        Ok(())
    }

    #[test]
    fn bytes_that_are_not_an_update_are_an_error() {
        assert!(decoded(b"not a board").is_err());
        assert!(planned(&Doc::new(), b"", Overwrite::Merge).is_err());
    }

    #[test]
    fn the_count_is_what_the_object_parser_lists() {
        let doc = source();
        put(&doc, "no-type", &json!({"id": "no-type"}));
        put(&doc, "no-id", &json!({"type": "sticker"}));
        put(&doc, "bad-type", &json!({"id": "bad-type", "type": 7}));
        put(&doc, "bad-id", &json!({"id": 7, "type": "shape"}));
        put(&doc, "plain", &json!("text"));
        put(&doc, "s3", &json!({"id": "s3", "type": "table"}));
        assert_eq!(
            object_count(&doc),
            crate::domain::board::objects(&doc).len()
        );
        assert_eq!(object_count(&doc), 2);
    }
}
