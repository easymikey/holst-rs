#![expect(
    clippy::panic_in_result_fn,
    reason = "assertions are the point of a test"
)]

use rstest::rstest;
use serde_json::{Value, json};

use crate::{
    domain::{
        account::account_workspaces,
        lookup::choice::{
            BOARDS,
            Choice,
            ChoiceQuery,
            FOLDERS,
            FoundNode,
            LISTED,
            MatchCoverage,
            NODES,
            ParentReference,
            SkippedWorkspaces,
            TRASH,
            Workspace,
            choose,
            folder,
            matches,
            one,
            parent,
            partition_reads,
            root_in,
            trashed,
            workspace,
        },
        node::Parent,
        tree::name_index::{build::NameIndex, query::NodeFilter},
    },
    error::{
        Error,
        Result,
        http::HttpError,
        input::InputError,
        login::LoginError,
        lookup,
        lookup::LookupError,
    },
};

fn named(id: &str, name: &str) -> Workspace {
    Workspace {
        id: id.into(),
        name: name.into(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Expected {
    Picked(String),
    Fails(String),
}

fn picked(id: &str) -> Expected {
    Expected::Picked(id.to_owned())
}

fn fails(text: &str) -> Expected {
    Expected::Fails(text.to_owned())
}

impl From<Result<String>> for Expected {
    fn from(result: Result<String>) -> Self {
        result.map_or_else(|error| Expected::Fails(error.to_string()), Expected::Picked)
    }
}

#[rstest]
#[case::merged_from_both_lists(
        json!({
            "workspace_users": [{ "workspace_id": "w1" }],
            "workspaces": [{ "id": "w1", "name": "easy" }],
        }),
        &[("w1", "easy")]
    )]
#[case::nested_name(
        json!({ "workspaceUsers": [{ "workspaceId": "w1", "workspace": { "name": "easy" } }] }),
        &[("w1", "easy")]
    )]
#[case::no_lists(json!({ "id": "u1" }), &[])]
#[case::item_without_id(json!({ "workspaces": [{ "name": "ghost" }] }), &[])]
fn workspaces_are_read_from_current_user(
    #[case] me: Value,
    #[case] expected: &[(&str, &str)],
) {
    let expected: Vec<_> = expected.iter().map(|&(id, name)| named(id, name)).collect();
    assert_eq!(account_workspaces(&me), expected);
}

const EASY_DESIGN: &[(&str, &str)] = &[("w1", "easy"), ("w2", "Design team")];

#[rstest]
#[case::single_is_default(&[("w1", "easy")], None, picked("w1"))]
#[case::blank_spec_is_no_spec(&[("w1", "easy")], Some("  "), picked("w1"))]
#[case::none_at_all(&[], None, fails("no workspaces found"))]
#[case::several_need_a_choice(
    EASY_DESIGN,
    None,
    fails("there are several workspaces; choose one: easy (w1), Design team (w2)")
)]
#[case::by_id(EASY_DESIGN, Some("w2"), picked("w2"))]
#[case::by_id_in_capitals(EASY_DESIGN, Some("W2"), picked("w2"))]
#[case::by_link(
        &[("ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b", "easy"), ("w2", "Design team")],
        Some("https://app.holst.so/workspace/ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b/boards"),
        picked("ae1f0a2b-c3d4-4f19-8a7b-1c2d3e4f5a6b")
    )]
#[case::by_partial_name(EASY_DESIGN, Some("design"), picked("w2"))]
#[case::ambiguous_name(
        &[("w1", "team alpha"), ("w2", "team beta")],
        Some("team"),
        fails("\"team\" matches several workspaces: team alpha (w1), team beta (w2)")
    )]
#[case::exact_beats_partial(&[("w1", "team"), ("w2", "team beta")], Some("Team"), picked("w1"))]
#[case::unknown_lists_existing(
        &[("w1", "easy")],
        Some("nope"),
        fails("workspace \"nope\" not found. Available: easy (w1)")
    )]
fn workspace_cases(
    #[case] all: &[(&str, &str)],
    #[case] reference: Option<&str>,
    #[case] expected: Expected,
) {
    let all: Vec<_> = all.iter().map(|&(id, name)| named(id, name)).collect();
    let got = Expected::from(workspace(&all, reference).map(|chosen| chosen.id));
    assert_eq!(got, expected);
}

fn index(names: &[&str]) -> Result<NameIndex> {
    let channels: Vec<_> = names
        .iter()
        .zip(1..)
        .map(|(name, number)| {
            let id = format!("00000001-0000-4000-8000-{number:012x}");
            json!({ "id": id, "type": "board", "name": name, "parent_id": "w" })
        })
        .collect();
    NameIndex::from_tree_json(&json!({ "channels": channels }).to_string())
}

