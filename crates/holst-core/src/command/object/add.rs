//! `object add`: stickers, texts, shapes, frames, arrows, links, images, files.

use std::path::Path;

use futures_util::{StreamExt as _, TryStreamExt as _, stream};

use crate::{
    command::{
        account,
        board::show::{Loading, account_cache, board_id},
        board_load::{self, CacheUse, RemoteBoard},
        board_write::{WriteRequest, write_cached},
        object::{AddArgs, AddItem, AddKind},
        table::not_value,
    },
    domain::{
        area::scene::Scene,
        board::{
            EditRight,
            Object,
            ObjectKind,
            objects as board_objects,
            shape_type::ShapeType,
        },
        change::{
            receipt::ChangeReport,
            value::{
                ArrowEdit,
                ArrowLook,
                BoardChange,
                ImageUpload,
                NewEntry,
                NewObject,
                ShapeEdit,
                Upload,
            },
        },
        geometry::{Point, Rectangle, Size},
        id,
        image::pixel_size,
        palette::Color,
        placement::{free_corner, free_spot_among},
        response::LinkPreview,
        rich_text::TextFormat,
        upload::{UploadKind, file_type},
    },
    effect::{
        board_write::{WriteInterpreter, WriteMode},
        confirmation::ALWAYS,
        settings::CacheSettings,
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        http::HttpError,
        input::InputError,
    },
    remote::{
        api::{
            files::{UploadBind, UploadOptions, upload},
            link::link_preview,
        },
        client::Client,
        socket::write::{connector, interpreter},
    },
    storage::{cache::Cache, file},
};

const COMMAND: &str = "object add";

/// Adds the objects the arguments ask for and says what was made, in the order
/// asked.
///
/// # Errors
/// An item that is not valid, a file that cannot be read or is not the
/// picture asked, a frame that is not found, a place outside its frame, a
/// board that is not found or not editable, and errors of the requests (the
/// uploads and the previews among them) and of the write. A page without a
/// preview is a warning, not an error.
pub async fn add(
    client: &Client,
    cache: &CacheSettings,
    args: &AddArgs,
) -> Result<ChangeReport> {
    // The arguments are read before the first request.
    let items = read_items(args)?;
    let cache_use = CacheUse::Use;
    let account = account_cache(client, cache, cache_use).await?;
    let mode = args.dry_run;
    added(
        client,
        Loading {
            connector: connector(),
            cache: account.as_ref().map(Cache::as_port),
            cache_use,
        },
        interpreter(mode),
        args,
        &items,
    )
    .await
}

async fn added<I: WriteInterpreter + ?Sized>(
    client: &Client,
    loading: Loading<'_>,
    interpreter: &I,
    args: &AddArgs,
    items: &[Item],
) -> Result<ChangeReport> {
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        loading.cache_use,
    )
    .await?;
    let (loaded, _) = board_load::load(
        RemoteBoard {
            client,
            connector: loading.connector,
            board: &board,
        },
        loading.cache,
        loading.cache_use,
    )
    .await?;
    // Before any upload: a board that cannot be changed takes no file.
    matches!(loaded.right, EditRight::Editable)
        .then_some(())
        .ok_or(Error::BoardChange(BoardChangeError::ReadOnly))?;
    let got = fetched(
        client,
        &board,
        matches!(args.dry_run, WriteMode::Send),
        items,
    )
    .await?;
    let objects = board_objects(&loaded.document);
    let scene = Scene::new(&objects);
    let change = BoardChange::AddAll(entries(
        items,
        &got,
        &scene,
        linked_frame(&scene, &args.board),
    )?);
    let user = account::account_id(client).await?;
    let changed = write_cached(
        client,
        interpreter,
        WriteRequest {
            board: &board,
            change: &change,
            send_if: ALWAYS,
            account_id: &user,
            kept: None,
        },
        loading.cache,
    )
    .await
    .map_err(Error::from)?;
    let base = client.base_url();
    let report = changed
        .change
        .written
        .report(&board, |object| base.object_link(&board, object));
    Ok(ChangeReport {
        warnings: warnings(items, &got),
        ..report
    })
}

/// What the network gave an item before the write.
#[derive(Debug, PartialEq)]
enum Fetched {
    /// Nothing was asked: another kind, or an upload of a dry run.
    Nothing,
    /// The preview of a link's page; `None` when the page has none.
    Page(Option<LinkPreview>),
    /// The file the server holds.
    Upload(Upload),
}

/// What the network gives each item, in item order, one request after the
/// other; a dry run uploads nothing.
async fn fetched(
    client: &Client,
    board: &str,
    send: bool,
    items: &[Item],
) -> Result<Vec<Fetched>> {
    stream::iter(items)
        .then(|item| async move {
            match &item.what {
                What::Link { url, .. } => match link_preview(client, url).await {
                    Ok(preview) => Ok(Fetched::Page(Some(preview))),
                    Err(Error::Http(HttpError::Status { .. })) => {
                        Ok(Fetched::Page(None))
                    }
                    Err(error) => Err(error),
                },
                What::Image { file, .. } | What::File { file, .. } if send => {
                    uploaded(client, board, file).await.map(Fetched::Upload)
                }
                What::Sticker { .. }
                | What::Text { .. }
                | What::Shape { .. }
                | What::Frame { .. }
                | What::Arrow { .. }
                | What::Image { .. }
                | What::File { .. } => Ok(Fetched::Nothing),
            }
        })
        .try_collect()
        .await
}

