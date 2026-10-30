#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests build and read real files in temp directories"
)]

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        board::{
            file::{Exported, exported, imported, listing_path},
            show::Loading,
        },
        board_load::CacheUse,
        fixture::{arguments, client, directory, json_reply, over},
    },
    domain::{
        board::EditRight,
        board_file::compression::{self, Compression},
    },
    effect::{cache::CachePort, clock::FixedClock},
    protocol::hud::{DocumentResponse, UpdatePayload},
    remote::{
        client::fake::Fake,
        socket::fake::{
            BOARD,
            Script,
            Scripted,
            acking,
            answer,
            empty_board,
            script,
            sticker_by,
            update,
        },
    },
    storage::cache::memory::MemoryCache,
};

const WORKSPACE: &str = "00000001-0000-4000-8000-000000000001";

fn account() -> crate::effect::http::HttpResponse {
    json_reply(&json!({
        "id": "u1",
        "name": "Ann",
        "workspaces": [{ "id": WORKSPACE, "name": "easy" }],
    }))
}

fn channel() -> crate::effect::http::HttpResponse {
    json_reply(&json!({
        "id": BOARD, "name": "Plan", "workspace_id": WORKSPACE,
        "schema_version": 2, "server_provider_type": "hud",
    }))
}

/// A board holding one sticker, `a`.
fn full() -> (Scripted, Vec<u8>) {
    let (state, _) = sticker_by(4, "a");
    let socket = script([Some(vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        update(0, UpdatePayload::Inline(state.clone())),
    ])]);
    (socket, state)
}

fn empty() -> Scripted {
    acking([Some(empty_board(EditRight::Editable))])
}

async fn export(
    given: &Value,
    downloads: Option<&Path>,
    replies: usize,
) -> Result<Exported> {
    let (_, client) = client(std::iter::repeat_n((), replies + 1).map(|()| channel()))?;
    exported(
        &client,
        over(&full().0),
        downloads,
        &arguments(given.clone())?,
    )
    .await
}

#[tokio::test]
async fn an_export_goes_through_the_cache() -> Result<()> {
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let (_, client) = client([channel(), channel()])?;
    let socket = full().0;
    let loading = Loading {
        connector: &socket,
        cache: Some(&memory),
        cache_use: CacheUse::Use,
    };
    let args = arguments(json!({ "board": BOARD, "file": "-" }))?;
    exported(&client, loading, None, &args).await?;
    assert!(memory.load_document(BOARD).is_some());
    Ok(())
}

fn names(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into()))
        .collect::<std::io::Result<_>>()?;
    names.sort();
    Ok(names)
}

#[test]
fn the_object_list_sits_next_to_the_file() {
    assert_eq!(
        (
            listing_path(Path::new("dir/a.ydoc")),
            listing_path(Path::new("dir/a.ydoc.gz"))
        ),
        (
            PathBuf::from("dir/a.ydoc.objects.jsonl"),
            PathBuf::from("dir/a.ydoc.objects.jsonl")
        )
    );
}

#[tokio::test]
async fn a_dash_returns_the_document_and_writes_nothing() -> Result<()> {
    let kept = export(&json!({ "board": BOARD, "file": "-" }), None, 0).await?;
    let listed = export(
        &json!({ "board": BOARD, "file": "-", "object_list": true }),
        None,
        0,
    )
    .await;
    assert_eq!(
        (
            kept.inline.as_deref().map(compression::is_compressed),
            kept.file,
            listed.err().map(|error| error.to_string()),
        ),
        (
            Some(true),
            PathBuf::from("-"),
            Some(
                "object_list of `board export` must be left out when the file is -"
                    .to_owned()
            )
        )
    );
    Ok(())
}

