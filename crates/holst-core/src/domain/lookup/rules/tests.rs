use rstest::rstest;
use serde_json::json;

use crate::{
    domain::{
        account::Workspace,
        area::scene::Scene,
        board::Object,
        id::BoardId,
        lookup::{
            choice::{MatchCoverage, SkippedWorkspaces},
            rules::{FRAME_ORDER, MatchOrder, MatchRule, MatchRules, WORKSPACE_ORDER},
        },
        tree::name_index::build::NameIndex,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        lookup,
        lookup::LookupError,
    },
};

#[derive(Debug)]
struct Expected {
    one: Option<&'static str>,
    several: &'static [&'static str],
    not_found: bool,
    refused: bool,
}

const fn one(id: &'static str) -> Expected {
    Expected {
        one: Some(id),
        several: &[],
        not_found: false,
        refused: false,
    }
}

const fn several(ids: &'static [&'static str]) -> Expected {
    Expected {
        one: None,
        several: ids,
        not_found: false,
        refused: false,
    }
}

const fn not_found() -> Expected {
    Expected {
        one: None,
        several: &[],
        not_found: true,
        refused: false,
    }
}

const fn refused() -> Expected {
    Expected {
        one: None,
        several: &[],
        not_found: false,
        refused: true,
    }
}

type Lookup = fn(&str) -> Result<String>;

const WORKSPACE: &str = "00000000-0000-4000-8000-0000000000aa";

fn uuid(head: &str, tail: u32) -> String {
    format!("{head}-0000-4000-8000-{tail:012}")
}

fn workspaces() -> Vec<Workspace> {
    [("w1", "easy"), ("w2", "Design team"), ("w3", "Design")]
        .into_iter()
        .map(|(id, name)| Workspace {
            id: id.into(),
            name: name.into(),
        })
        .collect()
}

fn tree() -> Result<NameIndex> {
    let node = |head: &str, tail, kind: &str, name: &str| json!({ "id": uuid(head, tail), "type": kind, "name": name });
    let channels = json!({ "channels": [
        node("b0000001", 1, "board", "Plans"),
        node("b0000002", 2, "board", "Plans Q4"),
        node("b0000003", 3, "board", "Retro"),
        node("b0000004", 4, "board", "Retro"),
        node("f0000001", 1, "folder", "Archive"),
        node("f0000002", 2, "folder", "Archive old"),
        node("f0000003", 3, "folder", "Team"),
        node("f0000004", 4, "folder", "Team"),
    ]});
    NameIndex::from_tree_json(&channels.to_string())
}

fn objects() -> Vec<Object> {
    let frame = |id: String, title: &str| {
        json!({ "id": id, "type": "frame", "position": { "x": 0, "y": 0 },
                "width": 10, "height": 10, "labelText": title })
    };
    [
        frame(uuid("abcdef01", 1), "Plan"),
        frame(uuid("abcdef01", 3), "abcdef01 again"),
        frame(uuid("12345678", 2), "abcdef01 notes"),
        frame(uuid("c2000000", 4), "Retro"),
        frame(uuid("c3000000", 5), "Retro"),
        frame(uuid("d4000000", 6), "Weekly sync"),
        frame(uuid("d5000000", 7), "Weekly retro"),
        frame(uuid("e6000000", 9), "c3000000"),
        json!({ "id": uuid("5a000000", 8), "type": "sticker",
                "position": { "x": 0, "y": 0 }, "labelText": "Plan" }),
    ]
    .into_iter()
    .filter_map(crate::domain::board::object_of)
    .collect()
}

fn object_ids() -> Vec<String> {
    vec![
        uuid("cafe1234", 1),
        uuid("bad00000", 2),
        "10000000-1111-4000-8000-000000000003".into(),
        "10000000-1111-4000-8000-000000000004".into(),
        "abcdef01".into(),
        uuid("abcdef01", 6),
    ]
}

