//! `object get`: the original of an image or a file object, downloaded.

use std::path::Path;

use futures_util::{StreamExt as _, stream};
use serde::Serialize;

use crate::{
    command::{
        board::{
            picture::{Delivery, Place},
            show::{Loading, account_cache, board_id},
        },
        board_load::CacheUse,
        object::{
            GetArgs,
            change::{load_board, named},
        },
        stored_file,
        table::not_value,
    },
    domain::{
        area::scene::Scene,
        board::{Object, objects as board_objects},
        lookup::rules::find_id,
    },
    effect::settings::CacheSettings,
    error::{Error, Result, board_change::BoardChangeError},
    remote::{client::Client, socket::write::connector},
    storage::cache::Cache,
};

const COMMAND: &str = "object get";

/// The originals downloaded from a board, and where they went.
#[derive(Debug, Serialize)]
pub struct Downloaded {
    pub board: String,
    pub objects: Vec<DownloadedObject>,
    /// The originals that could not be saved, with the reason.
    pub not_saved: Vec<String>,
}

/// The original of one object.
#[derive(Debug, Serialize)]
pub struct DownloadedObject {
    pub object: String,
    pub link: String,
    #[serde(flatten)]
    pub delivery: Delivery,
}

/// Downloads the originals of the image and file objects the call names. A
/// given file keeps its name, `-` returns the bytes; else each goes under its
/// own name into `--dir`, or into `downloads`, never replacing a file.
///
/// # Errors
/// No object, an object that is not found or has no file, a file for several
/// objects, no place to put the files, a file that exists, a board that is not
/// found, and errors of the requests.
pub async fn get(
    client: &Client,
    cache: &CacheSettings,
    downloads: Option<&Path>,
    args: &GetArgs,
) -> Result<Downloaded> {
    let cache_use = if args.refresh {
        CacheUse::Refresh
    } else {
        CacheUse::Use
    };
    let account = account_cache(client, cache, cache_use).await?;
    downloaded(
        client,
        Loading {
            connector: connector(),
            cache: account.as_ref().map(Cache::as_port),
            cache_use,
        },
        downloads,
        args,
    )
    .await
}

async fn downloaded(
    client: &Client,
    loading: Loading<'_>,
    downloads: Option<&Path>,
    args: &GetArgs,
) -> Result<Downloaded> {
    let references = named(&args.board, &args.object, COMMAND)?;
    // Refused before any request; the stems come with the objects.
    place_of(args, downloads, references.len(), "")?;
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        loading.cache_use,
    )
    .await?;
    let objects = board_objects(&load_board(client, &loading, &board).await?.document);
    let scene = Scene::new(&objects);
    let found = references
        .iter()
        .map(|reference| {
            let id =
                find_id(objects.iter().map(|object| object.id.as_str()), reference)?;
            scene.get(&id).ok_or_else(|| {
                Error::BoardChange(BoardChangeError::NoObject { reference: id })
            })
        })
        .collect::<Result<Vec<&Object>>>()?;
    let targets = found
        .iter()
        .map(|object| {
            let name = stored_file::saved_name(object);
            place_of(args, downloads, found.len(), &name.stem)
                .map(|place| (place, name.extension))
        })
        .collect::<Result<Vec<_>>>()?;
    let base = client.base_url();
    save_each(
        board.clone(),
        |id| base.object_link(&board, id),
        found.into_iter().zip(&targets),
        |object| stored_file::original(client, object),
    )
    .await
}

/// Downloads and writes the originals one at a time. One that fails is listed
/// in `not_saved`; the others stay. Only when none was saved is the first
/// failure the error.
async fn save_each<'a, Download, Bytes>(
    board: String,
    link: impl Fn(&str) -> String,
    found: impl IntoIterator<Item = (&'a Object, &'a (Option<Place<'a>>, String))>,
    download: Download,
) -> Result<Downloaded>
where
    Download: Fn(&'a Object) -> Bytes,
    Bytes: Future<Output = Result<Vec<u8>>>,
{
    let download = &download;
    let tried: Vec<(&Object, Result<Delivery>)> = stream::iter(found)
        .then(|(object, (place, extension))| async move {
            let delivery = download(object)
                .await
                .and_then(|bytes| deliver(place.as_ref(), extension, bytes));
            (object, delivery)
        })
        .collect()
        .await;
    let (objects, failed) = tried.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut objects, mut failed), (object, delivery)| {
            match delivery {
                Ok(delivery) => objects.push(DownloadedObject {
                    object: object.id.clone(),
                    link: link(&object.id),
                    delivery,
                }),
                Err(error) => failed.push((object.id.clone(), error)),
            }
            (objects, failed)
        },
    );
    let mut failed = failed.into_iter();
    match failed.next() {
        Some((_, error)) if objects.is_empty() => Err(error),
        first => Ok(Downloaded {
            board,
            objects,
            not_saved: first
                .into_iter()
                .chain(failed)
                .map(|(id, error)| format!("{id}: {error}"))
                .collect(),
        }),
    }
}

/// `None` for `-`: the bytes are returned. `stem` names a file in a directory.
fn place_of<'a>(
    args: &'a GetArgs,
    downloads: Option<&'a Path>,
    count: usize,
    stem: &str,
) -> Result<Option<Place<'a>>> {
    let dir = args.dir.as_deref().map(Path::new);
    match (args.file.as_ref(), dir, downloads) {
        (Some(_), Some(_), _) => Err(not_value(COMMAND, "dir", "left out with a file")),
        (Some(_), None, _) if count > 1 => {
            Err(not_value(COMMAND, "file", "left out with several objects"))
        }
        (Some(file), None, _) if file.0 == Path::new("-") => Ok(None),
        (Some(file), None, _) => Ok(Some(Place::File(&file.0))),
        (None, Some(dir), _) | (None, None, Some(dir)) => {
            Ok(Some(Place::Numbered(dir, stem.to_owned())))
        }
        (None, None, None) => Err(not_value(
            COMMAND,
            "file",
            "given: there is no home directory to find ~/Downloads in",
        )),
    }
}