#[rstest]
#[case::words_in_any_order("планы итоги", &[])]
#[case::phrase("планы и итоги", &["Ретро: планы и итоги"])]
#[case::substring("ланы", &["Планы Q4", "Ретро: планы и итоги"])]
fn candidates_ignore_words_in_any_order(
    #[case] reference: &str,
    #[case] expected: &[&str],
) -> Result<()> {
    let index = index(&["Ретро: планы и итоги", "Планы Q4"])?;
    let found = matches(&[&index], reference, BOARDS, MatchCoverage::Partial);
    let names: Vec<_> = found.iter().map(|found| index.name(found.node)).collect();
    assert_eq!(names, expected);
    Ok(())
}

fn api_error(status: u16) -> Error {
    Error::Http(HttpError::Status {
        method: "GET",
        path: "/t".into(),
        status,
        detail: String::new(),
    })
}

fn board(
    indexes: &[(Workspace, &NameIndex)],
    unread: &SkippedWorkspaces,
    reference: &str,
) -> Result<String> {
    one(indexes, unread, partial(BOARDS, reference))
}

const fn partial(filter: NodeFilter, reference: &str) -> ChoiceQuery<'_> {
    ChoiceQuery {
        reference,
        filter,
        matching: MatchCoverage::Partial,
    }
}

const fn exact(filter: NodeFilter, reference: &str) -> ChoiceQuery<'_> {
    ChoiceQuery {
        reference,
        filter,
        matching: MatchCoverage::Exact,
    }
}

/// A lookup inside one workspace's index and the position of that index.
#[derive(Clone, Copy)]
struct Within {
    lookup: fn(&NameIndex, &str) -> Choice,
    position: usize,
}

const LIVE_FOLDERS: Within = Within {
    lookup: folder,
    position: 0,
};

const TRASH_ITEMS: Within = Within {
    lookup: trashed,
    position: 1,
};

fn unread(failed: &[u16]) -> SkippedWorkspaces {
    failed
        .iter()
        .fold(SkippedWorkspaces::default(), |unread, &status| {
            unread.with(&named("w2", "team"), &api_error(status))
        })
}

fn live_and_trash() -> Result<[(Workspace, NameIndex); 2]> {
    let id = |number: u32| format!("00000001-0000-4000-8000-{number:012x}");
    let node = |number, kind: &str, name: &str, parent: Option<u32>| {
        let parent_id = parent.map(id);
        json!({ "id": id(number), "type": kind, "name": name, "parent_id": parent_id })
    };
    let live = json!({ "channels": [
        node(1, "folder", "Plans", None),
        node(2, "folder", "Plans archive", None),
        node(3, "board", "Plans Q4", Some(1)),
        node(4, "board", "Plans Q1", Some(1)),
        node(5, "board", "Retro", None),
        node(6, "board", "Retro", Some(2)),
    ]});
    let trash = json!({ "channels": [
        node(7, "board", "Old plans", None),
        node(8, "folder", "Old retro", None),
        node(9, "board", "Old retro notes", Some(8)),
    ]});
    Ok([
        (
            named("w1", "easy"),
            NameIndex::from_tree_json(&live.to_string())?,
        ),
        (named("w1", "easy"), NameIndex::of_trash(&trash)),
    ])
}

fn seen(
    choice: &Choice,
    indexes: &[(Workspace, NameIndex)],
) -> (&'static str, Vec<String>) {
    let name = |found: &FoundNode| {
        indexes
            .get(found.workspace)
            .map(|(_, index)| index.name(found.node).to_owned())
            .unwrap_or_default()
    };
    let chosen = indexes.iter().flat_map(|(_, index)| {
        index
            .nodes()
            .filter(|&node| Choice::Chosen(index.id(node)) == *choice)
            .map(|node| index.name(node).to_owned())
    });
    let listed: &[FoundNode] = match choice {
        Choice::Uncertain(found) => std::slice::from_ref(found),
        Choice::Ambiguous(found) => found,
        Choice::Chosen(_) | Choice::NotFound => &[],
    };
    let names = listed.iter().map(name).chain(chosen.take(1)).collect();
    let kind = [
        ("chosen", matches!(choice, Choice::Chosen(_))),
        ("uncertain", matches!(choice, Choice::Uncertain(_))),
        ("not found", matches!(choice, Choice::NotFound)),
        ("ambiguous", matches!(choice, Choice::Ambiguous(_))),
    ]
    .into_iter()
    .find(|(_, holds)| *holds)
    .map_or("", |(kind, _)| kind);
    (kind, names)
}