/// The file uploaded to `board` under a fresh key; the server names it.
async fn uploaded(client: &Client, board: &str, file: &Local) -> Result<Upload> {
    let key = client.new_id();
    let options = UploadOptions {
        key: Some(&key),
        freshness: None,
        content_type: file.mime,
    };
    let name = upload(
        client,
        UploadBind::Channel(board),
        options,
        &file.name,
        file.bytes.clone(),
    )
    .await?
    .name
    .ok_or(Error::Http(HttpError::MissingField {
        response: "upload",
        field: "name",
    }))?;
    Ok(file.upload(name))
}

/// A card for each link whose page gave no preview.
fn warnings(items: &[Item], got: &[Fetched]) -> Vec<String> {
    items
        .iter()
        .zip(got)
        .filter_map(|(item, got)| match (&item.what, got) {
            (What::Link { url, .. }, Fetched::Page(None)) => {
                Some(format!("no preview: {url}"))
            }
            _ => None,
        })
        .collect()
}

/// One object to add with every value read and its kind's rules kept.
#[derive(Debug, PartialEq)]
struct Item {
    what: What,
    color: Option<Color>,
    frame: Option<String>,
    /// How the text of a sticker, text or shape is written.
    format: TextFormat,
}

#[derive(Debug, PartialEq)]
enum What {
    Sticker {
        text: String,
        at: Option<Point>,
        size: Option<Size>,
    },
    Text {
        text: String,
        at: Option<Point>,
    },
    Shape {
        text: String,
        at: Option<Point>,
        size: Option<Size>,
        kind: ShapeType,
    },
    Frame {
        title: String,
        at: Option<Point>,
        size: Option<Size>,
    },
    /// The ends are objects as given, or `#N`.
    Arrow {
        from: String,
        to: String,
        look: ArrowLook,
    },
    Link {
        url: String,
        at: Option<Point>,
        embed: bool,
    },
    /// Shown at `size`: asked for, else the pixels fitted to `LONG_SIDE`.
    Image {
        file: Local,
        pixels: Size,
        size: Size,
        at: Option<Point>,
    },
    File {
        file: Local,
        at: Option<Point>,
    },
}

/// The longest side an image is placed at when no size is asked.
const LONG_SIDE: f64 = 1000.0;

/// A local file read before the first request.
#[derive(Debug, PartialEq)]
struct Local {
    /// The name on disk, without the folders.
    name: String,
    mime: &'static str,
    bytes: Vec<u8>,
}

impl Local {
    /// The file as the server holds it under `name`.
    fn upload(&self, name: String) -> Upload {
        Upload {
            name,
            original: self.name.clone(),
            mime: self.mime.to_owned(),
            bytes: u64::try_from(self.bytes.len()).unwrap_or(u64::MAX),
        }
    }
}

/// The pixels as they are, or scaled down so the longer side is [`LONG_SIDE`].
fn fitted(pixels: Size) -> Result<Size> {
    let scale = (LONG_SIDE / pixels.width().max(pixels.height())).min(1.0);
    if scale >= 1.0 {
        Ok(pixels)
    } else {
        Size::new(
            (pixels.width() * scale).round().max(1.0),
            (pixels.height() * scale).round().max(1.0),
        )
    }
}

/// The items: those of `--items`, or the one the flags describe.
fn read_items(args: &AddArgs) -> Result<Vec<Item>> {
    let kind = args.kind.unwrap_or_default();
    let listed = args.items.as_ref().map(|items| items.0.as_slice());
    listed
        .filter(|listed| listed.is_empty())
        .map_or(Ok(()), |_| {
            Err(not_value(
                COMMAND,
                "items",
                "a list of at least one object: --items holds no objects",
            ))
        })?;
    let many = listed.is_some();
    let given = [
        ("text", args.text.is_some()),
        ("shape", args.shape.is_some()),
        ("at", args.at.is_some()),
        ("size", args.size.is_some()),
        ("color", args.color.is_some()),
        ("from", args.from.is_some()),
        ("to", args.to.is_some()),
        ("line", args.line.is_some()),
        ("heads", args.heads.is_some()),
        ("stroke_style", args.stroke_style.is_some()),
        ("stroke_width", args.stroke_width.is_some()),
        ("url", args.url.is_some()),
        ("path", args.path.is_some()),
        ("no_embed", args.no_embed),
    ];
    given
        .into_iter()
        .find_map(|(name, given)| (many && given).then_some(name))
        .map_or(Ok(()), |name| {
            Err(not_value(COMMAND, name, "left out with --items"))
        })?;
    let own = [AddItem {
        kind: None,
        text: args.text.clone(),
        shape: args.shape,
        at: args.at.clone(),
        size: args.size.clone(),
        color: args.color.clone(),
        frame: None,
        from: args.from.clone(),
        to: args.to.clone(),
        line: args.line,
        heads: args.heads.clone(),
        stroke_style: args.stroke_style,
        stroke_width: args.stroke_width,
        url: args.url.clone(),
        path: args.path.clone(),
        no_embed: args.no_embed.then_some(true),
    }];
    let asked: &[AddItem] = listed.unwrap_or(&own);
    let format = args.text_format.unwrap_or_default();
    asked
        .iter()
        .enumerate()
        .try_fold(Vec::new(), |mut read, (slot, item)| {
            let item = Reader { many, slot, format }.item(
                item,
                kind,
                args.frame.as_deref(),
                &read,
            )?;
            read.push(item);
            Ok::<_, Error>(read)
        })
}

