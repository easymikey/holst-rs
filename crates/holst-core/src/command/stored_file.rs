//! Originals of images and files and the images of pictures, without the login.

use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    command::{
        board_load::CacheUse,
        board_text::{LOAD_CONCURRENCY, load_all_with},
    },
    domain::{board::Object, image, text},
    effect::cache::CachePort,
    error::{Error, Result, http::HttpError, stored_file::StoredFileError},
    remote::client::Client,
};

const MAX_ORIGINAL: usize = 200 * 1024 * 1024;

const MAX_IMAGE: usize = 32 * 1024 * 1024;

const ORIGINAL_TIMEOUT: Duration = Duration::from_mins(10);

const IMAGE_TIMEOUT: Duration = Duration::from_secs(20);

/// Image address → its bytes, for drawing.
pub type Images = HashMap<String, Arc<Vec<u8>>>;

/// Returns the address an object's stored file is downloaded from.
///
/// # Errors
/// [`StoredFileError::NoFile`] for an object that is not an image or a file.
pub fn address(client: &Client, object: &Object) -> Result<String> {
    object
        .file
        .as_ref()
        .map(|file| format!("{}{}", client.base_url().storage(), file.name))
        .ok_or_else(|| {
            Error::StoredFile(StoredFileError::NoFile {
                object: object.id.clone(),
                kind: object.kind.short_name().to_owned(),
            })
        })
}

/// The name an object's file is saved under, made safe.
#[derive(Debug, PartialEq, Eq)]
pub struct SavedName {
    pub stem: String,
    pub extension: String,
}

impl std::fmt::Display for SavedName {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.extension.as_str() {
            "" => write!(out, "{}", self.stem),
            extension => write!(out, "{}.{extension}", self.stem),
        }
    }
}

/// Returns the name to save the file of `object` under, without a directory.
#[must_use]
pub fn saved_name(object: &Object) -> SavedName {
    let stored = object.file.as_ref().map_or("", |file| file.name.as_str());
    let uploaded = object
        .file
        .as_ref()
        .and_then(|file| file.original.as_deref())
        .unwrap_or(stored);
    let split = |name: &str| {
        let path = std::path::Path::new(name);
        let part = |part: Option<&std::ffi::OsStr>| {
            part.map(|part| part.to_string_lossy().into_owned())
        };
        (part(path.file_stem()), part(path.extension()))
    };
    let (stem, extension) = split(uploaded);
    let extension = extension.or_else(|| split(stored).1).unwrap_or_default();
    SavedName {
        stem: text::file_part(&stem.unwrap_or_default()),
        extension: extension
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(10)
            .collect(),
    }
}

/// Downloads the original of an image or a file object, at most 200 MiB in
/// 10 minutes.
///
/// # Errors
/// [`HttpError::Transport`] if the limited client cannot be built, else as
/// [`address`] and [`download`].
pub async fn original(client: &Client, object: &Object) -> Result<Vec<u8>> {
    let limited = client
        .clone()
        .with_response_limit(ORIGINAL_TIMEOUT, MAX_ORIGINAL)?;
    download(&limited, &address(client, object)?, MAX_ORIGINAL).await
}

/// Downloads a stored file at `address`, at most `limit` bytes: only from the
/// public storage, with neither the token nor the cookies.
///
/// # Errors
/// [`StoredFileError::ForeignStorage`] off the storage,
/// [`crate::error::http::HttpError::ResponseTooLarge`] over the limit, or what
/// the download meets.
pub async fn download(client: &Client, address: &str, limit: usize) -> Result<Vec<u8>> {
    let base = client.base_url();
    base.stored_name(address).ok_or_else(|| {
        Error::StoredFile(StoredFileError::ForeignStorage {
            address: address.to_owned(),
            storage: base.storage(),
        })
    })?;
    let bytes = client.download_trusted_link(address).await?;
    if bytes.len() <= limit {
        Ok(bytes)
    } else {
        Err(Error::Http(HttpError::ResponseTooLarge { limit }))
    }
}