fn in_workspaces(reference: &str) -> Result<String> {
    crate::domain::lookup::choice::workspace(&workspaces(), Some(reference))
        .map(|workspace| workspace.id)
}

fn in_boards(reference: &str) -> Result<String> {
    let index = tree()?;
    let indexes = [(
        Workspace {
            id: WORKSPACE.into(),
            name: "easy".into(),
        },
        &index,
    )];
    let unread = SkippedWorkspaces::default();
    BoardId::find_in(reference).map_or_else(
        || {
            crate::domain::lookup::choice::one(
                &indexes,
                &unread,
                crate::domain::lookup::choice::ChoiceQuery {
                    reference,
                    filter: crate::domain::lookup::choice::BOARDS,
                    matching: MatchCoverage::Partial,
                },
            )
        },
        |id| Ok(id.into()),
    )
}

fn in_folders(reference: &str) -> Result<String> {
    let index = tree()?;
    BoardId::find_in(reference).map_or_else(
        || {
            crate::domain::lookup::choice::chosen(
                crate::domain::lookup::choice::folder(&index, reference),
                crate::domain::lookup::choice::LookupQuery {
                    what: lookup::LookupKind::Folder,
                    reference,
                },
                &SkippedWorkspaces::default(),
                |found| crate::domain::lookup::choice::listed(&index, found),
            )
        },
        |id| Ok(id.into()),
    )
}

fn in_trash(reference: &str) -> Result<String> {
    let node = |tail, kind: &str, name: &str| json!({ "id": uuid("a0000004", tail), "type": kind, "name": name });
    let index = NameIndex::of_trash(&json!([
        node(4, "board", "Old plans"),
        node(5, "folder", "Old plans 2"),
    ]));
    crate::domain::lookup::choice::chosen(
        crate::domain::lookup::choice::trashed(&index, reference),
        crate::domain::lookup::choice::LookupQuery {
            what: lookup::LookupKind::Trash,
            reference,
        },
        &SkippedWorkspaces::default(),
        |found| crate::domain::lookup::choice::listed(&index, found),
    )
}

fn in_frames(reference: &str) -> Result<String> {
    let objects = objects();
    Scene::new(&objects)
        .frame(reference)
        .map(|frame| frame.id.clone())
}

fn in_objects(reference: &str) -> Result<String> {
    let ids = object_ids();
    crate::domain::lookup::rules::find_id(ids.iter().map(String::as_str), reference)
}

fn ambiguous_candidates<T>(lookup: &Result<T>) -> String {
    match lookup {
        Err(Error::Lookup(LookupError {
            miss: lookup::LookupMiss::Ambiguous { candidates, .. },
            ..
        })) => candidates.to_string(),
        Ok(_) | Err(_) => String::new(),
    }
}

