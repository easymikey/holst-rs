//! `board snap` and `board save`: a board as a picture or a folder.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::future::try_join;
use serde::Serialize;

use crate::{
    command::{
        board::{
            AreaArgs,
            PictureFormat,
            SaveArgs,
            SnapArgs,
            show::{
                Loading,
                account_cache,
                board_id,
                cache_use,
                chosen_area,
                explicit_area,
            },
        },
        board_load::{self, RemoteBoard},
        board_text,
        stored_file::{self, Images},
        table::not_value,
    },
    domain::{
        area::{Area, ListedArea, listed_area, scene::Scene},
        board::{BoardContent, Object, ObjectKind},
        geometry::Rectangle,
        id,
        image,
        node::NodeBrief,
        picture::{self, LinkedPicture, MAX_SIDE_FILE, PictureImages, PictureRequest},
        text::file_part,
    },
    effect::settings::CacheSettings,
    error::{Error, Result, stored_file::StoredFileError},
    remote::{api, client::Client, socket::write::connector},
    storage::{
        cache::Cache,
        file::{
            PartialDir,
            create_dir,
            create_dir_all,
            place_dir,
            write_new,
            write_numbered,
        },
    },
};

const SNAP: &str = "board snap";
const SAVE: &str = "board save";

/// A picture of a board, and where it went.
#[derive(Debug, Serialize)]
pub struct Snapped {
    /// A link to the board, or to the frame or object drawn.
    pub link: String,
    pub rectangle: Rectangle,
    pub objects: usize,
    pub width: u32,
    pub height: u32,
    pub format: PictureFormat,
    /// Why the picture may be hard to read.
    pub warning: Option<String>,
    #[serde(flatten)]
    pub delivery: Delivery,
}

/// What came of the picture's bytes.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Delivery {
    /// The file was written.
    Written { file: PathBuf, bytes: usize },
    /// No file was asked for and none is the default: the bytes are returned.
    Inline {
        #[serde(skip)]
        bytes: Vec<u8>,
    },
}