#[rstest]
#[case::board_partial_one(partial(BOARDS, "q4"), &[], "chosen", &["Plans Q4"])]
#[case::board_partial_none(partial(BOARDS, "budget"), &[], "not found", &[])]
#[case::board_partial_many(partial(BOARDS, "plans"), &[], "ambiguous", &["Plans Q4", "Plans Q1"])]
#[case::board_partial_uncertain(partial(BOARDS, "q4"), &[500], "uncertain", &["Plans Q4"])]
#[case::board_partial_denied_is_read(partial(BOARDS, "q4"), &[403], "chosen", &["Plans Q4"])]
#[case::board_many_stay_ambiguous_when_uncertain(partial(BOARDS, "plans"), &[500], "ambiguous", &["Plans Q4", "Plans Q1"])]
#[case::board_exact_one(exact(BOARDS, "PLANS q4"), &[], "chosen", &["Plans Q4"])]
#[case::board_exact_part_is_none(exact(BOARDS, "q4"), &[], "not found", &[])]
#[case::board_exact_many(exact(BOARDS, "retro"), &[], "ambiguous", &["Retro", "Retro"])]
#[case::board_exact_uncertain(exact(BOARDS, "plans q4"), &[500], "uncertain", &["Plans Q4"])]
#[case::folder_whole_name_hides_parts(partial(FOLDERS, "plans"), &[], "chosen", &["Plans"])]
#[case::folder_partial_one(partial(FOLDERS, "archive"), &[], "chosen", &["Plans archive"])]
#[case::folder_ignores_boards(partial(FOLDERS, "q4"), &[], "not found", &[])]
#[case::folder_partial_many(partial(FOLDERS, "pla"), &[], "ambiguous", &["Plans", "Plans archive"])]
#[case::folder_partial_uncertain(partial(FOLDERS, "archive"), &[500], "uncertain", &["Plans archive"])]
#[case::folder_exact_part_is_none(exact(FOLDERS, "archive"), &[], "not found", &[])]
#[case::node_takes_a_folder(partial(NODES, "archive"), &[], "chosen", &["Plans archive"])]
#[case::node_whole_folder_name_beside_board_parts_is_ambiguous(partial(NODES, "plans"), &[], "ambiguous", &["Plans", "Plans Q4", "Plans Q1"])]
#[case::node_whole_name_hides_parts_of_its_kind(partial(NODES, "plans archive"), &[], "chosen", &["Plans archive"])]
#[case::node_whole_folder_name_beats_board_parts(exact(NODES, "plans"), &[], "chosen", &["Plans"])]
#[case::trash_partial_one(partial(TRASH, "notes"), &[], "chosen", &["Old retro notes"])]
#[case::trash_partial_none(partial(TRASH, "q4"), &[], "not found", &[])]
#[case::trash_partial_many(partial(TRASH, "old"), &[], "ambiguous", &["Old plans", "Old retro notes", "Old retro"])]
#[case::trash_partial_uncertain(partial(TRASH, "notes"), &[500], "uncertain", &["Old retro notes"])]
#[case::trash_exact_one(exact(TRASH, "old retro"), &[], "chosen", &["Old retro"])]
fn lookup_table(
    #[case] search: ChoiceQuery<'_>,
    #[case] failed: &[u16],
    #[case] kind: &str,
    #[case] names: &[&str],
) -> Result<()> {
    let indexes = live_and_trash()?;
    let choice = choose(&indexes, &unread(failed), search);
    let (got_kind, got_names) = seen(&choice, &indexes);
    assert_eq!(
        (got_kind, got_names),
        (kind, names.iter().map(|&name| name.to_owned()).collect())
    );
    Ok(())
}

#[test]
fn root_is_ambiguous_beside_a_folder_called_root() -> Result<()> {
    let tree = |name: &str| {
        NameIndex::from_tree_json(
                &json!({ "channels": [
                    { "id": "00000001-0000-4000-8000-0000000000f1", "type": "folder", "name": name },
                ]})
                .to_string(),
            )
    };
    assert_eq!(
        root_in(&tree("Plans")?, "w1", "root")?,
        Parent::Workspace("w1".into())
    );
    let both = root_in(&tree("Root")?, "w1", "root");
    assert!(
        matches!(&both, Err(Error::Input(InputError::RootOrFolder { .. }))),
        "{both:?}"
    );
    assert!(
        both.as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default()
            .contains("Root (00000001-0000-4000-8000-0000000000f1)"),
        "{both:?}"
    );
    let text = both
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(text.contains("give / or no folder"), "{text}");
    Ok(())
}