#[rstest]
#[case::workspace_exact_id(in_workspaces, MatchRule::ExactId, "W2", one("w2"))]
#[case::workspace_exact_name(in_workspaces, MatchRule::ExactName, "design", one("w3"))]
#[case::workspace_no_id_prefix(in_workspaces, MatchRule::IdPrefix, "w", not_found())]
#[case::workspace_name_part(in_workspaces, MatchRule::NamePart, "team", one("w2"))]
#[case::workspace_name_part_ambiguous(in_workspaces, MatchRule::NamePart, "es", several(&["w2", "w3"]))]
#[case::workspace_not_found(in_workspaces, MatchRule::NamePart, "budget", not_found())]
#[case::board_exact_id(
    in_boards,
    MatchRule::ExactId,
    "b0000003-0000-4000-8000-000000000003",
    one("b0000003-0000-4000-8000-000000000003")
)]
#[case::board_exact_name_hides_parts(
    in_boards,
    MatchRule::ExactName,
    "plans",
    one("b0000001-0000-4000-8000-000000000001")
)]
#[case::board_exact_name_ambiguous(in_boards, MatchRule::ExactName, "retro", several(&["b0000003-0000-4000-8000-000000000003", "b0000004-0000-4000-8000-000000000004"]))]
#[case::board_no_id_prefix(in_boards, MatchRule::IdPrefix, "b0000002", not_found())]
#[case::board_name_part(
    in_boards,
    MatchRule::NamePart,
    "q4",
    one("b0000002-0000-4000-8000-000000000002")
)]
#[case::board_name_part_ambiguous(in_boards, MatchRule::NamePart, "plan", several(&["b0000001-0000-4000-8000-000000000001", "b0000002-0000-4000-8000-000000000002"]))]
#[case::board_not_found(in_boards, MatchRule::NamePart, "budget", not_found())]
#[case::folder_exact_id(
    in_folders,
    MatchRule::ExactId,
    "f0000002-0000-4000-8000-000000000002",
    one("f0000002-0000-4000-8000-000000000002")
)]
#[case::folder_exact_name_hides_parts(
    in_folders,
    MatchRule::ExactName,
    "archive",
    one("f0000001-0000-4000-8000-000000000001")
)]
#[case::folder_exact_name_ambiguous(in_folders, MatchRule::ExactName, "team", several(&["f0000003-0000-4000-8000-000000000003", "f0000004-0000-4000-8000-000000000004"]))]
#[case::folder_no_id_prefix(in_folders, MatchRule::IdPrefix, "f0000002", not_found())]
#[case::folder_name_part(
    in_folders,
    MatchRule::NamePart,
    "old",
    one("f0000002-0000-4000-8000-000000000002")
)]
#[case::folder_name_part_ambiguous(in_folders, MatchRule::NamePart, "arch", several(&["f0000001-0000-4000-8000-000000000001", "f0000002-0000-4000-8000-000000000002"]))]
#[case::folder_not_found(in_folders, MatchRule::NamePart, "budget", not_found())]
#[case::trash_name_part(
    in_trash,
    MatchRule::NamePart,
    "plans 2",
    one("a0000004-0000-4000-8000-000000000005")
)]
#[case::trash_name_part_ambiguous(in_trash, MatchRule::NamePart, "old", several(&["a0000004-0000-4000-8000-000000000004", "a0000004-0000-4000-8000-000000000005"]))]
#[case::trash_not_found(in_trash, MatchRule::NamePart, "budget", not_found())]
#[case::frame_exact_id(
    in_frames,
    MatchRule::ExactId,
    "12345678-0000-4000-8000-000000000002",
    one("12345678-0000-4000-8000-000000000002")
)]
#[case::frame_exact_id_by_link(
    in_frames,
    MatchRule::ExactId,
    "https://app.holst.so/board/b?objectId=c2000000-0000-4000-8000-000000000004",
    one("c2000000-0000-4000-8000-000000000004")
)]
#[case::frame_exact_name_skips_other_kinds(
    in_frames,
    MatchRule::ExactName,
    "plan",
    one("abcdef01-0000-4000-8000-000000000001")
)]
#[case::frame_exact_name_hides_parts(in_frames, MatchRule::ExactName, "retro", several(&["c2000000-0000-4000-8000-000000000004", "c3000000-0000-4000-8000-000000000005"]))]
#[case::frame_id_prefix(
    in_frames,
    MatchRule::IdPrefix,
    "12345678",
    one("12345678-0000-4000-8000-000000000002")
)]
#[case::frame_id_prefix_and_titles_stand_level(in_frames, MatchRule::IdPrefix, "abcdef01", several(&["abcdef01-0000-4000-8000-000000000001", "abcdef01-0000-4000-8000-000000000003", "12345678-0000-4000-8000-000000000002"]))]
#[case::frame_whole_title_and_id_prefix_stand_level(in_frames, MatchRule::IdPrefix, "c3000000", several(&["c3000000-0000-4000-8000-000000000005", "e6000000-0000-4000-8000-000000000009"]))]
#[case::frame_short_id_start_is_no_prefix(
    in_frames,
    MatchRule::IdPrefix,
    "1234",
    not_found()
)]
#[case::frame_name_part(
    in_frames,
    MatchRule::NamePart,
    "sync",
    one("d4000000-0000-4000-8000-000000000006")
)]
#[case::frame_name_part_ambiguous(in_frames, MatchRule::NamePart, "week", several(&["d4000000-0000-4000-8000-000000000006", "d5000000-0000-4000-8000-000000000007"]))]
#[case::frame_not_found(in_frames, MatchRule::NamePart, "budget", not_found())]
#[case::object_exact_id(
    in_objects,
    MatchRule::ExactId,
    "CAFE1234-0000-4000-8000-000000000001",
    one("cafe1234-0000-4000-8000-000000000001")
)]
#[case::object_exact_id_hides_prefixes(
    in_objects,
    MatchRule::ExactId,
    "abcdef01",
    one("abcdef01")
)]
#[case::object_exact_id_by_link(
    in_objects,
    MatchRule::ExactId,
    "https://app.holst.so/board/b?objectId=bad00000-0000-4000-8000-000000000002",
    one("bad00000-0000-4000-8000-000000000002")
)]
#[case::object_id_prefix(
    in_objects,
    MatchRule::IdPrefix,
    "bad00000",
    one("bad00000-0000-4000-8000-000000000002")
)]
#[case::object_id_prefix_ambiguous(in_objects, MatchRule::IdPrefix, "10000000-1111", several(&["10000000-1111-4000-8000-000000000003", "10000000-1111-4000-8000-000000000004"]))]
#[case::object_id_prefix_after_the_at_of_the_listing(
    in_objects,
    MatchRule::IdPrefix,
    "@bad00000",
    one("bad00000-0000-4000-8000-000000000002")
)]
#[case::object_short_at_is_no_id_start(
    in_objects,
    MatchRule::IdPrefix,
    "@bad",
    refused()
)]
#[case::object_not_found(in_objects, MatchRule::IdPrefix, "deadbeef", not_found())]
#[case::object_takes_no_exact_name(
    in_objects,
    MatchRule::ExactName,
    "Plans",
    refused()
)]
#[case::object_takes_no_name_part(in_objects, MatchRule::NamePart, "plan", refused())]
fn every_kind_follows_its_match_order(
    #[case] lookup: Lookup,
    #[case] rule: MatchRule,
    #[case] reference: &str,
    #[case] expected: Expected,
) {
    let got = lookup(reference);
    let context = format!("{rule:?} from {reference:?}: {got:?}");
    let candidates = ambiguous_candidates(&got);
    assert_eq!(got.as_deref().ok(), expected.one, "{context}");
    assert_eq!(
        matches!(
            &got,
            Err(Error::Lookup(LookupError {
                miss: lookup::LookupMiss::NotFound { .. },
                ..
            }))
        ),
        expected.not_found,
        "{context}"
    );
    assert_eq!(
        matches!(
            &got,
            Err(Error::BoardChange(BoardChangeError::NoObject { .. }))
        ),
        expected.refused,
        "{context}"
    );
    assert_eq!(
        candidates
            .split(", ")
            .filter(|listed| !listed.is_empty())
            .count(),
        expected.several.len(),
        "{context}"
    );
    assert!(
        expected.several.iter().all(|id| candidates.contains(id)),
        "{context}"
    );
}

#[test]
fn a_stronger_rule_hides_only_the_weaker_ones_not_level_with_it() {
    let hidden =
        |order: &MatchOrder, rule| order.hidden_by(MatchRules::NONE.with(rule));
    let by_title = hidden(&FRAME_ORDER, MatchRule::ExactName);
    assert!(by_title.has(MatchRule::NamePart));
    assert!(!by_title.has(MatchRule::IdPrefix));
    assert!(hidden(&FRAME_ORDER, MatchRule::IdPrefix).is_empty());
    let by_id = hidden(&WORKSPACE_ORDER, MatchRule::ExactId);
    assert!(by_id.has(MatchRule::ExactName) && by_id.has(MatchRule::NamePart));
}
