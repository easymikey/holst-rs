//! Tool calls against a scripted network.

#![expect(
    clippy::disallowed_methods,
    reason = "tests build and read real files in temp directories"
)]

use holst_core::command::{
    backup::BackupVerb,
    board::BoardVerb,
    object::ObjectVerb,
    table::CommandName,
    template::TemplateVerb,
};
use rmcp::model::CallToolResult;

use crate::{
    login::fake::CannedResponse,
    server::McpServer,
    tools::files::SecretPaths,
};

const WS: &str = "00000002-0000-4000-8000-000000000001";
const BOARD: &str = "00000001-0000-4000-8000-000000000001";
const ME: &str = r#"{"id":"00000003-0000-4000-8000-000000000001","name":"Me",
    "workspaces":[{"id":"00000002-0000-4000-8000-000000000001","name":"easy"}]}"#;
#[derive(Debug, PartialEq, Eq)]
enum Shown {
    Reply(String),
    Failure(String),
}

impl Shown {
    fn into_text(self) -> String {
        match self {
            Shown::Reply(text) | Shown::Failure(text) => text,
        }
    }

    fn failure(&self) -> Option<&str> {
        match self {
            Shown::Failure(text) => Some(text),
            Shown::Reply(_) => None,
        }
    }
}

fn text(result: &CallToolResult) -> Shown {
    let text = result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .map(|block| block.text.as_str())
        .collect();
    match result.is_error {
        Some(true) => Shown::Failure(text),
        Some(false) | None => Shown::Reply(text),
    }
}

const Q4_TREE: &str = r#"{"channels":[
    {"id":"00000004-0000-4000-8000-000000000001","type":"folder","name":"Q4 plans"},
    {"id":"00000001-0000-4000-8000-000000000001","type":"board","name":"Plans",
     "parent_id":"00000004-0000-4000-8000-000000000001"},
    {"id":"00000001-0000-4000-8000-000000000003","type":"board","name":"Retro",
     "parent_id":"00000004-0000-4000-8000-000000000001"}]}"#;

const FOLDER: &str = "00000004-0000-4000-8000-000000000001";
const Q4_FOLDER: &str = r#"{"id":"00000004-0000-4000-8000-000000000001",
    "name":"Q4 plans","type":"folder","workspace_id":"00000002-0000-4000-8000-000000000001",
    "parent_id":"00000002-0000-4000-8000-000000000001","parent_type":"workspace"}"#;

#[tokio::test]
async fn search_in_trash_reads_the_trash() {
    let trash = r#"{"channels":[{"id":"00000001-0000-4000-8000-000000000009",
        "type":"board","name":"Old plans","in_trash":"2026-10-21"}]}"#;
    let (holst, network) = McpServer::scripted([
        CannedResponse::ok(ME),
        CannedResponse::ok(r#"{"channels":[]}"#),
        CannedResponse::ok(trash),
    ]);
    let arguments = serde_json::json!({
        "query": "old", "mode": "names", "kind": "any", "trash": true
    });
    let reply = text(
        &holst
            .command(
                CommandName::Search,
                arguments.as_object().cloned().unwrap_or_default(),
            )
            .await,
    )
    .into_text();
    assert!(reply.contains("Old plans"), "{reply}");
    assert_eq!(
        network.seen().get(2),
        Some(&format!("GET /gapi/workspace/{WS}/trash"))
    );
}

async fn object_call(verb: ObjectVerb, arguments: serde_json::Value) -> Shown {
    let (holst, _) = McpServer::scripted([]);
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    text(&holst.command(CommandName::Object(verb), arguments).await)
}

#[tokio::test]
async fn more_than_a_hundred_items_are_refused_before_any_request() {
    let items: Vec<_> = (0..101).map(|n| serde_json::json!({ "text": n })).collect();
    let shown = object_call(
        ObjectVerb::Add,
        serde_json::json!({ "board": BOARD, "items": items }),
    )
    .await;
    assert!(
        shown
            .failure()
            .is_some_and(|text| text.contains("at most 100 items per call, not 101")),
        "{shown:?}"
    );
}