fn deliver(
    place: Option<&Place<'_>>,
    extension: &str,
    bytes: Vec<u8>,
) -> Result<Delivery> {
    match place {
        None => Ok(Delivery::Inline { bytes }),
        Some(place) => Ok(Delivery::Written {
            file: place.write(extension, &bytes)?,
            bytes: bytes.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use crate::{
        Result,
        command::{
            board::{picture::Place, show::Loading},
            board_load::CacheUse,
            fixture::{
                IMAGE,
                STICKER,
                arguments,
                board_of,
                channel,
                directory,
                objects,
            },
            object::get::{deliver, downloaded, save_each},
        },
        domain::board::{Object, object_of},
        error::{Error, http::HttpError, stored_file::StoredFileError},
        remote::client::fake::{on_fake, stored},
    };

    fn names(dir: &Path) -> Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into()))
            .collect::<std::io::Result<_>>()?;
        names.sort();
        Ok(names)
    }

    /// What delivering the image's bytes to `place` came to.
    fn delivered(place: Option<&Place<'_>>) -> Result<Value> {
        let delivery = deliver(place, "png", b"\x89PNG..".to_vec())?;
        Ok(serde_json::to_value(delivery)?)
    }

    #[test]
    fn an_original_goes_under_its_own_name_and_is_never_replaced() -> Result<()> {
        let dir = directory()?;
        let place = Place::Numbered(&dir, "cat".to_owned());
        let first = delivered(Some(&place))?;
        delivered(Some(&place))?;
        assert_eq!(
            (names(&dir)?, std::fs::read(dir.join("cat.png"))?, first),
            (
                vec!["cat-2.png".to_owned(), "cat.png".to_owned()],
                b"\x89PNG..".to_vec(),
                json!({"status": "written", "file": dir.join("cat.png"), "bytes": 6})
            )
        );
        Ok(())
    }

    #[test]
    fn a_given_file_keeps_its_name_and_a_dash_returns_the_bytes() -> Result<()> {
        let dir = directory()?;
        let file: PathBuf = dir.join("mine.bin");
        let place = Place::File(&file);
        delivered(Some(&place))?;
        let again = delivered(Some(&place)).is_err();
        let inline = delivered(None)?;
        assert_eq!(
            (names(&dir)?, again, inline),
            (
                vec!["mine.bin".to_owned()],
                true,
                json!({"status": "inline"})
            )
        );
        Ok(())
    }

    async fn refusal(given: Value, downloads: Option<&Path>) -> Result<Option<Error>> {
        let (_, client) = on_fake([channel()], Some(stored("tok", "s=1")))?;
        Ok(downloaded(
            &client,
            Loading {
                connector: &board_of(&objects()),
                cache: None,
                cache_use: CacheUse::Use,
            },
            downloads,
            &arguments(given)?,
        )
        .await
        .err())
    }

    /// The storage holds every original but `i3`.
    fn storage_answer(object: &Object) -> Result<Vec<u8>> {
        if object.id == "i3" {
            Err(Error::Http(HttpError::Status {
                method: "GET",
                path: "/storage/i3.png".to_owned(),
                status: 404,
                detail: String::new(),
            }))
        } else {
            Ok(object.id.clone().into_bytes())
        }
    }

    #[tokio::test]
    async fn an_original_that_fails_is_named_and_the_others_are_kept() -> Result<()> {
        let dir = directory()?;
        let ids = ["i1", "i2", "i3"];
        let objects: Vec<Object> = ids
            .into_iter()
            .filter_map(|id| {
                object_of(json!({"id": id, "type": "image", "zIndex": 1,
                    "position": {"x": 0, "y": 0}, "width": 100, "height": 100,
                    "name": format!("{id}.png")}))
            })
            .collect();
        let targets: Vec<(Option<Place<'_>>, String)> = ids
            .into_iter()
            .map(|id| (Some(Place::Numbered(&dir, id.to_owned())), "png".to_owned()))
            .collect();
        let got = save_each(
            "b1".to_owned(),
            |id| format!("L/{id}"),
            objects.iter().zip(&targets),
            |object| std::future::ready(storage_answer(object)),
        )
        .await?;
        let listed: Vec<&str> = got
            .objects
            .iter()
            .map(|object| object.object.as_str())
            .collect();
        assert_eq!(
            (names(&dir)?, listed, got.not_saved),
            (
                vec!["i1.png".to_owned(), "i2.png".to_owned()],
                vec!["i1", "i2"],
                vec!["i3: GET /storage/i3.png -> HTTP 404".to_owned()]
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_object_without_a_file_is_refused() -> Result<()> {
        let refused =
            refusal(json!({"object": [STICKER]}), Some(Path::new("/nowhere"))).await?;
        assert!(
            matches!(
                refused,
                Some(Error::StoredFile(StoredFileError::NoFile { .. }))
            ),
            "{refused:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn one_file_cannot_hold_several_objects() -> Result<()> {
        let refused =
            refusal(json!({"object": [IMAGE, STICKER], "file": "a.bin"}), None)
                .await?
                .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("file of `object get` must be left out with several objects")
        );
        Ok(())
    }

    #[tokio::test]
    async fn no_place_for_the_file_is_refused() -> Result<()> {
        let refused = refusal(json!({"object": [IMAGE]}), None)
            .await?
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some(
                "file of `object get` must be given: there is no home directory to find ~/Downloads in"
            )
        );
        Ok(())
    }
}