/// A folder of a board, written.
#[derive(Debug, Serialize)]
pub struct Saved {
    pub dir: PathBuf,
    pub files: Vec<SavedFile>,
    pub images: Vec<SavedFile>,
    /// The originals that could not be saved, with the reason.
    pub not_saved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedFile {
    pub name: String,
    pub bytes: usize,
    pub about: String,
}

/// Draws a board, or the part of it a selection or a link names. A named file
/// is written; `downloads` is where a picture goes when the call names no file,
/// and none means the bytes are returned.
///
/// # Errors
/// A file that is neither `.svg` nor `.png`, a format that differs from the
/// file's, `--refresh` with `--offline`, an area or a number out of range, a
/// board that is not found, a file that exists, and errors of the requests.
pub async fn snap(
    client: &Client,
    cache: &CacheSettings,
    downloads: Option<&Path>,
    args: &SnapArgs,
) -> Result<Snapped> {
    let cache_use = cache_use(&args.caching, SNAP)?;
    let account = account_cache(client, cache, cache_use).await?;
    snapped(
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

/// Saves a board, or the part of it a selection names, as a folder: the
/// picture, the SVG, the objects and the original images. The folder is new and
/// never replaces one.
///
/// # Errors
/// As [`snap`], and no directory when the call names none and `downloads` is
/// none.
pub async fn save(
    client: &Client,
    cache: &CacheSettings,
    downloads: Option<&Path>,
    args: &SaveArgs,
) -> Result<Saved> {
    let cache_use = cache_use(&args.caching, SAVE)?;
    let account = account_cache(client, cache, cache_use).await?;
    saved(
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

struct Drawing {
    id: String,
    name: String,
    content: BoardContent,
    request: PictureRequest,
    drawn: LinkedPicture,
}

/// `<title>-<id>`: the title made safe for a file name and the first 8
/// characters of the id.
pub(super) fn stem_of(title: &str, id: &str) -> String {
    format!(
        "{}-{}",
        file_part(title),
        id.chars().take(8).collect::<String>()
    )
}

/// What to draw: the board, the part of it and the size.
struct Ask<'a> {
    board: &'a str,
    workspace: Option<&'a str>,
    selection: &'a AreaArgs,
    scale: f64,
    max_side: f64,
}

impl Drawing {
    async fn of(client: &Client, loading: Loading<'_>, ask: Ask<'_>) -> Result<Self> {
        let request = PictureRequest::new(
            explicit_area(ask.selection)?,
            ask.selection.padding,
            ask.scale,
            ask.max_side,
        )?;
        let id = board_id(client, ask.board, ask.workspace, loading.cache_use).await?;
        let (channel, content) = try_join(
            api::channel::get(client, &id),
            board_load::content(
                RemoteBoard {
                    client,
                    connector: loading.connector,
                    board: &id,
                },
                loading.cache,
                loading.cache_use,
            ),
        )
        .await?;
        let request = PictureRequest {
            area: chosen_area(
                &request.area,
                id::object_in(ask.board),
                &content.objects,
            )?,
            ..request
        };
        let drawn =
            picture::draw(&content.objects, &request, &client.base_url().storage())?;
        Ok(Self {
            name: NodeBrief::of(&channel)?.name().to_owned(),
            id,
            content,
            request,
            drawn,
        })
    }

    /// The part asked for: a frame or objects, with the one a link opens; none
    /// for the board or a rectangle, even when one object fills it.
    fn target(&self) -> Option<&Object> {
        let wanted = self.drawn.target.as_deref().filter(|_| {
            matches!(self.request.area, Area::Frame(_) | Area::Objects(_))
        })?;
        self.content
            .objects
            .iter()
            .find(|object| object.id == wanted)
    }

    /// `<name>-<id>`: the frame's title, else the board's name, and the first
    /// 8 characters of the id of the frame or object, else of the board.
    fn file_stem(&self) -> String {
        let title = self
            .target()
            .filter(|object| object.kind == ObjectKind::Frame)
            .map(|frame| frame.text.as_str())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&self.name);
        let id = self
            .target()
            .map_or(self.id.as_str(), |object| object.id.as_str());
        stem_of(title, id)
    }

    async fn images(&self, client: &Client, loading: Loading<'_>) -> Result<Images> {
        stored_file::images(
            client,
            &self.drawn.picture.image_addresses,
            loading.cache,
            loading.cache_use,
        )
        .await
    }

    /// The SVG with the images at `replacements` in place of their links.
    fn svg_with(
        &self,
        client: &Client,
        replacements: &HashMap<String, String>,
    ) -> Result<String> {
        let storage = client.base_url().storage();
        let assets = PictureImages {
            storage: &storage,
            replacements,
        };
        Ok(
            picture::draw_with(&self.content.objects, &self.request, &assets)?
                .picture
                .svg,
        )
    }

    async fn bytes(
        &self,
        client: &Client,
        loading: Loading<'_>,
        (format, embed): (PictureFormat, bool),
    ) -> Result<Vec<u8>> {
        match (format, embed) {
            (PictureFormat::Svg, false) => {
                Ok(self.drawn.picture.svg.clone().into_bytes())
            }
            (PictureFormat::Svg, true) => {
                let replacements: HashMap<String, String> = self
                    .images(client, loading)
                    .await?
                    .into_iter()
                    .filter_map(|(address, bytes)| {
                        image::data_url(&bytes).map(|url| (address, url))
                    })
                    .collect();
                Ok(self.svg_with(client, &replacements)?.into_bytes())
            }
            (PictureFormat::Png, _) => {
                png(
                    &self.drawn.picture.svg,
                    &self.images(client, loading).await?,
                )
                .await
            }
        }
    }
}

// Renders off the runtime thread.
#[cfg(feature = "png")]
async fn png(svg: &str, images: &Images) -> Result<Vec<u8>> {
    let (svg, images) = (svg.to_owned(), images.clone());
    tokio::task::spawn_blocking(move || crate::system::raster::png_with(&svg, &images))
        .await
        .map_err(|_| Error::Picture(crate::error::picture::PictureError::PngEncode))?
}

#[cfg(not(feature = "png"))]
fn png(_: &str, _: &Images) -> impl Future<Output = Result<Vec<u8>>> {
    std::future::ready(Err(not_value(
        SNAP,
        "format",
        "svg: this build draws no PNG",
    )))
}

/// As asked, else by the file's extension, else PNG.
fn picture_format(
    asked: Option<PictureFormat>,
    file: Option<&Path>,
) -> Result<PictureFormat> {
    let extension = file.and_then(Path::extension);
    let by_extension = extension.and_then(|extension| {
        [PictureFormat::Svg, PictureFormat::Png]
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    });
    match (asked, by_extension, extension) {
        (Some(asked), Some(found), _) if asked != found => Err(not_value(
            SNAP,
            "format",
            &format!("{}, as the file's name says", found.extension()),
        )),
        (None, None, Some(_)) => {
            Err(not_value(SNAP, "file", "a name that ends in .svg or .png"))
        }
        (asked, found, _) => Ok(asked.or(found).unwrap_or(PictureFormat::Png)),
    }
}

/// Where a picture's bytes go.
pub(crate) enum Place<'a> {
    File(&'a Path),
    /// The first free `<stem>.<ext>`, `<stem>-2.<ext>`, … in a directory.
    Numbered(&'a Path, String),
}

impl Place<'_> {
    pub(crate) fn write(&self, extension: &str, bytes: &[u8]) -> Result<PathBuf> {
        match self {
            Place::File(path) => write_new(path, bytes).map(|()| path.to_path_buf()),
            Place::Numbered(directory, stem) => {
                create_dir_all(directory)?;
                write_numbered(directory, stem, extension, bytes)
            }
        }
    }
}

pub(super) async fn snapped(
    client: &Client,
    loading: Loading<'_>,
    downloads: Option<&Path>,
    args: &SnapArgs,
) -> Result<Snapped> {
    let file = args.file.as_ref().map(|file| file.0.as_path());
    let format = picture_format(args.format, file)?;
    let board = Drawing::of(
        client,
        loading,
        Ask {
            board: &args.board,
            workspace: args.workspace.as_deref(),
            selection: &args.selection,
            scale: args.scale.unwrap_or(1.0),
            max_side: args.max_side.map_or(MAX_SIDE_FILE.get(), f64::from),
        },
    )
    .await?;
    let place = match (file, downloads) {
        (Some(file), _) => Some(Place::File(file)),
        (None, Some(dir)) => Some(Place::Numbered(dir, board.file_stem())),
        (None, None) => None,
    };
    let bytes = board.bytes(client, loading, (format, args.embed)).await?;
    let delivery = match place {
        Some(place) => Delivery::Written {
            file: place.write(format.extension(), &bytes)?,
            bytes: bytes.len(),
        },
        None => Delivery::Inline { bytes },
    };
    let picture = &board.drawn.picture;
    Ok(Snapped {
        link: client
            .base_url()
            .link(&board.id, board.drawn.target.as_deref()),
        rectangle: picture.rectangle,
        objects: picture.object_count,
        width: picture.width,
        height: picture.height,
        format,
        warning: picture.warning(),
        delivery,
    })
}

async fn saved(
    client: &Client,
    loading: Loading<'_>,
    downloads: Option<&Path>,
    args: &SaveArgs,
) -> Result<Saved> {
    let parent = args
        .dir
        .as_deref()
        .map(Path::new)
        .or(downloads)
        .ok_or_else(|| {
            not_value(
                SAVE,
                "dir",
                "given: there is no home directory to find ~/Downloads in",
            )
        })?;
    let board = Drawing::of(
        client,
        loading,
        Ask {
            board: &args.board,
            workspace: args.workspace.as_deref(),
            selection: &args.selection,
            scale: 1.0,
            max_side: MAX_SIDE_FILE.get(),
        },
    )
    .await?;
    let listed = listed_area(
        &board.content.objects,
        &board.request.area,
        board.request.padding,
    )?;
    let partial = PartialDir::create(parent)?;
    let (files, originals) = fill(client, &board, &listed, partial.path()).await?;
    Ok(Saved {
        dir: place_dir(parent, &board.file_stem(), &partial)?,
        files,
        images: originals.saved,
        not_saved: originals.not_saved,
    })
}

/// Writes the picture, the SVG and the objects, and the originals under
/// `images/`, into `dir`.
async fn fill(
    client: &Client,
    board: &Drawing,
    listed: &ListedArea<'_>,
    dir: &Path,
) -> Result<(Vec<SavedFile>, Originals)> {
    let originals = save_originals(client, dir, &listed.objects).await?;
    let name = match board.target().map(|object| &object.kind) {
        None => "board",
        Some(ObjectKind::Frame) => "frame",
        Some(_) => "object",
    };
    let png = png(&board.drawn.picture.svg, &originals.images).await?;
    let svg = board.svg_with(client, &originals.local_names)?;
    let (listing, matched) = object_list_bytes(client, board, listed)?;
    let picture = &board.drawn.picture;
    let files = [
        (
            format!("{name}.png"),
            png,
            format!("{}×{} px", picture.width, picture.height),
        ),
        (
            format!("{name}.svg"),
            svg.into_bytes(),
            "linked to images/".to_owned(),
        ),
        (
            "objects.json".to_owned(),
            listing,
            format!("{matched} objects"),
        ),
    ]
    .into_iter()
    .map(|(file, bytes, about)| {
        write_new(&dir.join(&file), &bytes)?;
        Ok(SavedFile {
            name: file,
            bytes: bytes.len(),
            about,
        })
    })
    .collect::<Result<Vec<_>>>()?;
    Ok((files, originals))
}

fn object_list_bytes(
    client: &Client,
    board: &Drawing,
    listed: &ListedArea<'_>,
) -> Result<(Vec<u8>, usize)> {
    let shown = BoardContent {
        edit_right: board.content.edit_right,
        objects: listed
            .objects
            .iter()
            .map(|&object| object.clone())
            .collect(),
    };
    let matched = shown.objects.len();
    let scene = Scene::new(&board.content.objects);
    let base = client.base_url();
    let linked = shown.linked(
        |object| base.object_link(&board.id, object),
        |object| scene.drawn_ends(object),
        &base.storage(),
    )?;
    let mut listing = linked;
    if let Some(fields) = listing.as_object_mut() {
        fields.insert("rectangle".to_owned(), serde_json::json!(listed.rectangle));
        fields.insert("matched".to_owned(), serde_json::json!(matched));
    }
    Ok((serde_json::to_vec_pretty(&listing)?, matched))
}

type FilesById = Arc<HashMap<String, Object>>;

struct Originals {
    saved: Vec<SavedFile>,
    images: Images,
    local_names: HashMap<String, String>,
    not_saved: Vec<String>,
}

/// The ids of the objects that carry a file, with those objects by id.
fn files_by_id(objects: &[&Object]) -> (Vec<String>, FilesById) {
    let with_files: Vec<Object> = objects
        .iter()
        .filter(|object| object.file.is_some())
        .map(|&object| object.clone())
        .collect();
    let ids = with_files.iter().map(|object| object.id.clone()).collect();
    let by_id = with_files
        .into_iter()
        .map(|object| (object.id.clone(), object))
        .collect();
    (ids, Arc::new(by_id))
}

/// Downloads the originals of the objects' files into `dir/images`; one that
/// cannot be downloaded or written is listed in `not_saved`.
async fn save_originals(
    client: &Client,
    dir: &Path,
    objects: &[&Object],
) -> Result<Originals> {
    let images_dir = dir.join("images");
    create_dir(&images_dir)?;
    let (ids, shared) = files_by_id(objects);
    let downloaded =
        board_text::load_all_with(ids, board_text::LOAD_CONCURRENCY, |object_id| {
            let (client, held) = (client.clone(), Arc::clone(&shared));
            async move {
                match held.get(&object_id) {
                    Some(object) => stored_file::original(&client, object).await,
                    None => Err(Error::StoredFile(StoredFileError::NoFile {
                        object: object_id,
                        kind: "object".to_owned(),
                    })),
                }
            }
        })
        .await;
    Ok(kept_originals(client, &images_dir, &shared, downloaded))
}

/// Writes what was downloaded into `images_dir`; one that cannot be written is
/// listed in `not_saved`.
fn kept_originals(
    client: &Client,
    images_dir: &Path,
    shared: &FilesById,
    downloaded: Vec<(String, Result<Vec<u8>>)>,
) -> Originals {
    downloaded
        .into_iter()
        .filter_map(|(object_id, downloaded)| {
            shared
                .get(&object_id)
                .cloned()
                .map(|object| (object, downloaded))
        })
        .fold(
            Originals {
                saved: Vec::new(),
                images: Images::new(),
                local_names: HashMap::new(),
                not_saved: Vec::new(),
            },
            |mut originals, (object, downloaded)| {
                let kept = downloaded.and_then(|bytes| {
                    save_original(client, images_dir, &object, bytes)
                });
                match kept {
                    Ok((file, address, bytes)) => {
                        originals
                            .local_names
                            .insert(address.clone(), percent_encoded(&file.name));
                        originals.saved.push(file);
                        let image = image::image_format(&bytes)
                            .map(|_| (address, Arc::new(bytes)));
                        originals.images.extend(image);
                    }
                    Err(err) => {
                        originals.not_saved.push(format!("{}: {err}", object.id));
                    }
                }
                originals
            },
        )
}

/// `path` for a link: every byte but an unreserved character and `/` is `%XX`.
fn percent_encoded(path: &str) -> String {
    path.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/' => char::from(byte).to_string(),
            byte => format!("%{byte:02X}"),
        })
        .collect()
}

fn save_original(
    client: &Client,
    dir: &Path,
    object: &Object,
    bytes: Vec<u8>,
) -> Result<(SavedFile, String, Vec<u8>)> {
    let address = stored_file::address(client, object)?;
    let saved = stored_file::saved_name(object);
    let path = write_numbered(dir, &saved.stem, &saved.extension, &bytes)?;
    let name = path
        .file_name()
        .map(|name| format!("images/{}", name.to_string_lossy()))
        .unwrap_or_default();
    let file = SavedFile {
        name,
        bytes: bytes.len(),
        about: object.kind.short_name().to_owned(),
    };
    Ok((file, address, bytes))
}

#[cfg(test)] mod tests;