/// Reads the values of one item, which is the `slot`-th of `--items` if `many`.
struct Reader {
    many: bool,
    slot: usize,
    format: TextFormat,
}

impl Reader {
    fn bad(&self, name: &'static str, expected: &str) -> Error {
        let place = self.many.then(|| format!(", in item {}", self.slot + 1));
        not_value(
            COMMAND,
            name,
            &format!("{expected}{}", place.unwrap_or_default()),
        )
    }

    /// Whether the kind holds rich text; markdown for another kind alone is
    /// refused.
    fn rich(&self, kind: AddKind, word: &str) -> Result<bool> {
        let rich = matches!(kind, AddKind::Sticker | AddKind::Text | AddKind::Shape);
        let refused = !rich && !self.many && self.format == TextFormat::Markdown;
        refused
            .then(|| self.bad("text_format", &format!("left out for {word}")))
            .map_or(Ok(rich), Err)
    }

    /// Refuses the keys only an arrow takes on another kind.
    fn refuse_arrow_keys(&self, item: &AddItem, kind: AddKind) -> Result<()> {
        [
            ("from", item.from.is_some()),
            ("to", item.to.is_some()),
            ("line", item.line.is_some()),
            ("heads", item.heads.is_some()),
            ("stroke_style", item.stroke_style.is_some()),
            ("stroke_width", item.stroke_width.is_some()),
        ]
        .into_iter()
        .find_map(|(name, given)| (given && kind != AddKind::Arrow).then_some(name))
        .map_or(Ok(()), |name| {
            Err(self.bad(name, &format!("left out for {}", word(kind))))
        })
    }

    fn missing(&self, name: &'static str) -> Error {
        if self.many {
            self.bad(name, "given")
        } else {
            Error::from(InputError::NoArgument {
                command: COMMAND.to_owned(),
                name,
            })
        }
    }

    fn item(
        &self,
        item: &AddItem,
        kind: AddKind,
        frame: Option<&str>,
        earlier: &[Item],
    ) -> Result<Item> {
        let kind = item.kind.unwrap_or(kind);
        let word = word(kind);
        let refuse = |name: &'static str, given: bool| {
            given
                .then(|| self.bad(name, &format!("left out for {word}")))
                .map_or(Ok(()), Err)
        };
        let arrow = kind == AddKind::Arrow;
        self.refuse_arrow_keys(item, kind)?;
        let linked = kind == AddKind::Link;
        let uploaded = matches!(kind, AddKind::Image | AddKind::File);
        refuse("text", (arrow || linked || uploaded) && item.text.is_some())?;
        refuse("at", arrow && item.at.is_some())?;
        refuse(
            "size",
            matches!(
                kind,
                AddKind::Arrow | AddKind::Text | AddKind::Link | AddKind::File
            ) && item.size.is_some(),
        )?;
        refuse("shape", kind != AddKind::Shape && item.shape.is_some())?;
        refuse("color", (linked || uploaded) && item.color.is_some())?;
        refuse("url", !linked && item.url.is_some())?;
        refuse("path", !uploaded && item.path.is_some())?;
        let rich = self.rich(kind, word)?;
        refuse("no_embed", !linked && item.no_embed.is_some())?;
        let at = item.at.as_deref().map(str::parse).transpose()?;
        let size = item.size.as_deref().map(str::parse).transpose()?;
        let text = item.text.clone();
        let needed = || text.clone().ok_or_else(|| self.missing("text"));
        let what = match kind {
            AddKind::Sticker => What::Sticker {
                text: needed()?,
                at,
                size,
            },
            AddKind::Text => What::Text {
                text: needed()?,
                at,
            },
            AddKind::Shape => What::Shape {
                text: text.clone().unwrap_or_default(),
                at,
                size: size.or_else(|| item.shape.and_then(ShapeType::default_size)),
                kind: item.shape.unwrap_or(ShapeType::Square),
            },
            AddKind::Frame => What::Frame {
                title: text.clone().unwrap_or_default(),
                at,
                size,
            },
            AddKind::Arrow => What::Arrow {
                from: self.end("from", item.from.as_deref(), earlier)?,
                to: self.end("to", item.to.as_deref(), earlier)?,
                look: arrow_look(item)?,
            },
            AddKind::Link => What::Link {
                url: item.url.clone().ok_or_else(|| self.missing("url"))?,
                at,
                embed: item.no_embed != Some(true),
            },
            AddKind::Image => {
                let (name, bytes) = self.read(item.path.as_deref())?;
                let (pixels, mime) = self.picture(&bytes)?;
                What::Image {
                    size: size.map_or_else(|| fitted(pixels), Ok)?,
                    pixels,
                    at,
                    file: Local { name, mime, bytes },
                }
            }
            AddKind::File => {
                let (name, bytes) = self.read(item.path.as_deref())?;
                What::File {
                    at,
                    file: Local {
                        mime: file_type(&name),
                        name,
                        bytes,
                    },
                }
            }
        };
        let frame = item.frame.as_deref().or(frame);
        frame
            .filter(|reference| reference.starts_with('#'))
            .filter(|reference| {
                !slot_of(reference)
                    .and_then(|slot| earlier.get(slot))
                    .is_some_and(|before| matches!(before.what, What::Frame { .. }))
            })
            .map_or(Ok(()), |_| Err(self.bad("frame", "#N of an earlier frame")))?;
        Ok(Item {
            what,
            color: item.color.as_deref().map(str::parse).transpose()?,
            frame: frame.map(str::to_owned),
            format: if rich { self.format } else { TextFormat::Plain },
        })
    }

    /// The name and the bytes of the local file `given`.
    fn read(&self, given: Option<&str>) -> Result<(String, Vec<u8>)> {
        let path = given.ok_or_else(|| self.missing("path"))?;
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| self.bad("path", "a file"))?;
        let bytes = file::read(path)?;
        Ok((name, bytes))
    }

    /// The pixels and the MIME type of `bytes`, which must be a PNG, JPEG, GIF
    /// or WebP picture.
    fn picture(&self, bytes: &[u8]) -> Result<(Size, &'static str)> {
        let (format, (width, height)) = match UploadKind::of(bytes, true) {
            Some(UploadKind::Image(format)) => {
                pixel_size(bytes).map(|pixels| (format, pixels))
            }
            Some(UploadKind::File) | None => None,
        }
        .ok_or_else(|| self.bad("path", "a png, jpg, gif or webp picture"))?;
        Ok((
            Size::new(f64::from(width), f64::from(height))?,
            format.mime(),
        ))
    }

    /// An arrow end: an object as given, or `#N`, an earlier item that is not
    /// an arrow.
    fn end(
        &self,
        name: &'static str,
        given: Option<&str>,
        earlier: &[Item],
    ) -> Result<String> {
        let end = given.ok_or_else(|| self.missing(name))?;
        match slot_of(end).and_then(|slot| earlier.get(slot)) {
            Some(Item {
                what: What::Arrow { .. },
                ..
            }) => Err(self.bad(
                name,
                "an object, or #N of an earlier item that is not an arrow",
            )),
            None if end.starts_with('#') => Err(self.bad(
                name,
                "an object, or #N of an earlier item that is not an arrow",
            )),
            Some(_) | None => Ok(end.to_owned()),
        }
    }
}

