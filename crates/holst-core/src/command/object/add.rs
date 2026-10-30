//! `object add`: new objects of every kind the CLI can place.

use std::path::Path;

use futures_util::{StreamExt as _, TryStreamExt as _, stream};

use crate::{
    command::{
        board::show::{Loading, board_id, through_cache},
        board_load::CacheUse,
        board_write::{BoardWrite, change_board},
        object::{
            AddArgs,
            AddFields,
            AddItem,
            AddKind,
            change::{load_board, refuse_hiding},
        },
        table::{ArgumentGroup as _, not_value},
    },
    domain::{
        area::scene::Scene,
        board::{
            EditRight,
            Object,
            ObjectKind,
            frame_look::FrameRatio,
            objects as board_objects,
            shape_type::ShapeType,
        },
        change::{
            receipt::ChangeReport,
            value::{
                ArrowEdit,
                ArrowLook,
                BoardChange,
                FrameEdit,
                ImageUpload,
                Made,
                NewEntry,
                NewObject,
                ShapeEdit,
                Upload,
            },
        },
        geometry::{Point, Rectangle, Size},
        id,
        image::pixel_size,
        kinds::{
            dice,
            flipcard,
            mind_map::{self, Direction, Under},
            sticker_stack,
            wheel,
        },
        lookup::rules::find_id,
        palette::Color,
        placement::{free_corner, free_spot_among},
        response::LinkPreview,
        rich_text::TextFormat,
        upload::{UploadKind, file_type},
    },
    effect::{board_write::WriteMode, confirmation::ALWAYS, settings::CacheSettings},
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
    },
    storage::file,
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
    let items = read_items(args, &|| client.new_id())?;
    through_cache(client, cache, CacheUse::Use, async |loading| {
        added(client, loading, args, &items).await
    })
    .await
}