#[tokio::test]
async fn an_upload_from_a_secret_place_is_refused_before_login() {
    let dir = empty_downloads("secret-upload");
    let session = dir.join("session.json");
    let made =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&session, "{}"));
    let path = session.to_string_lossy().into_owned();
    let on_call = serde_json::json!({ "board": BOARD, "kind": "file", "path": path });
    let on_item = serde_json::json!({ "board": BOARD,
        "items": [{ "kind": "link", "url": "https://example.com" },
                  { "kind": "image", "path": path }] });
    let secret_upload = async |call: serde_json::Value| {
        let (mut holst, network) = McpServer::scripted([]);
        holst.secrets = SecretPaths::new(Some(&session), None);
        let arguments = call.as_object().cloned().unwrap_or_default();
        let result = holst
            .command(CommandName::Object(ObjectVerb::Add), arguments)
            .await;
        (text(&result), network.seen())
    };
    let (by_call, by_item) =
        (secret_upload(on_call).await, secret_upload(on_item).await);
    drop(std::fs::remove_dir_all(&dir));
    assert!(made.is_ok(), "{made:?}");
    let refused = |(reply, seen): &(Shown, Vec<String>)| {
        seen.is_empty()
            && reply
                .failure()
                .is_some_and(|text| text.contains("is a secret; uploading it"))
    };
    assert!(
        refused(&by_call) && refused(&by_item),
        "{by_call:?} {by_item:?}"
    );
}

#[tokio::test]
async fn an_original_is_never_sent_to_standard_output() {
    let shown = object_call(
        ObjectVerb::Get,
        serde_json::json!({ "board": BOARD, "object": ["a1b2c3d4"], "file": "-" }),
    )
    .await;
    assert!(
        shown
            .failure()
            .is_some_and(|text| text.contains("no standard output")),
        "{shown:?}"
    );
}

async fn board_call(
    holst: &McpServer,
    verb: BoardVerb,
    arguments: serde_json::Value,
) -> Shown {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    text(&holst.command(CommandName::Board(verb), arguments).await)
}

#[tokio::test]
async fn board_ls_lists_the_tree_with_aliases() {
    let (holst, _) =
        McpServer::scripted([CannedResponse::ok(ME), CannedResponse::ok(Q4_TREE)]);
    let shown = board_call(&holst, BoardVerb::Ls, serde_json::json!({})).await;
    let alias = |id| holst.state.alias(id);
    assert_eq!(
        shown,
        Shown::Reply(format!(
            "Q4 plans/ {}\n  Plans {}\n  Retro {}\n",
            alias(FOLDER),
            alias(BOARD),
            alias("00000001-0000-4000-8000-000000000003")
        ))
    );
}

#[tokio::test]
async fn a_folder_with_boards_moves_only_with_confirm() {
    let (holst, network) = McpServer::scripted([
        CannedResponse::ok(Q4_FOLDER),
        CannedResponse::ok(Q4_TREE),
    ]);
    let arguments = serde_json::json!({ "board": FOLDER, "folder": "/" });
    let held = board_call(&holst, BoardVerb::Mv, arguments).await;
    assert!(
        held.failure()
            .is_some_and(|text| text.contains("confirm: true")),
        "{held:?}"
    );
    assert!(
        network
            .seen()
            .iter()
            .all(|request| request.starts_with("GET ")),
        "{:?}",
        network.seen()
    );
}

#[tokio::test]
async fn a_dry_move_names_the_request_and_an_alias_stands_for_the_folder() {
    let (holst, network) = McpServer::scripted([
        CannedResponse::ok(Q4_FOLDER),
        CannedResponse::ok(Q4_TREE),
    ]);
    let alias = holst.state.alias(FOLDER);
    let arguments =
        serde_json::json!({ "board": alias, "folder": "/", "dry_run": true });
    let shown = board_call(&holst, BoardVerb::Mv, arguments)
        .await
        .into_text();
    assert!(
        shown.starts_with(&format!(
            "would move Q4 plans, with 2 boards\nPOST /gapi/channel/{alias}/move\n"
        )),
        "{shown}"
    );
    assert_eq!(
        network.seen().first(),
        Some(&format!("GET /gapi/channel/{FOLDER}"))
    );
}