/// The item `#N` names, counted from 0.
fn slot_of(reference: &str) -> Option<usize> {
    reference
        .strip_prefix('#')?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

const fn word(kind: AddKind) -> &'static str {
    match kind {
        AddKind::Sticker => "a sticker",
        AddKind::Text => "a text",
        AddKind::Shape => "a shape",
        AddKind::Frame => "a frame",
        AddKind::Arrow => "an arrow",
        AddKind::Link => "a link",
        AddKind::Image => "an image",
        AddKind::File => "a file",
    }
}

/// How the item draws an arrow: its line, heads, dash and width.
fn arrow_look(item: &AddItem) -> Result<ArrowLook> {
    Ok(ArrowLook {
        arrow: ArrowEdit {
            line: item.line,
            heads: item
                .heads
                .as_deref()
                .map(str::parse)
                .transpose()?
                .unwrap_or_default(),
            ..ArrowEdit::default()
        },
        outline: ShapeEdit {
            stroke_width: item.stroke_width,
            stroke_style: item.stroke_style,
            ..ShapeEdit::default()
        },
    })
}

/// The frame a link to the board names, or the one the linked object sits in.
fn linked_frame<'a>(scene: &Scene<'a>, link: &str) -> Option<&'a Object> {
    let linked = id::object_in(link).and_then(|object| scene.get(object.as_str()))?;
    let frame =
        |object: &'a Object| (object.kind == ObjectKind::Frame).then_some(object);
    frame(linked).or_else(|| {
        linked
            .parent
            .as_deref()
            .and_then(|parent| scene.get(parent))
            .and_then(frame)
    })
}

/// The new objects in order: a corner is found for those without one, among
/// the objects there and the ones before in the list. A frame `#N` is the
/// N-th item, a frame made in the same list, and is kept as `#N`.
fn entries(
    items: &[Item],
    got: &[Fetched],
    scene: &Scene<'_>,
    linked: Option<&Object>,
) -> Result<Vec<NewEntry>> {
    let mut placed: Vec<Rectangle> = Vec::new();
    // The board rectangle of each item that is a frame, by slot.
    let mut frames: Vec<Option<Rectangle>> = Vec::new();
    items
        .iter()
        .enumerate()
        .map(|(place, item)| {
            let fresh = item.frame.as_deref().filter(|name| name.starts_with('#'));
            let (within, named) = frames_of(item, scene, &frames)?;
            let color = item.color.clone();
            let mut spot =
                |at: Option<Point>, size: Option<Size>, kind: &ObjectKind| {
                    let at = Placement {
                        scene,
                        frame: named.or(linked),
                        kind,
                        size,
                        within,
                    }
                    .find(at, &placed)?;
                    placed.push(at.1);
                    Ok::<_, Error>(at.0)
                };
            let new = match &item.what {
                What::Sticker { text, at, size } => NewObject::Sticker {
                    text: text.clone(),
                    at: spot(*at, *size, &ObjectKind::Sticker)?,
                    size: *size,
                    color,
                },
                What::Text { text, at } => NewObject::Text {
                    text: text.clone(),
                    at: spot(*at, None, &ObjectKind::Text)?,
                },
                What::Shape {
                    text,
                    at,
                    size,
                    kind,
                } => NewObject::Shape {
                    text: text.clone(),
                    at: spot(*at, *size, &ObjectKind::Shape)?,
                    size: *size,
                    color,
                    kind: *kind,
                },
                What::Frame { title, at, size } => NewObject::Frame {
                    title: title.clone(),
                    at: spot(*at, *size, &ObjectKind::Frame)?,
                    size: *size,
                    color,
                },
                What::Arrow { from, to, look } => NewObject::Arrow {
                    from: from.clone(),
                    to: to.clone(),
                    color,
                    look: look.clone(),
                },
                What::Link { url, at, embed } => NewObject::Link {
                    url: url.clone(),
                    at: spot(*at, None, &ObjectKind::Link)?,
                    preview: page(got.get(place)),
                    embed: *embed,
                },
                What::Image {
                    file,
                    pixels,
                    size,
                    at,
                } => NewObject::Image {
                    upload: ImageUpload {
                        file: held(file, got.get(place)),
                        pixels: *pixels,
                    },
                    at: spot(*at, Some(*size), &ObjectKind::Image)?,
                    size: Some(*size),
                },
                What::File { file, at } => NewObject::File {
                    upload: held(file, got.get(place)),
                    at: spot(*at, None, &ObjectKind::File)?,
                },
            };
            frames.push(
                matches!(item.what, What::Frame { .. })
                    .then(|| placed.last().copied())
                    .flatten(),
            );
            // An arrow goes into a frame only when asked, else where its ends are.
            let frame = if matches!(item.what, What::Arrow { .. }) {
                named
            } else {
                named.or(linked)
            };
            Ok(NewEntry {
                new,
                frame: fresh
                    .map(str::to_owned)
                    .or_else(|| frame.map(|frame| frame.id.clone())),
                format: item.format,
            })
        })
        .collect()
}