/// Returns the images at `addresses` that the storage holds: from `cache`,
/// else downloaded a few at a time unless offline, and cached. One that fails,
/// is too large (over 32 MiB) or slow (over 20 s), or is not an image is left
/// out: it stays a placeholder.
///
/// # Errors
/// [`HttpError::Transport`] if the limited client cannot be built.
pub async fn images(
    client: &Client,
    addresses: &[String],
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Result<Images> {
    let limited = client
        .clone()
        .with_response_limit(IMAGE_TIMEOUT, MAX_IMAGE)?;
    Ok(images_over(&limited, addresses, cache, cache_use).await)
}

async fn images_over(
    client: &Client,
    addresses: &[String],
    cache: Option<&dyn CachePort>,
    cache_use: CacheUse,
) -> Images {
    let base = client.base_url();
    let stored: Vec<(&String, &str)> = addresses
        .iter()
        .filter_map(|address| base.stored_name(address).map(|name| (address, name)))
        .collect();
    let cached = cached_images(&stored, cache);
    let missing: Vec<String> = stored
        .iter()
        .filter(|(address, _)| !cached.contains_key(*address))
        .filter(|_| cache_use != CacheUse::Offline)
        .map(|(address, _)| (*address).clone())
        .collect();
    let fresh = downloaded(client, missing).await;
    let fresh = kept(client, cache, fresh).await;
    cached
        .into_iter()
        .chain(
            fresh
                .into_iter()
                .map(|(address, bytes)| (address, Arc::new(bytes))),
        )
        .collect()
}

fn cached_images(stored: &[(&String, &str)], cache: Option<&dyn CachePort>) -> Images {
    stored
        .iter()
        .filter_map(|(address, name)| {
            let bytes = cache.and_then(|cache| cache.load_asset(name))?;
            image::image_format(&bytes).map(|_| ((*address).clone(), Arc::new(bytes)))
        })
        .collect()
}

async fn downloaded(client: &Client, missing: Vec<String>) -> Vec<(String, Vec<u8>)> {
    let fetched = load_all_with(missing, LOAD_CONCURRENCY, |address| {
        let client = client.clone();
        async move {
            let downloading = download(&client, &address, MAX_IMAGE);
            tokio::time::timeout(IMAGE_TIMEOUT, downloading).await
        }
    })
    .await;
    fetched
        .into_iter()
        .filter_map(|(address, outcome)| match outcome {
            Ok(Ok(bytes)) if image::image_format(&bytes).is_some() => {
                Some((address, bytes))
            }
            Ok(Ok(_)) => {
                tracing::debug!(address, "a stored file is not an image");
                None
            }
            Ok(Err(err)) => {
                tracing::debug!(address, %err, "an image was not downloaded");
                None
            }
            Err(_) => {
                tracing::debug!(address, "an image took too long");
                None
            }
        })
        .collect()
}

async fn kept(
    client: &Client,
    cache: Option<&dyn CachePort>,
    fresh: Vec<(String, Vec<u8>)>,
) -> Vec<(String, Vec<u8>)> {
    let base = client.base_url().clone();
    let handle = cache.filter(|_| !fresh.is_empty()).map(CachePort::shared);
    match handle {
        None => fresh,
        Some(handle) => {
            let storing = tokio::task::spawn_blocking(move || {
                let named: Vec<(&str, &[u8])> = fresh
                    .iter()
                    .filter_map(|(address, bytes)| {
                        base.stored_name(address)
                            .map(|name| (name, bytes.as_slice()))
                    })
                    .collect();
                handle.store_assets(&named).err().into_iter().for_each(
                    |err| tracing::debug!(%err, "board cache: images not kept"),
                );
                fresh
            });
            storing.await.unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "tests build and read real files in temp directories"
    )]
    #![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]

    use crate::{
        command::{
            board_load::CacheUse,
            stored_file::{SavedName, download, images_over, saved_name},
        },
        domain::board::{Object, ObjectKind, StoredFile},
        effect::{http::HttpResponse, settings::CacheSettings},
        error::{Error, Result, http::HttpError, stored_file::StoredFileError},
        remote::client::fake::{on_fake, reply, stored},
        storage::cache::Cache,
    };

    const IMAGE: &str = "https://storage-edge.app.holst.so/storage/a1.png";

    fn bytes(body: &[u8]) -> HttpResponse {
        HttpResponse {
            body: body.to_vec(),
            ..reply(200, &[], "")
        }
    }

    #[test]
    fn saved_names_keep_the_upload_name_made_safe() {
        let with = |name: &str, original: Option<&str>| Object {
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
            look: None,
            ends: None,
            author: None,
            locked: false,
            rotation: 0.0,
            link_to: None,
            file: Some(StoredFile {
                name: name.into(),
                original: original.map(str::to_owned),
            }),
            fields: serde_json::Value::Null,
        };
        let named = |stem: &str, extension: &str| SavedName {
            stem: stem.into(),
            extension: extension.into(),
        };
        assert_eq!(
            saved_name(&with("a1.png", Some("Кот.png"))),
            named("Кот", "png")
        );
        assert_eq!(
            saved_name(&with("a1.docx", Some("../План"))),
            named("План", "docx")
        );
        assert_eq!(saved_name(&with("a1.png", None)), named("a1", "png"));
        assert_eq!(
            saved_name(&with("x/abc.png", Some("Screenshot"))).to_string(),
            "Screenshot.png"
        );
    }

    #[tokio::test]
    async fn stored_files_come_from_the_storage_without_the_login() -> Result<()> {
        let session = stored("secret-token", "sessionToken=secret-cookie");
        let (fake, client) = on_fake([bytes(b"\x89PNG..")], Some(session))?;
        let foreign = download(&client, "https://evil.test/storage/a1.png", 10).await;
        assert!(matches!(
            foreign,
            Err(Error::StoredFile(StoredFileError::ForeignStorage { .. }))
        ));
        assert!(fake.requests().is_empty());
        assert_eq!(download(&client, IMAGE, 10).await?, b"\x89PNG..");
        let sent = fake.requests();
        let headers = sent.first().map(|request| format!("{:?}", request.headers));
        assert!(
            headers.is_some_and(|headers| !headers.contains("secret")),
            "{sent:?}"
        );
        let (_, small) = on_fake([bytes(b"\x89PNG..")], None)?;
        assert!(matches!(
            download(&small, IMAGE, 3).await,
            Err(Error::Http(HttpError::ResponseTooLarge { limit: 3 }))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn images_are_cached_and_failures_left_out() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("holst-assets-{}", std::process::id()));
        std::fs::remove_dir_all(&dir)
            .inspect_err(
                |err| tracing::debug!(%err, "the test directory is not removed"),
            )
            .unwrap_or_default();
        let settings = CacheSettings {
            root: Some(dir.clone()),
            roots: vec![dir.clone()],
            host: "app.holst.so".into(),
            max_bytes: 1 << 20,
            trust_millis: crate::effect::settings::DEFAULT_CACHE_TRUST_MILLIS,
        };
        let cache = Cache::from_settings(
            &settings,
            "aaaaaaaa-0000-4000-8000-000000000001",
            crate::effect::clock::FixedClock { unix_millis: 1 },
        )
        .ok_or(Error::LocalData(
            crate::error::local_data::LocalDataError::NoHome,
        ))?;
        let text = "https://storage-edge.app.holst.so/storage/b2.txt".to_owned();
        let asked = [IMAGE.to_owned(), text, "https://evil.test/c.png".to_owned()];
        let (fake, client) = on_fake([bytes(b"\x89PNG.."), bytes(b"plain")], None)?;
        let got =
            images_over(&client, &asked, Some(cache.as_port()), CacheUse::Use).await;
        assert_eq!(got.keys().collect::<Vec<_>>(), [IMAGE]);
        assert_eq!(fake.requests().len(), 2);
        let (quiet, offline) = on_fake([], None)?;
        let again =
            images_over(&offline, &asked, Some(cache.as_port()), CacheUse::Offline)
                .await;
        assert_eq!(again.get(IMAGE).map(|bytes| bytes.len()), Some(6));
        assert!(quiet.requests().is_empty());
        std::fs::remove_dir_all(&dir)
            .inspect_err(
                |err| tracing::debug!(%err, "the test directory is not removed"),
            )
            .unwrap_or_default();
        Ok(())
    }
}