async fn added(
    client: &Client,
    loading: Loading<'_>,
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
    let loaded = load_board(client, &loading, &board).await?;
    // Before any upload: a board that cannot be changed takes no file.
    match loaded.right {
        EditRight::Editable => Ok(()),
        EditRight::ReadOnly => Err(Error::BoardChange(BoardChangeError::ReadOnly)),
    }?;
    let hides = items.iter().any(
        |item| matches!(&item.what, What::Frame { look, .. } if look.hidden.is_some()),
    );
    if hides {
        refuse_hiding(client, &board).await?;
    }
    let objects = board_objects(&loaded.document);
    let scene = Scene::new(&objects);
    // Before any upload: a frame named twice is refused with nothing sent.
    items
        .iter()
        .filter_map(|item| item.frame.as_deref())
        .filter(|name| !name.starts_with('#'))
        .try_for_each(|name| scene.frame(name).map(|_| ()))?;
    let got = fetched(
        client,
        &board,
        matches!(args.dry_run, WriteMode::Send),
        items,
    )
    .await?;
    let change = BoardChange::AddAll(entries(
        items,
        &got,
        &scene,
        linked_frame(&scene, &args.board),
    )?);
    let report = change_board(
        client,
        loading,
        BoardWrite {
            board: &board,
            change,
            release: ALWAYS,
            guard: None,
            mode: args.dry_run,
        },
    )
    .await?;
    Ok(ChangeReport {
        warnings: report
            .warnings
            .into_iter()
            .chain(warnings(items, &got))
            .collect(),
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
                | What::File { .. }
                | What::Node { .. }
                | What::Made { .. } => Ok(Fetched::Nothing),
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
        look: FrameEdit,
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
    /// A node of a mind map: a root, or a child of `parent`, an object as
    /// given.
    Node {
        branch: Branch,
        at: Option<Point>,
    },
    /// An object its kind's own code builds; `at` is asked for, else found.
    Made {
        made: Made,
        at: Option<Point>,
    },
}

/// Where a mind-map node hangs: from `parent` on the `direction` side, or a
/// root.
#[derive(Debug, PartialEq)]
struct Branch {
    text: String,
    parent: Option<String>,
    direction: Direction,
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
fn read_items(args: &AddArgs, ids: &dyn Fn() -> String) -> Result<Vec<Item>> {
    let kind = args.kind.unwrap_or_default();
    let listed = args.items.as_ref().map(|items| items.0.as_slice());
    match listed {
        Some([]) => Err(not_value(
            COMMAND,
            "items",
            "a list of at least one object: --items holds no objects",
        )),
        _ => Ok(()),
    }?;
    let many = listed.is_some();
    args.add
        .first_given()
        .filter(|_| many)
        .map_or(Ok(()), |name| {
            Err(not_value(COMMAND, name, "left out with --items"))
        })?;
    let own = [AddItem {
        kind: None,
        frame: None,
        add: args.add.clone(),
    }];
    let asked: &[AddItem] = listed.unwrap_or(&own);
    let format = args.text_format.unwrap_or_default();
    asked
        .iter()
        .enumerate()
        .try_fold(Vec::new(), |mut read, (slot, item)| {
            let item = Reader {
                many,
                slot,
                format,
                ids,
            }
            .item(item, kind, args.frame.as_deref(), &read)?;
            read.push(item);
            Ok::<_, Error>(read)
        })
}

impl AddFields {
    /// The keys only an arrow takes, each with whether it is given.
    fn arrow_keys(&self) -> [(&'static str, bool); 7] {
        [
            ("from", self.from.is_some()),
            ("to", self.to.is_some()),
            ("line", self.line.is_some()),
            ("heads", self.heads.is_some()),
            ("stroke_style", self.stroke_style.is_some()),
            ("stroke_width", self.stroke_width.is_some()),
            ("label", !self.label.is_empty()),
        ]
    }

    /// The keys only a frame takes, each with whether it is given.
    fn frame_keys(&self) -> [(&'static str, bool); 3] {
        [
            ("ratio", self.ratio.is_some()),
            ("background", self.background.is_some()),
            ("hide_content", self.hide_content),
        ]
    }
}

/// Reads the values of one item, which is the `slot`-th of `--items` if `many`.
struct Reader<'a> {
    many: bool,
    slot: usize,
    format: TextFormat,
    /// Draws the ids a kind gives its parts.
    ids: &'a dyn Fn() -> String,
}

impl Reader<'_> {
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
        let rich = matches!(
            kind,
            AddKind::Sticker | AddKind::Text | AddKind::Shape | AddKind::MindMap
        );
        match (rich, self.many, self.format) {
            (false, false, TextFormat::Markdown) => {
                Err(self.bad("text_format", &format!("left out for {word}")))
            }
            _ => Ok(rich),
        }
    }

    /// Refuses the first given key of `keys`, which only the `owner` kind
    /// takes.
    fn refuse_foreign(
        &self,
        kind: AddKind,
        owner: AddKind,
        keys: &[(&'static str, bool)],
    ) -> Result<()> {
        keys.iter()
            .find(|(_, given)| *given)
            .filter(|_| kind != owner)
            .map_or(Ok(()), |(name, _)| {
                Err(self.bad(name, &format!("left out for {}", word(kind))))
            })
    }

    /// Refuses the first key a mind-map node's parent or direction breaks:
    /// those of another kind, a corner or frame beside a parent, a direction
    /// without one.
    fn refuse_node_keys(
        &self,
        kind: AddKind,
        fields: &AddFields,
        framed: bool,
    ) -> Result<()> {
        let node = kind == AddKind::MindMap;
        let hanging = node && fields.parent.is_some();
        let foreign = format!("left out for {}", word(kind));
        [
            ("parent", !node && fields.parent.is_some(), foreign.as_str()),
            (
                "direction",
                !node && fields.direction.is_some(),
                foreign.as_str(),
            ),
            ("at", hanging && fields.at.is_some(), "left out with parent"),
            ("frame", hanging && framed, "left out with parent"),
            (
                "direction",
                node && !hanging && fields.direction.is_some(),
                "left out without parent",
            ),
        ]
        .into_iter()
        .find(|(_, given, _)| *given)
        .map_or(Ok(()), |(name, _, why)| Err(self.bad(name, why)))
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
        let fields = &item.add;
        let word = word(kind);
        let refuse = |name: &'static str, given: bool| {
            given
                .then(|| self.bad(name, &format!("left out for {word}")))
                .map_or(Ok(()), Err)
        };
        let arrow = kind == AddKind::Arrow;
        self.refuse_foreign(kind, AddKind::Arrow, &fields.arrow_keys())?;
        let linked = kind == AddKind::Link;
        let uploaded = matches!(kind, AddKind::Image | AddKind::File);
        let die = kind == AddKind::Dice;
        let wheel = kind == AddKind::Wheel;
        refuse(
            "text",
            (arrow || linked || uploaded || die || wheel) && fields.text.is_some(),
        )?;
        refuse("faces", !die && fields.faces.is_some())?;
        refuse("choice", !wheel && !fields.choice.is_empty())?;
        refuse("back", kind != AddKind::Flipcard && fields.back.is_some())?;
        self.refuse_node_keys(kind, fields, item.frame.is_some() || frame.is_some())?;
        refuse("at", arrow && fields.at.is_some())?;
        refuse("size", sizeless(kind) && fields.size.is_some())?;
        refuse("shape", kind != AddKind::Shape && fields.shape.is_some())?;
        refuse(
            "color",
            (linked || uploaded || wheel) && fields.color.is_some(),
        )?;
        refuse("url", !linked && fields.url.is_some())?;
        refuse("path", !uploaded && fields.path.is_some())?;
        let rich = self.rich(kind, word)?;
        refuse("no_preview", !linked && fields.no_preview)?;
        self.refuse_foreign(kind, AddKind::Frame, &fields.frame_keys())?;
        let at = fields.at.as_deref().map(str::parse).transpose()?;
        let size = fields.size.as_deref().map(str::parse).transpose()?;
        let color: Option<Color> =
            fields.color.as_deref().map(str::parse).transpose()?;
        let text = fields.text.clone();
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
                size: size.or_else(|| fields.shape.and_then(ShapeType::default_size)),
                kind: fields.shape.unwrap_or(ShapeType::Square),
            },
            AddKind::Frame => frame_of(fields, (at, size))?,
            AddKind::Arrow => What::Arrow {
                from: self.end("from", fields.from.as_deref(), earlier)?,
                to: self.end("to", fields.to.as_deref(), earlier)?,
                look: arrow_look(fields)?,
            },
            AddKind::Link => What::Link {
                url: fields.url.clone().ok_or_else(|| self.missing("url"))?,
                at,
                embed: !fields.no_preview,
            },
            AddKind::Image => self.image(fields.path.as_deref(), at, size)?,
            AddKind::File => self.file(fields.path.as_deref(), at)?,
            AddKind::Dice => What::Made {
                made: dice::made(self.faces(fields.faces)?, color.as_ref()),
                at,
            },
            AddKind::StickerStack => self.stack(fields, size, color.as_ref(), at)?,
            AddKind::Flipcard => self.card(fields, color.as_ref(), at)?,
            AddKind::Wheel => What::Made {
                made: wheel::made(self.choices(&fields.choice)?, self.ids),
                at,
            },
            AddKind::MindMap => What::Node {
                branch: Branch {
                    text: needed()?,
                    parent: fields.parent.clone(),
                    direction: fields.direction.unwrap_or_default(),
                },
                at,
            },
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
            color,
            frame: frame.map(str::to_owned),
            format: if rich { self.format } else { TextFormat::Plain },
        })
    }

    /// The faces asked for, or the default, which the web client offers.
    fn faces(&self, given: Option<u32>) -> Result<u32> {
        let faces = given.unwrap_or(dice::DEFAULT_FACES);
        dice::FACES
            .contains(&faces)
            .then_some(faces)
            .ok_or_else(|| self.bad("faces", "one of 2, 4, 6, 8, 10, 12 or 20"))
    }

    /// A flip card of the text and the back the call gives.
    fn card(
        &self,
        fields: &AddFields,
        color: Option<&Color>,
        at: Option<Point>,
    ) -> Result<What> {
        let front = fields.text.as_deref().ok_or_else(|| self.missing("text"))?;
        let back = fields.back.as_deref().ok_or_else(|| self.missing("back"))?;
        Ok(What::Made {
            made: flipcard::made(front, back, color),
            at,
        })
    }

    /// A sticker stack titled by the text, of one of the two widths.
    fn stack(
        &self,
        fields: &AddFields,
        size: Option<Size>,
        color: Option<&Color>,
        at: Option<Point>,
    ) -> Result<What> {
        Ok(What::Made {
            made: sticker_stack::made(
                fields.text.as_deref(),
                self.stack_size(size)?,
                color,
            ),
            at,
        })
    }

    /// The size of a stack: one of the two widths the web client offers.
    fn stack_size(&self, given: Option<Size>) -> Result<Size> {
        let size = given.unwrap_or(Size::STICKER);
        sticker_stack::WIDTHS
            .contains(&size.width())
            .then_some(size)
            .filter(|size| size.height() == sticker_stack::HEIGHT_OF)
            .ok_or_else(|| self.bad("size", "192,192 or 384,192"))
    }

    /// The choices of a wheel: how many and how long the web client takes.
    fn choices<'c>(&self, given: &'c [String]) -> Result<&'c [String]> {
        match given {
            _ if !(wheel::MIN_CHOICES..=wheel::MAX_CHOICES).contains(&given.len()) => {
                Err(self.bad(
                    "choice",
                    &format!(
                        "{} to {} choices",
                        wheel::MIN_CHOICES,
                        wheel::MAX_CHOICES
                    ),
                ))
            }
            _ if given
                .iter()
                .any(|label| label.chars().count() > wheel::MAX_LABEL) =>
            {
                Err(self.bad(
                    "choice",
                    &format!("labels of at most {} characters", wheel::MAX_LABEL),
                ))
            }
            _ => Ok(given),
        }
    }

    /// An image shown at `size` or at its pixels, fitted.
    fn image(
        &self,
        path: Option<&str>,
        at: Option<Point>,
        size: Option<Size>,
    ) -> Result<What> {
        let (name, bytes) = self.read(path)?;
        let (pixels, mime) = self.picture(&bytes)?;
        Ok(What::Image {
            size: size.map_or_else(|| fitted(pixels), Ok)?,
            pixels,
            at,
            file: Local { name, mime, bytes },
        })
    }

    fn file(&self, path: Option<&str>, at: Option<Point>) -> Result<What> {
        let (name, bytes) = self.read(path)?;
        Ok(What::File {
            at,
            file: Local {
                mime: file_type(&name),
                name,
                bytes,
            },
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

/// A frame with the ratio, background and hidden content the item asks for.
fn frame_of(
    item: &AddFields,
    (at, size): (Option<Point>, Option<Size>),
) -> Result<What> {
    let look = FrameEdit {
        ratio: item.ratio,
        background: item.background,
        hidden: item.hide_content.then_some(true),
    };
    Ok(What::Frame {
        title: item.text.clone().unwrap_or_default(),
        at,
        size: reshaped(look.ratio, size)?,
        look,
    })
}

/// The size a frame of `ratio` gets: the one asked for reshaped keeping its
/// area, else the ratio's own; the size asked for when there is no ratio.
fn reshaped(ratio: Option<FrameRatio>, size: Option<Size>) -> Result<Option<Size>> {
    ratio.map_or(Ok(size), |ratio| {
        size.map_or_else(
            || ratio.default_size(),
            |asked| ratio.fitted(asked.width(), asked.height()),
        )
        .map(Some)
    })
}

/// The item `#N` names, counted from 0.
fn slot_of(reference: &str) -> Option<usize> {
    reference
        .strip_prefix('#')?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

/// Whether the kind has a size of its own, which a call cannot change.
const fn sizeless(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Arrow
            | AddKind::Text
            | AddKind::Link
            | AddKind::File
            | AddKind::Dice
            | AddKind::Wheel
            | AddKind::Flipcard
            | AddKind::MindMap
    )
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
        AddKind::Dice => "a die",
        AddKind::Wheel => "a wheel",
        AddKind::StickerStack => "a sticker stack",
        AddKind::Flipcard => "a flip card",
        AddKind::MindMap => "a mind-map node",
    }
}

/// How the item draws an arrow: its line, heads, labels, dash and width.
fn arrow_look(item: &AddFields) -> Result<ArrowLook> {
    Ok(ArrowLook {
        arrow: ArrowEdit {
            line: item.line,
            heads: item
                .heads
                .as_deref()
                .map(str::parse)
                .transpose()?
                .unwrap_or_default(),
            labels: item
                .label
                .iter()
                .map(|raw| raw.parse())
                .collect::<Result<_>>()?,
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

/// A new mind-map node with its text, hung from `parent` if there is one; the
/// corner of a root is left to the caller.
fn hung_node(
    scene: &Scene<'_>,
    branch: &Branch,
    color: Option<&Color>,
) -> Result<Made> {
    let hung = branch
        .parent
        .as_deref()
        .map(|reference| parent_node(scene, reference))
        .transpose()?;
    let under = hung.map(|node| Under {
        scene,
        parent: node,
        direction: branch.direction,
    });
    mind_map::made(&branch.text, color, under.as_ref())
}

/// The mind-map node `reference` names on the board.
fn parent_node<'a>(scene: &Scene<'a>, reference: &str) -> Result<&'a Object> {
    let id = find_id(
        scene.objects().iter().map(|object| object.id.as_str()),
        reference,
    )?;
    scene
        .get(&id)
        .filter(|object| mind_map::is_node(object))
        .ok_or_else(|| not_value(COMMAND, "parent", "a mind-map node"))
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
                What::Frame {
                    title,
                    at,
                    size,
                    look,
                } => NewObject::Frame {
                    title: title.clone(),
                    at: spot(*at, *size, &ObjectKind::Frame)?,
                    size: *size,
                    color,
                    look: *look,
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
                What::Made { made, at } => NewObject::Made(Made {
                    at: spot(*at, made.size, &made.kind)?,
                    ..made.clone()
                }),
                What::Node { branch, at } => {
                    let node = hung_node(scene, branch, color.as_ref())?;
                    let fixed = branch.parent.as_ref().map(|_| node.at).or(*at);
                    let corner = spot(fixed, node.size, &node.kind)?;
                    NewObject::Made(Made { at: corner, ..node })
                }
            };
            frames.push(made_frame(item, &placed));
            Ok(entry_in(item, new, (named, linked)))
        })
        .collect()
}

/// The new object as an entry, in the frame it names or, but for an arrow,
/// the linked one; `#N` is kept as given.
fn entry_in(
    item: &Item,
    new: NewObject,
    (named, linked): (Option<&Object>, Option<&Object>),
) -> NewEntry {
    let fresh = item.frame.as_deref().filter(|name| name.starts_with('#'));
    // An arrow goes into a frame only when asked, else where its ends are.
    let frame = named.or(linked.filter(|_| !matches!(item.what, What::Arrow { .. })));
    NewEntry {
        new,
        frame: fresh
            .map(str::to_owned)
            .or_else(|| frame.map(|frame| frame.id.clone())),
        format: item.format,
    }
}

/// The rectangle of the frame the item made, as the last one placed.
fn made_frame(item: &Item, placed: &[Rectangle]) -> Option<Rectangle> {
    placed
        .last()
        .copied()
        .filter(|_| matches!(item.what, What::Frame { .. }))
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

    use std::{cell::Cell, sync::Arc};

    use serde_json::{Map, Value, json};
    use yrs::{Any, Doc, Map as _, ReadTxn as _, StateVector, Transact as _};

    use super::{
        Fetched,
        Item,
        add,
        added,
        entries,
        fetched,
        linked_frame,
        read_items,
    };
    use crate::{
        Error,
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
            change::value::{Made, NewObject, Upload},
            response::LinkPreview,
        },
        effect::{http::HttpResponse, settings::CacheSettings},
        error::lookup::{LookupError, LookupMiss},
        fake::png,
        protocol::hud::{DocumentResponse, UpdatePayload},
        remote::{
            client::{
                Client,
                fake::{Fake, reply},
            },
            socket::fake::{BOARD, Scripted, answer, script, signed_in, update},
        },
    };

    /// The items, each id drawn as `w1`, `w2`, ...
    fn read(args: &AddArgs) -> Result<Vec<Item>> {
        let drawn = Cell::new(0_u32);
        read_items(args, &|| {
            drawn.set(drawn.get() + 1);
            format!("w{}", drawn.get())
        })
    }

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
            &read(&args)?,
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
                    | NewObject::File { at, .. }
                    | NewObject::Made(Made { at, .. }) => Some((at.x(), at.y())),
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
            Ok(read(&args(given)?)
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
                    json!({"items": [{"kind": "text", "text": "a", "size": "10,10"}]})
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
    fn a_stack_takes_only_its_two_widths() -> Result<()> {
        let refused = |size: &str| -> Result<String> {
            Ok(read(&args(json!({"kind": "sticker-stack", "size": size}))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            refused("300,300")?,
            "size of `object add` must be 192,192 or 384,192"
        );
        assert_eq!(
            read(&args(json!({
                "kind": "sticker-stack", "text": "Ideas", "size": "384,192"
            }))?)
            .map(|found| found.len())
            .ok(),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn a_flip_card_needs_its_back_and_no_other_kind_takes_one() -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "flipcard", "text": "Q"}))?,
                refused(json!({"kind": "flipcard", "back": "A"}))?,
                refused(json!({"text": "a", "back": "A"}))?,
            ],
            [
                "back of `object add` must be given, in item 1",
                "text of `object add` must be given, in item 1",
                "back of `object add` must be left out for a sticker, in item 1",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_die_refuses_a_size_a_text_and_faces_the_web_does_not_offer() -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "dice", "size": "10,10"}))?,
                refused(json!({"kind": "dice", "text": "a"}))?,
                refused(json!({"kind": "dice", "faces": 7}))?,
                refused(json!({"text": "a", "faces": 6}))?,
            ],
            [
                "size of `object add` must be left out for a die, in item 1",
                "text of `object add` must be left out for a die, in item 1",
                "faces of `object add` must be one of 2, 4, 6, 8, 10, 12 or 20, in item 1",
                "faces of `object add` must be left out for a sticker, in item 1",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_die_is_made_with_its_faces_colour_and_corner() -> Result<()> {
        let args = args(json!({
            "kind": "dice", "faces": 20, "color": "red4", "at": "10,20"
        }))?;
        let scene = Scene::new(&[]);
        let found = entries(&read(&args)?, &[], &scene, None)?;
        let die = found.into_iter().find_map(|entry| made_of(entry.new));
        assert_eq!(
            die.map(|made| (
                made.kind,
                (made.at.x(), made.at.y()),
                made.fields.contains(&("faces", json!(20))),
                made.fields.iter().any(|(name, _)| *name == "fillColor"),
            )),
            Some((ObjectKind::Dice, (10.0, 20.0), true, true))
        );
        Ok(())
    }

    #[test]
    fn a_wheel_refuses_one_choice_too_many_and_a_long_label() -> Result<()> {
        let refused = |choices: Vec<String>| -> Result<String> {
            Ok(read(&args(json!({"kind": "wheel", "choice": choices}))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let many = |count: usize| vec!["a".to_owned(); count];
        assert_eq!(
            [
                refused(many(1))?,
                refused(many(21))?,
                refused(vec!["a".to_owned(), "b".repeat(41)])?,
                refused(many(20))?,
            ],
            [
                "choice of `object add` must be 2 to 20 choices",
                "choice of `object add` must be 2 to 20 choices",
                "choice of `object add` must be labels of at most 40 characters",
                "",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_wheel_refuses_a_text_a_size_a_color_and_choices_of_another_kind() -> Result<()>
    {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let two = json!(["a", "b"]);
        assert_eq!(
            [
                refused(json!({"kind": "wheel", "choice": two, "text": "a"}))?,
                refused(json!({"kind": "wheel", "choice": two, "size": "10,10"}))?,
                refused(json!({"kind": "wheel", "choice": two, "color": "red4"}))?,
                refused(json!({"text": "a", "choice": two}))?,
            ],
            [
                "text of `object add` must be left out for a wheel, in item 1",
                "size of `object add` must be left out for a wheel, in item 1",
                "color of `object add` must be left out for a wheel, in item 1",
                "choice of `object add` must be left out for a sticker, in item 1",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_wheel_is_made_with_the_choices_it_is_given_and_a_corner() -> Result<()> {
        let args = args(json!({
            "kind": "wheel", "choice": ["Pizza", "Sushi", "Soup"], "at": "10,20"
        }))?;
        let scene = Scene::new(&[]);
        let found = entries(&read(&args)?, &[], &scene, None)?;
        let wheel = found.into_iter().find_map(|entry| made_of(entry.new));
        assert_eq!(
            wheel.map(|made| (
                made.kind,
                (made.at.x(), made.at.y()),
                made.fields
                    .iter()
                    .find(|(name, _)| *name == "items")
                    .map(|(_, items)| items.clone()),
            )),
            Some((
                ObjectKind::Wheel,
                (10.0, 20.0),
                Some(json!([
                    {"id": "w1", "label": "Pizza"},
                    {"id": "w2", "label": "Sushi"},
                    {"id": "w3", "label": "Soup"},
                ]))
            ))
        );
        Ok(())
    }

    fn mind_map_board(children: &[Value]) -> Vec<Object> {
        let node = json!({"id": "m1", "type": "mind-map-node", "documentId": "d1",
            "position": {"x": 400, "y": 100}, "width": 200, "height": 80,
            "nodeType": 2, "nodeColor": {"color": "red4", "opacity": 1}});
        on_board()
            .into_iter()
            .chain(
                std::iter::once(node)
                    .chain(children.iter().cloned())
                    .filter_map(object_of),
            )
            .collect()
    }

    #[test]
    fn a_mind_map_child_goes_beside_its_node_after_the_children_there() -> Result<()> {
        let child = json!({"id": "m2", "type": "mind-map-node", "parentId": "m1",
            "position": {"x": 700, "y": 100}});
        let corner = |board: &[Object], direction: &str| -> Result<Vec<Placed>> {
            placed_on(
                board,
                json!({"kind": "mind-map", "text": "Budget", "parent": "m1",
                       "direction": direction}),
            )
        };
        let (first, second) = (mind_map_board(&[]), mind_map_board(&[child]));
        assert_eq!(
            [
                corner(&first, "right")?,
                corner(&second, "right")?,
                corner(&first, "left")?,
                corner(&first, "down")?,
                corner(&first, "up")?,
            ],
            [
                vec![(Some((700.0, 100.0)), None)],
                vec![(Some((700.0, 216.0)), None)],
                vec![(Some((140.0, 100.0)), None)],
                vec![(Some((400.0, 280.0)), None)],
                vec![(Some((400.0, -100.0)), None)],
            ]
        );
        Ok(())
    }

    #[test]
    fn a_mind_map_child_takes_the_look_of_its_node_and_a_color_of_its_own() -> Result<()>
    {
        let board = mind_map_board(&[]);
        let fields = |given: Value| -> Result<Option<Value>> {
            let args = args(given)?;
            let scene = Scene::new(&board);
            Ok(entries(&read(&args)?, &[], &scene, None)?
                .into_iter()
                .find_map(|entry| made_of(entry.new))
                .map(|made| {
                    json!({"parent": made.parent, "fields": made.fields.into_iter()
                        .map(|(name, value)| (name.to_owned(), value)).collect::<Map<_, _>>()})
                }))
        };
        assert_eq!(
            [
                fields(json!({"kind": "mind-map", "text": "a", "parent": "m1"}))?,
                fields(
                    json!({"kind": "mind-map", "text": "a", "parent": "m1", "color": "blue4"})
                )?,
            ],
            [
                Some(json!({"parent": "m1", "fields": {
                    "nodeType": 2, "nodeColor": {"color": "red4", "opacity": 1}}})),
                Some(json!({"parent": "m1", "fields": {
                    "nodeType": 2, "nodeColor": {"color": "blue4", "opacity": 1}}})),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_mind_map_parent_must_be_a_node_of_the_board() -> Result<()> {
        let refused = |parent: &str| -> Result<String> {
            Ok(placed_on(
                &mind_map_board(&[]),
                json!({"kind": "mind-map", "text": "a", "parent": parent}),
            )
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default())
        };
        assert_eq!(
            refused("s1")?,
            "parent of `object add` must be a mind-map node"
        );
        assert!(!refused("nowhere")?.is_empty());
        Ok(())
    }

    #[test]
    fn a_mind_map_refuses_what_it_cannot_take_and_what_only_it_takes() -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let node = |extra: Value| {
            let mut item = json!({"kind": "mind-map", "text": "a"});
            item.as_object_mut()
                .zip(extra.as_object())
                .into_iter()
                .for_each(|(item, extra)| item.extend(extra.clone()));
            item
        };
        assert_eq!(
            [
                refused(node(json!({"parent": "m1", "at": "1,2"})))?,
                refused(node(json!({"parent": "m1", "frame": "f1"})))?,
                refused(node(json!({"direction": "left"})))?,
                refused(node(json!({"size": "10,10"})))?,
                refused(json!({"text": "a", "parent": "m1"}))?,
                refused(json!({"text": "a", "direction": "left"}))?,
                refused(node(
                    json!({"parent": "m1", "direction": "left", "color": "red4"})
                ))?,
            ],
            [
                "at of `object add` must be left out with parent, in item 1",
                "frame of `object add` must be left out with parent, in item 1",
                "direction of `object add` must be left out without parent, in item 1",
                "size of `object add` must be left out for a mind-map node, in item 1",
                "parent of `object add` must be left out for a sticker, in item 1",
                "direction of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        Ok(())
    }

    fn made_of(new: NewObject) -> Option<Made> {
        match new {
            NewObject::Made(made) => Some(made),
            NewObject::Sticker { .. }
            | NewObject::Text { .. }
            | NewObject::Shape { .. }
            | NewObject::Frame { .. }
            | NewObject::Arrow { .. }
            | NewObject::Link { .. }
            | NewObject::Image { .. }
            | NewObject::File { .. } => None,
        }
    }

    #[test]
    fn an_arrow_ends_at_an_earlier_item_that_is_not_an_arrow() {
        let items = |second: Value| {
            args(json!({"items": [{"text": "a"}, {"text": "b"}, second]}))
                .and_then(|args| read(&args))
        };
        let ok = items(json!({"kind": "arrow", "from": "#1", "to": "#2"}));
        let forward = items(json!({"kind": "arrow", "from": "#1", "to": "#4"}));
        let to_arrow = args(json!({"items": [
            {"text": "a"}, {"kind": "arrow", "from": "#1", "to": "f1"},
            {"kind": "arrow", "from": "#1", "to": "#2"}]}))
        .and_then(|args| read(&args));
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
                {"kind": "frame", "text": "New", "at": "2000,0", "size": "800,600"},
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
            Ok(read(&args(given)?)
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
        let refused = read(&args(json!({"text": "hi", "items": []}))?)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(refused.contains("--items holds no objects"), "{refused}");
        assert_eq!(read(&args(json!({"text": "hi"}))?)?.len(), 1);
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
        document_of(objects(), right)
    }

    fn document_of(fields_of: Vec<Value>, right: EditRight) -> Scripted {
        let doc = Doc::with_client_id(4);
        let store = doc.get_or_insert_map("objects");
        {
            // imperative: yrs inserts through one `&mut` transaction
            let mut txn = doc.transact_mut();
            // imperative: yrs inserts through one `&mut` transaction
            for fields in fields_of {
                let id = fields.get("id").and_then(Value::as_str).unwrap_or_default();
                let any = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
                store.insert(&mut txn, id, any);
            }
        }
        let state = doc
            .transact()
            .encode_state_as_update_v2(&StateVector::default());
        let session = || {
            Some(vec![
                answer(DocumentResponse::Subscribed { right }),
                update(0, UpdatePayload::Inline(state.clone())),
            ])
        };
        script([session(), session()])
    }

    #[tokio::test]
    async fn mixed_kinds_are_one_dry_run_write_in_the_order_given() -> Result<()> {
        let (_, client) = signed_in_hud()?;
        let arguments = args(json!({"kind": "shape", "dry_run": true, "items": [
            {"text": "Start", "shape": "diamond"},
            {"kind": "sticker", "text": "Ask", "color": "blue3"},
            {"kind": "arrow", "from": "#1", "to": "#2"},
            {"kind": "frame", "text": "Plan", "at": "1000,0", "size": "400,300"}]}))?;
        let report = added(
            &client,
            Loading {
                connector: &board_document(),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &arguments,
            &read(&arguments)?,
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

    #[tokio::test]
    async fn a_dry_run_reports_what_stayed_text_and_the_links_without_preview()
    -> Result<()> {
        let (_, client) = serving(vec![reply(404, &[], "")])?;
        let arguments = args(json!({"dry_run": true, "text_format": "markdown",
            "items": [
                {"kind": "sticker", "text": "> quoted"},
                {"kind": "link", "url": "https://example.test/x"}]}))?;
        let report = added(
            &client,
            Loading {
                connector: &board_document(),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &arguments,
            &read(&arguments)?,
        )
        .await?;
        assert_eq!(
            report.warnings,
            ["kept as text: quote", "no preview: https://example.test/x"]
        );
        Ok(())
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
            | NewObject::File { .. }
            | NewObject::Made(_) => None,
        }
    }

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
            | NewObject::File { .. }
            | NewObject::Made(_) => None,
        }
    }

    #[test]
    fn an_embed_preview_is_embedded_unless_no_embed() -> Result<()> {
        let offered: LinkPreview = serde_json::from_value(
            json!({"title": "Clip", "embedInfo": {"type": "video", "providerName": "YouTube"}}),
        )?;
        let embedded = |given: Value| -> Result<Option<(bool, bool)>> {
            let items = read(&args(given)?)?;
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
                embedded(json!({"kind": "link", "url": url, "no_preview": true}))?,
            ),
            (Some((true, true)), Some((false, true)))
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_file_gets_its_mime_type_and_size() -> Result<()> {
        let path = local("plan.pdf", b"%PDF-")?;
        let items = read(&args(json!({"kind": "file", "path": path}))?)?;
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
    async fn frames_named_alike_are_refused_before_any_upload() -> Result<()> {
        let (fake, client) = serving(vec![reply(200, &[], r#"{"name":"u-2.pdf"}"#)])?;
        let path = local("plan.pdf", b"%PDF-")?;
        let arguments = args(json!({"kind": "file", "path": path, "frame": "Q3"}))?;
        let twin = json!({"id": "f2", "type": "frame", "position": {"x": 900, "y": 0},
            "width": 800, "height": 600, "labelText": "Q3"});
        let refused = added(
            &client,
            Loading {
                connector: &document_of(
                    objects().into_iter().chain([twin]).collect(),
                    EditRight::Editable,
                ),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &arguments,
            &read(&arguments)?,
        )
        .await;
        let uploads = fake
            .requests()
            .iter()
            .filter(|request| request.url.path().contains("upload"))
            .count();
        assert_eq!(
            (
                matches!(
                    refused,
                    Err(Error::Lookup(LookupError {
                        miss: LookupMiss::Ambiguous { .. },
                        ..
                    }))
                ),
                uploads
            ),
            (true, 0)
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
            let found = entries(&read(&args(given)?)?, &[], &Scene::new(&board), None)?;
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
                shown(2000, 500, Some("300,200"))?,
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