/// The frame an item sits in: the rectangle of one made earlier in the list
/// (`#N`), or the one on the board it names.
fn frames_of<'a>(
    item: &Item,
    scene: &Scene<'a>,
    made: &[Option<Rectangle>],
) -> Result<(Option<Rectangle>, Option<&'a Object>)> {
    let fresh = item.frame.as_deref().filter(|name| name.starts_with('#'));
    let within = fresh
        .map(|name| {
            slot_of(name)
                .and_then(|slot| made.get(slot).copied().flatten())
                .ok_or_else(|| not_value(COMMAND, "frame", "#N of an earlier frame"))
        })
        .transpose()?;
    let named = item
        .frame
        .as_deref()
        .filter(|_| fresh.is_none())
        .map(|name| scene.frame(name))
        .transpose()?;
    Ok((within, named))
}

/// The preview a link got, if any.
fn page(got: Option<&Fetched>) -> Option<LinkPreview> {
    match got {
        Some(Fetched::Page(preview)) => preview.clone(),
        Some(Fetched::Nothing | Fetched::Upload(_)) | None => None,
    }
}

/// The file as the server holds it; as it will be named when none was sent.
fn held(file: &Local, got: Option<&Fetched>) -> Upload {
    match got {
        Some(Fetched::Upload(done)) => done.clone(),
        Some(Fetched::Nothing | Fetched::Page(_)) | None => {
            file.upload(file.name.clone())
        }
    }
}

/// Where a new box object goes.
struct Placement<'a> {
    scene: &'a Scene<'a>,
    frame: Option<&'a Object>,
    kind: &'a ObjectKind,
    size: Option<Size>,
    /// The rectangle of a frame made in the same list; wins over `frame`.
    within: Option<Rectangle>,
}

