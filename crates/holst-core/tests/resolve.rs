//! Finding a board by name against a local mock; nothing goes to Holst.

use holst_core::{
    Error,
    command::lookup as lookup_fetch,
    error::{login::LoginError, lookup, lookup::LookupError},
};
use rstest::rstest;
use serde_json::json;

use crate::common;

type TestResult = holst_core::Result<()>;

#[derive(Debug, PartialEq, Eq)]
enum Found<'a> {
    Board(&'a str),
    Missing(bool),
    Uncertain,
    Status(u16),
    Revoked,
}

use Found::{Board, Missing, Revoked, Status, Uncertain};

impl Found<'_> {
    fn of(found: &holst_core::Result<String>) -> Found<'_> {
        match found {
            Ok(id) => Board(id),
            Err(Error::Lookup(LookupError {
                miss: lookup::LookupMiss::NotFound { unread, .. },
                ..
            })) => Missing(!unread.is_empty()),
            Err(Error::Lookup(LookupError {
                miss: lookup::LookupMiss::Uncertain { .. },
                ..
            })) => Uncertain,
            Err(Error::Login(LoginError::SessionRevoked)) => Revoked,
            Err(err) => Status(err.status().unwrap_or_default()),
        }
    }
}

fn tree(boards: &[(&str, &str)]) -> String {
    let channels: Vec<_> = boards
        .iter()
        .map(|(id, name)| json!({ "id": id, "type": "board", "name": name, "parent_id": "w" }))
        .collect();
    json!({ "workspace": {}, "spaces": [], "channels": channels }).to_string()
}

/// What the first workspace's tree endpoint answers with.
struct Tree {
    status: u16,
    body: String,
}

impl Tree {
    fn answering(status: u16, boards: &[(&str, &str)]) -> Self {
        Self {
            status,
            body: match status {
                200 => tree(boards),
                _ => "{}".into(),
            },
        }
    }
}

const PLANS: &[(&str, &str)] = &[("b1", "Plans")];

#[rstest]
#[case::exact_name_wins_across_workspaces(Tree::answering(200, &[("b1", "Plans Q4")]), &[("b2", "plans")], None, Board("b2"))]
#[case::not_found_mentions_unreadable_workspaces(Tree::answering(403, &[]), &[("b1", "Other")], None, Missing(true))]
#[case::not_found_when_everything_was_read(Tree::answering(200, &[]), &[("b1", "Other")], None, Missing(false))]
// A server error elsewhere: another board could be there.
#[case::server_error_makes_single_match_ambiguous(Tree::answering(500, &[]), PLANS, None, Uncertain)]
#[case::bad_request_makes_single_match_ambiguous(Tree::answering(400, &[]), PLANS, None, Uncertain)]
// A workspace without access (403/404) is skipped: nothing there can be changed anyway.
#[case::forbidden_workspace_is_skipped(Tree::answering(403, &[]), PLANS, None, Board("b1"))]
#[case::missing_workspace_is_skipped(Tree::answering(404, &[]), PLANS, None, Board("b1"))]
#[case::explicit_workspace_skips_server_error(Tree::answering(500, &[]), PLANS, Some("easy"), Board("b1"))]
#[case::explicit_workspace_skips_forbidden(Tree::answering(403, &[]), PLANS, Some("easy"), Board("b1"))]
#[case::revoked_session_stops_search(Tree::answering(401, &[]), PLANS, None, Revoked)]
#[tokio::test]
async fn resolves_board_by_name(
    #[case] w1: Tree,
    #[case] w2: &[(&str, &str)],
    #[case] workspace: Option<&str>,
    #[case] expected: Found<'_>,
) -> TestResult {
    let mock = common::routes(vec![
        common::current_user(),
        ("/gapi/workspace/w1/tree", w1.status, w1.body),
        ("/gapi/workspace/w2/tree", 200, tree(w2)),
    ])
    .await?;
    let found = lookup_fetch::board_id(&mock.client("tok")?, "Plans", workspace).await;
    assert_eq!(Found::of(&found), expected, "{found:?}");
    Ok(())
}

#[rstest]
#[case::bare_id("0b3e2c4a-1f2d-4e5f-9a8b-7c6d5e4f3a2b")]
#[case::link("https://app.holst.so/board/0b3e2c4a-1f2d-4e5f-9a8b-7c6d5e4f3a2b?x=1")]
#[tokio::test]
async fn id_or_link_needs_no_request(#[case] spec: &str) -> TestResult {
    let mock = common::routes(vec![]).await?;
    let id = lookup_fetch::board_id(&mock.client("tok")?, spec, None).await?;
    assert_eq!(id, "0b3e2c4a-1f2d-4e5f-9a8b-7c6d5e4f3a2b");
    assert_eq!(mock.hits(), 0);
    Ok(())
}