#[test]
fn ambiguous_names_the_skipped_workspaces() -> Result<()> {
    let easy = index(&["Plans Q4", "Plans Q1"])?;
    let got = board(&[(named("w1", "easy"), &easy)], &unread(&[500]), "plans");
    let text = got.err().map(|error| error.to_string()).unwrap_or_default();
    assert!(
        text.contains("matches several boards")
            && text.contains("could not read: team"),
        "{text}"
    );
    Ok(())
}

#[rstest]
#[case::folder_one(LIVE_FOLDERS, "archive", "chosen", &["Plans archive"])]
#[case::folder_none(LIVE_FOLDERS, "q4", "not found", &[])]
#[case::folder_many(LIVE_FOLDERS, "pla", "ambiguous", &["Plans", "Plans archive"])]
#[case::trash_one(TRASH_ITEMS, "plans", "chosen", &["Old plans"])]
#[case::trash_none(TRASH_ITEMS, "retro q", "not found", &[])]
#[case::trash_many(TRASH_ITEMS, "retro", "ambiguous", &["Old retro notes", "Old retro"])]
fn within_one_workspace(
    #[case] within: Within,
    #[case] reference: &str,
    #[case] kind: &str,
    #[case] names: &[&str],
) -> Result<()> {
    let indexes: Vec<_> = live_and_trash()?
        .into_iter()
        .skip(within.position)
        .take(1)
        .collect();
    let choice = indexes.first().map_or(Choice::NotFound, |(_, index)| {
        (within.lookup)(index, reference)
    });
    let (got_kind, got_names) = seen(&choice, &indexes);
    assert_eq!(
        (got_kind, got_names),
        (kind, names.iter().map(|&name| name.to_owned()).collect())
    );
    Ok(())
}

#[rstest]
#[case::nothing(None, ParentReference::Known(Parent::Workspace("w1".into())))]
#[case::blank(Some("  "), ParentReference::Known(Parent::Workspace("w1".into())))]
#[case::root_in_capitals(Some("ROOT"), ParentReference::Root("ROOT"))]
#[case::folder_id(
        Some("0000000A-0000-4000-8000-00000000ABCD"),
        ParentReference::Known(Parent::Folder("0000000a-0000-4000-8000-00000000abcd".into()))
    )]
#[case::name(Some(" Plans "), ParentReference::Named("Plans"))]
fn parent_specs(
    #[case] reference: Option<&str>,
    #[case] expected: ParentReference<'_>,
) {
    assert_eq!(parent("w1", reference), expected);
}

#[test]
fn the_workspace_id_is_its_root() {
    let workspace = "0000000b-0000-4000-8000-00000000abcd";
    assert_eq!(
        parent(workspace, Some(&workspace.to_ascii_uppercase())),
        ParentReference::Known(Parent::Workspace(workspace.into()))
    );
}

#[rstest]
#[case::single("Планы Q4", &[], picked("00000001-0000-4000-8000-000000000002"))]
#[case::single_but_uncertain(
        "Планы Q4",
        &[500],
        fails("board \"Планы Q4\" matched easy/Планы Q4 (00000001-0000-4000-8000-000000000002): \
             not every workspace could be read (team: HTTP 500), so the match may be \
             ambiguous")
    )]
#[case::denied_does_not_block("Планы Q4", &[403], picked("00000001-0000-4000-8000-000000000002"))]
#[case::gone_does_not_block("Планы Q4", &[404], picked("00000001-0000-4000-8000-000000000002"))]
#[case::not_found("нет", &[], fails("board \"нет\" not found"))]
#[case::not_found_lists_unread(
        "нет",
        &[403],
        fails("board \"нет\" not found; could not read: team: HTTP 403")
    )]
#[case::ambiguous(
        "ланы",
        &[],
        fails("\"ланы\" matches several boards: easy/Планы Q4 \
             (00000001-0000-4000-8000-000000000002), easy/Ретро: планы и итоги \
             (00000001-0000-4000-8000-000000000001)")
    )]
fn board_cases(
    #[case] reference: &str,
    #[case] failed: &[u16],
    #[case] expected: Expected,
) -> Result<()> {
    let index = index(&["Ретро: планы и итоги", "Планы Q4"])?;
    let got = Expected::from(board(
        &[(named("w1", "easy"), &index)],
        &unread(failed),
        reference,
    ));
    assert_eq!(got, expected);
    Ok(())
}