impl Placement<'_> {
    /// The corner asked for, else a free one, and the board rectangle taken;
    /// the corner is from the frame's, if there is one.
    fn find(
        &self,
        at: Option<Point>,
        placed: &[Rectangle],
    ) -> Result<(Point, Rectangle)> {
        let size = self
            .size
            .or_else(|| self.kind.default_size())
            .unwrap_or(Size::STICKER);
        let corner = self
            .within
            .or_else(|| self.frame.and_then(|frame| self.scene.rectangle_of(frame)));
        let at = match (at, self.within) {
            (Some(at), _) => at,
            (None, Some(area)) => {
                let taken: Vec<Rectangle> = self
                    .scene
                    .objects()
                    .iter()
                    .filter_map(|object| self.scene.covered_by(object))
                    .chain(placed.iter().copied())
                    .filter(|other| area.intersects(other) && *other != area)
                    .collect();
                free_corner(area, &taken, size)?.shifted(-area.x, -area.y)?
            }
            (None, None) => free_spot_among(
                self.scene,
                self.frame.filter(|_| corner.is_some()),
                size,
                placed,
            )?,
        };
        let origin = match corner {
            Some(corner) => at.shifted(corner.x, corner.y)?,
            None => at,
        };
        Ok((
            at,
            Rectangle::new(origin.x(), origin.y(), size.width(), size.height()),
        ))
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "tests build and read real files in temp directories"
    )]
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use std::sync::Arc;

    use serde_json::{Map, Value, json};
    use yrs::{Any, Doc, Map as _, ReadTxn as _, StateVector, Transact as _};

    use super::{Fetched, add, added, entries, fetched, linked_frame, read_items};
    use crate::{
        Result,
        command::{
            board::show::Loading,
            board_load::CacheUse,
            fixture::{channel, client, directory, signed_in_hud},
            object::AddArgs,
            table::parse,
        },
        domain::{
            area::scene::Scene,
            board::{EditRight, Object, ObjectKind, object_of},
            change::value::{NewObject, Upload},
            response::LinkPreview,
        },
        effect::{http::HttpResponse, settings::CacheSettings},
        protocol::hud::{DocumentResponse, UpdatePayload},
        remote::{
            client::{
                Client,
                fake::{Fake, reply},
            },
            socket::{
                fake::{BOARD, Scripted, answer, script, signed_in, update},
                write::WriteOver,
            },
        },
    };

    fn args(given: Value) -> Result<AddArgs> {
        let mut arguments = match given {
            Value::Object(map) => map,
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => Map::new(),
        };
        arguments.entry("board").or_insert_with(|| json!(BOARD));
        parse::<AddArgs>(arguments)
    }

    fn objects() -> Vec<Value> {
        let at = |x: i32, y: i32| json!({"x": x, "y": y});
        vec![
            json!({"id": "f1", "type": "frame", "position": at(0, 0), "width": 800,
                   "height": 600, "labelText": "Q3"}),
            json!({"id": "s1", "type": "sticker", "position": at(24, 24), "width": 192,
                   "height": 192, "parentId": "f1"}),
        ]
    }

    /// The corner and the frame of a new object.
    type Placed = (Option<(f64, f64)>, Option<String>);

    fn on_board() -> Vec<Object> {
        objects().into_iter().filter_map(object_of).collect()
    }

    /// The corner and frame of each new object, for `given` on the board.
    fn placed(given: Value) -> Result<Vec<Placed>> {
        placed_on(&on_board(), given)
    }

    fn placed_on(board: &[Object], given: Value) -> Result<Vec<Placed>> {
        let scene = Scene::new(board);
        let args = args(given)?;
        let found = entries(
            &read_items(&args)?,
            &[],
            &scene,
            linked_frame(&scene, &args.board),
        )?;
        Ok(found
            .into_iter()
            .map(|entry| {
                let corner = match entry.new {
                    NewObject::Sticker { at, .. }
                    | NewObject::Text { at, .. }
                    | NewObject::Shape { at, .. }
                    | NewObject::Frame { at, .. }
                    | NewObject::Link { at, .. }
                    | NewObject::Image { at, .. }
                    | NewObject::File { at, .. } => Some((at.x(), at.y())),
                    NewObject::Arrow { .. } => None,
                };
                (corner, entry.frame)
            })
            .collect())
    }

    #[test]
    fn items_without_a_corner_take_free_corners_one_after_another() -> Result<()> {
        let two = json!({"frame": "Q3", "items": [{"text": "a"}, {"text": "b"}]});
        let loose = json!({"items": [{"text": "a"}, {"text": "b"}]});
        let (inside, outside) = (placed(two)?, placed(loose)?);
        let frame = Some("f1".to_owned());
        assert_eq!(
            (
                inside,
                outside.len(),
                outside.first() != outside.get(1),
                outside.iter().all(|(_, framed)| framed.is_none())
            ),
            (
                vec![
                    (Some((240.0, 24.0)), frame.clone()),
                    (Some((456.0, 24.0)), frame)
                ],
                2,
                true,
                true
            )
        );
        Ok(())
    }

    #[test]
    fn a_link_to_an_object_in_a_frame_is_the_default_frame() -> Result<()> {
        let link = format!("https://app.holst.so/board/{BOARD}?objectId=s1");
        let found = placed(json!({"board": link, "text": "x"}))?;
        assert_eq!(found, vec![(Some((240.0, 24.0)), Some("f1".to_owned()))]);
        Ok(())
    }

    #[test]
    fn a_given_corner_is_kept_and_taken_by_the_next() -> Result<()> {
        let found = placed(json!({"frame": "f1", "items": [
            {"text": "a", "at": "240,24"}, {"text": "b"}]}))?;
        assert_eq!(
            found.iter().map(|(corner, _)| *corner).collect::<Vec<_>>(),
            vec![Some((240.0, 24.0)), Some((456.0, 24.0))]
        );
        Ok(())
    }

    #[test]
    fn what_an_item_cannot_take_is_refused_with_its_number() -> Result<()> {
        let refused = |given: Value| -> Result<String> {
            Ok(read_items(&args(given)?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let arrow =
            |from: &str, to: &str| json!({"kind": "arrow", "from": from, "to": to});
        assert_eq!(
            [
                refused(json!({"text": "a", "items": [{"text": "b"}]}))?,
                refused(json!({"items": [{"text": "a"}, arrow("#3", "#1")]}))?,
                refused(json!({"items": [{"kind": "arrow", "from": "x"}]}))?,
                refused(json!({"items": [{"text": "a", "from": "x"}]}))?,
                refused(
                    json!({"items": [{"kind": "text", "text": "a", "size": "10x10"}]})
                )?,
                refused(json!({"items": [{"kind": "frame"}, {"kind": "sticker"}]}))?,
            ],
            [
                "text of `object add` must be left out with --items",
                "from of `object add` must be an object, or #N of an earlier item that is not an arrow, in item 2",
                "to of `object add` must be given, in item 1",
                "from of `object add` must be left out for a sticker, in item 1",
                "size of `object add` must be left out for a text, in item 1",
                "text of `object add` must be given, in item 2",
            ]
        );
        Ok(())
    }

    #[test]
    fn an_arrow_ends_at_an_earlier_item_that_is_not_an_arrow() {
        let items = |second: Value| {
            args(json!({"items": [{"text": "a"}, {"text": "b"}, second]}))
                .and_then(|args| read_items(&args))
        };
        let ok = items(json!({"kind": "arrow", "from": "#1", "to": "#2"}));
        let forward = items(json!({"kind": "arrow", "from": "#1", "to": "#4"}));
        let to_arrow = args(json!({"items": [
            {"text": "a"}, {"kind": "arrow", "from": "#1", "to": "f1"},
            {"kind": "arrow", "from": "#1", "to": "#2"}]}))
        .and_then(|args| read_items(&args));
        assert_eq!(
            (ok.is_ok(), forward.is_err(), to_arrow.is_err()),
            (true, true, true)
        );
    }

    #[test]
    fn a_frame_hash_n_is_the_frame_made_in_the_same_list() -> Result<()> {
        let mut board = on_board();
        board.extend(object_of(json!({"id": "f-sprint", "type": "frame",
            "position": {"x": 0, "y": 900}, "width": 800, "height": 600,
            "labelText": "Sprint 1"})));
        let found = placed_on(
            &board,
            json!({"items": [
                {"kind": "frame", "text": "New", "at": "2000,0", "size": "800x600"},
                {"text": "x", "frame": "#1"},
                {"text": "y", "frame": "#1"}]}),
        )?;
        let hash = Some("#1".to_owned());
        assert_eq!(
            found,
            vec![
                (Some((2000.0, 0.0)), None),
                (Some((24.0, 24.0)), hash.clone()),
                (Some((240.0, 24.0)), hash)
            ]
        );
        Ok(())
    }

    #[test]
    fn a_frame_hash_n_must_name_an_earlier_frame() -> Result<()> {
        let refused = |given: Value| -> Result<String> {
            Ok(read_items(&args(given)?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let expected = "frame of `object add` must be #N of an earlier frame, in item";
        let found = [
            refused(json!({"items": [{"text": "a", "frame": "#1"}]}))?,
            refused(json!({"items": [{"text": "a"}, {"text": "b", "frame": "#1"}]}))?,
            refused(json!({"items": [{"text": "b", "frame": "#1x"}]}))?,
        ];
        assert_eq!(found, [1, 2, 1].map(|slot| format!("{expected} {slot}")));
        Ok(())
    }

    #[test]
    fn items_given_but_empty_are_refused() -> Result<()> {
        let refused = read_items(&args(json!({"text": "hi", "items": []}))?)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(refused.contains("--items holds no objects"), "{refused}");
        assert_eq!(read_items(&args(json!({"text": "hi"}))?)?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn an_arrow_without_an_end_is_refused_before_any_request() -> Result<()> {
        let (http, client) = signed_in([])?;
        let arguments = args(json!({"kind": "arrow", "from": "x"}))?;
        let refused = add(&client, &CacheSettings::default(), &arguments)
            .await
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            (refused, http.requests().len()),
            (Some("to is required by `object add`".to_owned()), 0)
        );
        Ok(())
    }

    fn board_document() -> Scripted {
        document_with(EditRight::Editable)
    }

    fn document_with(right: EditRight) -> Scripted {
        let doc = Doc::with_client_id(4);
        let store = doc.get_or_insert_map("objects");
        {
            // imperative: yrs inserts through one `&mut` transaction
            let mut txn = doc.transact_mut();
            // imperative: yrs inserts through one `&mut` transaction
            for fields in objects() {
                let id = fields.get("id").and_then(Value::as_str).unwrap_or_default();
                let any = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
                store.insert(&mut txn, id, any);
            }
        }
        let state = doc
            .transact()
            .encode_state_as_update_v2(&StateVector::default());
        script([Some(vec![
            answer(DocumentResponse::Subscribed { right }),
            update(0, UpdatePayload::Inline(state)),
        ])])
    }

    #[tokio::test]
    async fn mixed_kinds_are_one_dry_run_write_in_the_order_given() -> Result<()> {
        let (_, client) = signed_in_hud()?;
        let arguments = args(json!({"kind": "shape", "items": [
            {"text": "Start", "shape": "diamond"},
            {"kind": "sticker", "text": "Ask", "color": "blue3"},
            {"kind": "arrow", "from": "#1", "to": "#2"},
            {"kind": "frame", "text": "Plan", "at": "1000,0", "size": "400x300"}]}))?;
        let report = added(
            &client,
            Loading {
                connector: &board_document(),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &WriteOver::reporter(&board_document()),
            &arguments,
            &read_items(&arguments)?,
        )
        .await?;
        let kinds: Vec<&ObjectKind> =
            report.objects.iter().map(|object| &object.kind).collect();
        assert_eq!(
            (
                kinds,
                report.receipt.delivery,
                report.objects.iter().all(|object| object.link.is_some()),
                report.objects.first().map(|object| object.id.clone()),
            ),
            (
                vec![
                    &ObjectKind::Shape,
                    &ObjectKind::Sticker,
                    &ObjectKind::Arrow,
                    &ObjectKind::Frame
                ],
                crate::domain::change::receipt::Delivery::Built,
                true,
                report.objects.first().map(|object| object.id.clone()),
            )
        );
        Ok(())
    }

    /// A PNG header of `width` by `height` pixels.
    fn png(width: u32, height: u32) -> Vec<u8> {
        [
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13][..],
            b"IHDR",
            &width.to_be_bytes(),
            &height.to_be_bytes(),
            &[8, 2, 0, 0, 0],
        ]
        .concat()
    }

    /// A new file in a folder of its own: its path.
    fn local(name: &str, bytes: &[u8]) -> Result<String> {
        let path = directory()?.join(name);
        std::fs::write(&path, bytes)?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// A client whose server answers the load, then `between`, then the
    /// account and the board of the write.
    fn serving(between: Vec<HttpResponse>) -> Result<(Arc<Fake>, Client)> {
        client(std::iter::once(channel()).chain(between).chain([
            reply(200, &[], r#"{"id":"me"}"#),
            reply(200, &[], r#"{"id":"b","in_trash":null}"#),
        ]))
    }

    fn error_of<T>(result: Result<T>) -> Option<String> {
        result.err().map(|error| error.to_string())
    }

    /// Whether a link card is embedded, and whether its preview offers it.
    fn embedding(new: NewObject) -> Option<(bool, bool)> {
        match new {
            NewObject::Link { preview, embed, .. } => {
                Some((embed, preview.is_some_and(|page| page.embed.is_some())))
            }
            NewObject::Sticker { .. }
            | NewObject::Text { .. }
            | NewObject::Shape { .. }
            | NewObject::Frame { .. }
            | NewObject::Arrow { .. }
            | NewObject::Image { .. }
            | NewObject::File { .. } => None,
        }
    }

    /// The width and height an image is placed at.
    fn shown_size(new: &NewObject) -> Option<(f64, f64)> {
        match new {
            NewObject::Image { size, .. } => {
                size.map(|shown| (shown.width(), shown.height()))
            }
            NewObject::Sticker { .. }
            | NewObject::Text { .. }
            | NewObject::Shape { .. }
            | NewObject::Frame { .. }
            | NewObject::Arrow { .. }
            | NewObject::Link { .. }
            | NewObject::File { .. } => None,
        }
    }

    #[test]
    fn an_embed_preview_is_embedded_unless_no_embed() -> Result<()> {
        let offered: LinkPreview = serde_json::from_value(
            json!({"title": "Clip", "embedInfo": {"type": "video", "providerName": "YouTube"}}),
        )?;
        let embedded = |given: Value| -> Result<Option<(bool, bool)>> {
            let items = read_items(&args(given)?)?;
            let scene_objects = on_board();
            let scene = Scene::new(&scene_objects);
            let found = entries(
                &items,
                &[Fetched::Page(Some(offered.clone()))],
                &scene,
                None,
            )?;
            Ok(found
                .into_iter()
                .next()
                .and_then(|entry| embedding(entry.new)))
        };
        let url = "https://youtu.be/x";
        assert_eq!(
            (
                embedded(json!({"kind": "link", "url": url}))?,
                embedded(json!({"kind": "link", "url": url, "no_embed": true}))?,
            ),
            (Some((true, true)), Some((false, true)))
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_file_gets_its_mime_type_and_size() -> Result<()> {
        let path = local("plan.pdf", b"%PDF-")?;
        let items = read_items(&args(json!({"kind": "file", "path": path}))?)?;
        let (_, client) = client(vec![reply(200, &[], r#"{"name":"u-2.pdf"}"#)])?;
        let got = fetched(&client, BOARD, true, &items).await?;
        assert_eq!(
            got,
            vec![Fetched::Upload(Upload {
                name: "u-2.pdf".to_owned(),
                original: "plan.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                bytes: 5,
            })]
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_missing_file_is_refused_before_any_request() -> Result<()> {
        let (fake, client) = serving(Vec::new())?;
        let path = directory()?.join("gone.png");
        let refused = add(
            &client,
            &CacheSettings::default(),
            &args(json!({"kind": "file", "path": path.to_string_lossy()}))?,
        )
        .await;
        assert_eq!(
            (error_of(refused), fake.requests().len()),
            (Some(format!("could not read {}", path.display())), 0)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_text_file_as_image_is_refused_before_any_request() -> Result<()> {
        let (fake, client) = serving(Vec::new())?;
        let path = local("notes.txt", b"hello")?;
        let refused = add(
            &client,
            &CacheSettings::default(),
            &args(json!({"kind": "image", "path": path}))?,
        )
        .await;
        assert_eq!(
            (error_of(refused), fake.requests().len()),
            (
                Some(
                    "path of `object add` must be a png, jpg, gif or webp picture"
                        .to_owned()
                ),
                0
            )
        );
        Ok(())
    }

    #[test]
    fn a_big_image_fits_in_1000_units() -> Result<()> {
        let shown = |width: u32,
                     height: u32,
                     asked: Option<&str>|
         -> Result<Option<(f64, f64)>> {
            let path = local("big.png", &png(width, height))?;
            let given = asked.map_or_else(
                || json!({"kind": "image", "path": path}),
                |text| json!({"kind": "image", "path": path, "size": text}),
            );
            let board = on_board();
            let found =
                entries(&read_items(&args(given)?)?, &[], &Scene::new(&board), None)?;
            Ok(found
                .into_iter()
                .next()
                .and_then(|entry| shown_size(&entry.new)))
        };
        assert_eq!(
            (
                shown(2000, 500, None)?,
                shown(500, 3000, None)?,
                shown(16, 16, None)?,
                shown(2000, 500, Some("300x200"))?,
            ),
            (
                Some((1000.0, 250.0)),
                Some((167.0, 1000.0)),
                Some((16.0, 16.0)),
                Some((300.0, 200.0)),
            )
        );
        Ok(())
    }
}