#[tokio::test]
#[expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
async fn a_second_search_costs_no_request() -> holst_core::Result<()> {
    let tree = r#"{"channels":[{"id":"00000001-0000-4000-8000-000000000009",
        "type":"board","name":"Plans"}]}"#;
    let (holst, network) =
        McpServer::scripted_kept([CannedResponse::ok(ME), CannedResponse::ok(tree)])?;
    let search = async || {
        let arguments = serde_json::json!({ "query": "plans", "mode": "names" });
        let arguments = arguments.as_object().cloned().unwrap_or_default();
        text(&holst.command(CommandName::Search, arguments).await).into_text()
    };
    let replies = [search().await, search().await];
    assert!(
        replies.iter().all(|reply| reply.contains("Plans")),
        "{replies:?}"
    );
    assert_eq!(network.seen().len(), 2, "{:?}", network.seen());
    Ok(())
}

/// A new empty folder of this process for a download to go into.
fn empty_downloads(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("holst-mcp-{}-{name}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    dir
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[tokio::test]
async fn a_backup_without_a_file_goes_to_the_downloads_folder() {
    let dir = empty_downloads("backup");
    let (mut holst, _) = McpServer::scripted([CannedResponse::ok("snapshot")]);
    holst.downloads = Some(dir.clone());
    let arguments = serde_json::json!({ "key": "k" });
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let shown = text(
        &holst
            .command(CommandName::Backup(BackupVerb::Get), arguments)
            .await,
    );
    let saved = files_in(&dir);
    drop(std::fs::remove_dir_all(&dir));
    assert!(shown.failure().is_none(), "{shown:?}");
    assert_eq!(saved, ["backup-k.ydoc.gz"], "{shown:?}");
}

#[rstest::rstest]
#[case::export("export", BoardVerb::Export, "file: the new file")]
#[case::save("save", BoardVerb::Save, "dir: the directory")]
#[tokio::test]
async fn a_save_or_export_without_a_place_goes_on_to_the_board(
    #[case] label: &str,
    #[case] verb: BoardVerb,
    #[case] complaint: &str,
) {
    let dir = empty_downloads(label);
    let (mut holst, network) = McpServer::scripted([CannedResponse::ok(Q4_FOLDER)]);
    holst.downloads = Some(dir.clone());
    let arguments = serde_json::json!({ "board": BOARD });
    let shown = board_call(&holst, verb, arguments).await;
    drop(std::fs::remove_dir_all(&dir));
    assert!(
        shown
            .failure()
            .is_none_or(|reason| !reason.contains(complaint)),
        "{shown:?}"
    );
    assert!(!network.seen().is_empty(), "{shown:?}");
}

#[tokio::test]
async fn a_board_listing_shows_300_and_says_of_how_many() {
    let channels: Vec<_> = (0..301)
        .map(|n| {
            format!(r#"{{"id":"00000001-0000-4000-8000-{n:012}","type":"board","name":"b{n}"}}"#)
        })
        .collect();
    let tree = format!(r#"{{"channels":[{}]}}"#, channels.join(","));
    let (holst, _) =
        McpServer::scripted([CannedResponse::ok(ME), CannedResponse::ok(tree.leak())]);
    let shown = board_call(&holst, BoardVerb::Ls, serde_json::json!({}))
        .await
        .into_text();
    let lines: Vec<_> = shown.lines().collect();
    assert_eq!(lines.len(), 301, "{shown}");
    assert_eq!(
        lines.get(300),
        Some(&"… 300 of 301; next: cursor 300"),
        "{shown}"
    );
}

#[tokio::test]
async fn a_template_listing_shows_100_and_says_of_how_many() {
    let templates: Vec<_> = (0..101)
        .map(|n| format!(r#"{{"id":"t{n}","name":"T{n}"}}"#))
        .collect();
    let body = format!(r#"{{"board_templates":[{}]}}"#, templates.join(","));
    let (holst, _) = McpServer::scripted([CannedResponse::ok(body.leak())]);
    let arguments = serde_json::json!({});
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let shown = text(
        &holst
            .command(CommandName::Template(TemplateVerb::Ls), arguments)
            .await,
    )
    .into_text();
    let lines: Vec<_> = shown.lines().collect();
    assert_eq!(lines.len(), 101, "{shown}");
    assert!(lines.last() == Some(&"100 of 101"), "{:?}", lines.last());
}