#[test]
fn an_exact_board_needs_the_whole_name() -> Result<()> {
    let index = index(&["Ретро: планы и итоги", "Планы Q4"])?;
    let indexes = [(named("w1", "easy"), &index)];
    let one_exact = |reference| {
        one(
            &indexes,
            &SkippedWorkspaces::default(),
            exact(BOARDS, reference),
        )
    };
    assert_eq!(
        one_exact("планы q4").ok().as_deref(),
        Some("00000001-0000-4000-8000-000000000002")
    );
    assert!(matches!(
        one_exact("Планы"),
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Board,
            miss: lookup::LookupMiss::NotFound { .. },
            ..
        }))
    ));
    assert!(matches!(
        one_exact("Ретро"),
        Err(Error::Lookup(LookupError {
            kind: lookup::LookupKind::Board,
            miss: lookup::LookupMiss::NotFound { .. },
            ..
        }))
    ));
    Ok(())
}

#[test]
fn same_name_in_two_workspaces_is_ambiguous() -> Result<()> {
    let easy = index(&["Plans"])?;
    let team = index(&["Plans"])?;
    let got = board(
        &[(named("w1", "easy"), &easy), (named("w2", "team"), &team)],
        &SkippedWorkspaces::default(),
        "Plans",
    );
    assert!(
        matches!(
            &got,
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Board,
                miss: lookup::LookupMiss::Ambiguous { .. },
                ..
            }))
        ),
        "{got:?}"
    );
    let shown = got
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        shown.contains("easy/Plans") && shown.contains("team/Plans"),
        "{shown}"
    );
    Ok(())
}

#[test]
fn ambiguous_boards_in_folders_name_the_folder() -> Result<()> {
    let tree = json!({ "channels": [
        { "id": "00000001-0000-4000-8000-0000000000f1", "type": "folder", "name": "Q1" },
        { "id": "00000001-0000-4000-8000-0000000000f2", "type": "folder", "name": "Q2" },
        { "id": "00000001-0000-4000-8000-000000000001", "type": "board", "name": "Plans",
          "parent_id": "00000001-0000-4000-8000-0000000000f1" },
        { "id": "00000001-0000-4000-8000-000000000002", "type": "board", "name": "Plans",
          "parent_id": "00000001-0000-4000-8000-0000000000f2" },
        { "id": "00000001-0000-4000-8000-000000000003", "type": "board", "name": "Plans" },
    ]});
    let index = NameIndex::from_tree_json(&tree.to_string())?;
    let got = board(
        &[(named("w1", "easy"), &index)],
        &SkippedWorkspaces::default(),
        "Plans",
    );
    assert!(
        matches!(
            &got,
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Board,
                miss: lookup::LookupMiss::Ambiguous { .. },
                ..
            }))
        ),
        "{got:?}"
    );
    let shown = got
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        shown.contains("easy/Q1/Plans (")
            && shown.contains("easy/Q2/Plans (")
            && shown.contains("easy/Plans ("),
        "{shown}"
    );
    Ok(())
}

#[test]
fn ambiguous_list_is_capped() -> Result<()> {
    let names: Vec<String> = (0..25).map(|number| format!("Plan {number}")).collect();
    let many = index(&names.iter().map(String::as_str).collect::<Vec<_>>())?;
    let got = board(
        &[(named("w1", "easy"), &many)],
        &SkippedWorkspaces::default(),
        "plan",
    );
    assert!(
        matches!(
            &got,
            Err(Error::Lookup(LookupError {
                kind: lookup::LookupKind::Board,
                miss: lookup::LookupMiss::Ambiguous { .. },
                ..
            }))
        ),
        "{got:?}"
    );
    let shown = got
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert_eq!(shown.matches("easy/").count(), LISTED, "{shown}");
    Ok(())
}

#[test]
fn fatal_error_stops_the_partition() {
    let fatal = vec![
        (named("w1", "a"), Ok(1)),
        (named("w2", "b"), Err(api_error(403))),
        (named("w3", "c"), Err(api_error(401))),
    ];
    assert!(matches!(
        partition_reads(fatal),
        Err(Error::Http(HttpError::Status { status: 401, .. }))
    ));
    let denied = vec![
        (named("w1", "a"), Ok(1)),
        (named("w2", "b"), Err(api_error(403))),
    ];
    let split =
        partition_reads(denied).map(|(ok, unread)| (ok.len(), unread.is_empty()));
    assert_eq!(split.ok(), Some((1, false)));
}

#[test]
fn revoked_session_stops_the_partition_too() {
    let results: Vec<(Workspace, Result<u8>)> = vec![(
        named("w1", "a"),
        Err(Error::Login(LoginError::SessionRevoked)),
    )];
    assert!(matches!(
        partition_reads(results),
        Err(Error::Login(LoginError::SessionRevoked))
    ));
}