#[tokio::test]
async fn the_default_name_ends_in_gz_only_when_compressed() -> Result<()> {
    let dir = directory()?;
    let given = json!({ "board": BOARD });
    export(&given, Some(&dir), 1).await?;
    export(&given, Some(&dir), 1).await?;
    export(
        &json!({ "board": BOARD, "no_compress": true }),
        Some(&dir),
        1,
    )
    .await?;
    let stem = format!("Plan-{}", BOARD.chars().take(8).collect::<String>());
    assert_eq!(
        names(&dir)?,
        [
            format!("{stem}-2.ydoc.gz"),
            format!("{stem}.ydoc"),
            format!("{stem}.ydoc.gz")
        ]
    );
    let none = export(&given, None, 0).await.err();
    assert_eq!(
        none.map(|error| error.to_string()),
        Some("file of `board export` must be given: there is no home directory to find ~/Downloads in".to_owned())
    );
    Ok(())
}

/// A board file of one sticker: gzip, as `export` writes it.
fn board_file(dir: &Path, name: &str) -> Result<PathBuf> {
    let (_, state) = full();
    let file = dir.join(name);
    std::fs::write(
        &file,
        compression::compressed(&state, Compression::Gzip)?.as_ref(),
    )?;
    Ok(file)
}

fn made() -> crate::effect::http::HttpResponse {
    json_reply(&json!({ "id": BOARD, "name": "Made" }))
}

/// A board that is subscribed, then the connection drops: an update sent
/// to it is never acknowledged.
fn dropping() -> Scripted {
    let Scripted(script) = script([Some(empty_board(EditRight::Editable))]);
    let incoming = std::mem::take(
        &mut *script
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    Scripted(Arc::new(Script {
        incoming: Mutex::new(incoming),
        resets: true,
        ..Script::default()
    }))
}

/// The method and path of each request that is not a read.
fn writes(fake: &Fake) -> Vec<(String, String)> {
    fake.requests()
        .iter()
        .filter(|request| request.method.as_str() != "GET")
        .map(|request| {
            (
                request.method.as_str().to_owned(),
                request.url.path().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_failed_import_removes_the_new_board() -> Result<()> {
    let file = board_file(&directory()?, "roadmap.ydoc.gz")?;
    let (fake, client) = client([
        account(),
        account(),
        made(),
        channel(),
        json_reply(&json!({})),
    ])?;
    let refused = imported(
        &client,
        over(&dropping()),
        &arguments(json!({ "path": file }))?,
    )
    .await;
    let deletes = writes(&fake)
        .into_iter()
        .filter(|(_, path)| path.contains(BOARD) && path.ends_with("/delete"))
        .count();
    assert_eq!((refused.is_err(), deletes), (true, 1));
    Ok(())
}

#[tokio::test]
async fn a_failed_import_that_cannot_remove_its_board_names_it() -> Result<()> {
    let file = board_file(&directory()?, "roadmap.ydoc.gz")?;
    let (_, client) = client([account(), account(), made(), channel()])?;
    let refused = imported(
        &client,
        over(&dropping()),
        &arguments(json!({ "path": file }))?,
    )
    .await;
    assert!(
        refused.is_err_and(|error| error.to_string().contains(BOARD)),
        "the error does not name the board"
    );
    Ok(())
}

#[tokio::test]
async fn only_into_takes_merge_and_only_a_new_board_takes_a_name() -> Result<()> {
    let file = board_file(&directory()?, "a.ydoc.gz")?;
    let refused = |given: Value| async move {
        let (_, client) = client([])?;
        let socket = empty();
        let error = imported(&client, over(&socket), &arguments(given)?)
            .await
            .err();
        Ok::<_, crate::Error>(error.map(|error| error.to_string()))
    };
    assert_eq!(
        refused(json!({ "path": file, "merge": true })).await?,
        Some(
            "merge of `board import` must be left out unless into is given".to_owned()
        )
    );
    assert_eq!(
        refused(json!({ "path": file, "into": BOARD, "name": "X" })).await?,
        Some("name of `board import` must be left out with into".to_owned())
    );
    Ok(())
}
