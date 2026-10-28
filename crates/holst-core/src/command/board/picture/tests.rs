#![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests build and read real files in temp directories"
)]

use serde_json::{Value, json};

use crate::{
    Result,
    command::{
        board::{
            SaveArgs,
            picture::{Delivery, Saved, Snapped, kept_originals, saved, snapped},
            show::Loading,
        },
        board_load::CacheUse,
        fixture::{arguments, client, directory},
    },
    domain::board::{EditRight, Object, ObjectKind, StoredFile},
    protocol::hud::{DocumentResponse, UpdatePayload},
    remote::{
        client::fake::reply,
        socket::fake::{BOARD, Scripted, answer, script, sticker_by, update},
    },
};

fn channel() -> crate::effect::http::HttpResponse {
    reply(
        200,
        &[("content-type", "application/json")],
        &json!({
            "id": BOARD,
            "name": "Plan",
            "workspace_id": "00000001-0000-4000-8000-000000000001",
            "link_access": "viewer",
            "schema_version": 2,
            "server_provider_type": "hud",
        })
        .to_string(),
    )
}

/// A board holding one sticker, `a`, at (1, 2).
fn board() -> Scripted {
    let (state, _) = sticker_by(4, "a");
    script([Some(vec![
        answer(DocumentResponse::Subscribed {
            right: EditRight::Editable,
        }),
        update(0, UpdatePayload::Inline(state)),
    ])])
}

fn loading(socket: &Scripted) -> Loading<'_> {
    Loading {
        connector: socket,
        cache: None,
        cache_use: CacheUse::Use,
    }
}

async fn snap(given: &Value, downloads: Option<&std::path::Path>) -> Result<Snapped> {
    let (_, client) = client([channel(), channel()])?;
    let socket = board();
    snapped(
        &client,
        loading(&socket),
        downloads,
        &arguments(given.clone())?,
    )
    .await
}

fn error_text(result: Result<Snapped>) -> Option<String> {
    result.err().map(|error| error.to_string())
}

fn written_name(snapped: &Snapped) -> Option<String> {
    match &snapped.delivery {
        Delivery::Written { file, .. } => file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        Delivery::Inline { .. } => None,
    }
}

#[tokio::test]
async fn the_default_place_is_a_numbered_name_in_the_directory() -> Result<()> {
    let dir = directory()?;
    let given = json!({ "board": BOARD, "format": "svg" });
    let names = [
        written_name(&snap(&given, Some(&dir)).await?),
        written_name(&snap(&given, Some(&dir)).await?),
    ];
    let stem = format!("Plan-{}", BOARD.chars().take(8).collect::<String>());
    assert_eq!(
        names,
        [Some(format!("{stem}.svg")), Some(format!("{stem}-2.svg"))]
    );
    Ok(())
}

#[tokio::test]
async fn a_file_that_is_neither_svg_nor_png_is_refused() {
    let gif = snap(&json!({ "board": BOARD, "file": "x.gif" }), None).await;
    let mismatch = snap(
        &json!({ "board": BOARD, "file": "x.svg", "format": "png" }),
        None,
    )
    .await;
    assert_eq!(
        (error_text(gif), error_text(mismatch)),
        (
            Some(
                "file of `board snap` must be a name that ends in .svg or .png"
                    .to_owned()
            ),
            Some(
                "format of `board snap` must be svg, as the file's name says"
                    .to_owned()
            )
        )
    );
}

#[tokio::test]
async fn the_longest_side_is_limited() -> Result<()> {
    let wide = snap(&json!({ "board": BOARD, "format": "svg" }), None).await?;
    let narrow = snap(
        &json!({ "board": BOARD, "format": "svg", "max_side": 50 }),
        None,
    )
    .await?;
    assert!(
        narrow.width.max(narrow.height) <= 50 && wide.width > narrow.width,
        "{} {}",
        wide.width,
        narrow.width
    );
    let refused = snap(&json!({ "board": BOARD, "scale": 0 }), None).await;
    assert!(error_text(refused).is_some_and(|text| text.contains("scale")));
    Ok(())
}

async fn save(given: &Value, parent: Option<&std::path::Path>) -> Result<Saved> {
    let (_, client) = client([channel(), channel()])?;
    let socket = board();
    saved(
        &client,
        loading(&socket),
        parent,
        &arguments::<SaveArgs>(given.clone())?,
    )
    .await
}

#[tokio::test]
async fn a_folder_needs_a_directory_to_go_into() {
    let none = save(&json!({ "board": BOARD }), None).await.err();
    assert_eq!(
        none.map(|error| error.to_string()),
        Some("dir of `board save` must be given: there is no home directory to find ~/Downloads in".to_owned())
    );
}

#[tokio::test]
async fn a_saved_original_is_linked_by_its_percent_encoded_name() -> Result<()> {
    let object = Object {
        id: "i".into(),
        kind: ObjectKind::of("image"),
        parent: None,
        x: 0.0,
        y: 0.0,
        width: None,
        height: None,
        z_index: 0.0,
        text: String::new(),
        color: None,
        ends: None,
        author: None,
        file: Some(StoredFile {
            name: "x/a1.png".into(),
            original: Some("a#1 100%.png".into()),
        }),
        fields: Value::Null,
    };
    let (_, client) = client([])?;
    let dir = directory()?;
    let images = dir.join("images");
    crate::storage::file::create_dir(&images)?;
    let shared = std::sync::Arc::new(std::collections::HashMap::from([(
        object.id.clone(),
        object,
    )]));
    let downloaded = vec![("i".to_owned(), Ok(b"not an image".to_vec()))];
    let originals = kept_originals(&client, &images, &shared, downloaded);
    assert!(originals.not_saved.is_empty(), "{:?}", originals.not_saved);
    assert!(
        originals
            .local_names
            .values()
            .eq(["images/a%231%20100%25.png"]),
        "{:?}",
        originals.local_names
    );
    assert!(dir.join("images").join("a#1 100%.png").is_file());
    Ok(())
}
