//! `object add`: new objects of every kind the CLI can place.

use std::{path::Path, pin::pin, time::Duration};

use futures_util::{StreamExt as _, TryStreamExt as _, future, stream};
use serde_json::Value;
use yrs::Doc;

use crate::{
    command::{
        board::show::{Loading, board_id, through_cache},
        board_load::CacheUse,
        board_write::{BoardWrite, change_board_with},
        object::{
            AddArgs,
            AddFields,
            AddItem,
            AddKind,
            change::{load_board_read, refuse_hiding},
        },
        stored_file::download,
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
            build::author_number,
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
            code,
            dice,
            document,
            flipcard,
            icon,
            kanban,
            mind_map::{self, Direction, Under},
            reaction_stamp::{self, Target},
            stamp,
            sticker_stack,
            table,
            task::{self, Day},
            wheel,
        },
        lookup::rules::find_id,
        palette::{self, Color},
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
    let (loaded, read) = load_board_read(client, &loading, &board).await?;
    // Before any upload: a board that cannot be changed takes no file.
    match loaded.right {
        EditRight::Editable => Ok(()),
        EditRight::ReadOnly => Err(Error::BoardChange(BoardChangeError::ReadOnly)),
    }?;
    let hides = items.iter().any(
        |item| matches!(&item.what, What::Frame { look, .. } if look.hidden.is_some()),
    );
    if hides {
        refuse_hiding(client, &board, read.as_ref()).await?;
    }
    let objects = board_objects(&loaded.document);
    let scene = Scene::new(&objects);
    // Before any upload: a frame named twice is refused with nothing sent.
    items
        .iter()
        .filter_map(|item| item.frame.as_deref())
        .filter(|name| !name.starts_with('#'))
        .try_for_each(|name| scene.frame(name).map(|_| ()))?;
    // Before any upload: an icon the set lacks is refused with nothing sent.
    let unchecked = icons_checked(client, items).await?;
    // Before any upload: what the tasks and the places can refuse is refused.
    let unfetched: Vec<Fetched> = items.iter().map(|_| Fetched::Nothing).collect();
    let planned = with_tasks(items, unfetched, &loaded.document, &scene)?;
    entries(items, &planned, &scene, linked_frame(&scene, &args.board))?;
    let got = fetched(
        client,
        &board,
        matches!(args.dry_run, WriteMode::Send),
        items,
    )
    .await?;
    let got = with_tasks(items, got, &loaded.document, &scene)?;
    let change = BoardChange::AddAll(entries(
        items,
        &got,
        &scene,
        linked_frame(&scene, &args.board),
    )?);
    let report = change_board_with(
        client,
        loading,
        BoardWrite {
            board: &board,
            change,
            release: ALWAYS,
            guard: None,
            mode: args.dry_run,
        },
        read,
    )
    .await?;
    Ok(ChangeReport {
        warnings: report
            .warnings
            .into_iter()
            .chain(warnings(items, &got))
            .chain(unchecked)
            .collect(),
        ..report
    })
}

/// The longest wait for the icon manifest.
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The most bytes the icon manifest may have.
const MANIFEST_LIMIT: usize = 8 * 1024 * 1024;

/// Manifest the web client reads, gzipped or plain; none if neither reads.
async fn icon_manifest(client: &Client) -> Option<icon::Manifest> {
    let address = format!("{}{}", client.base_url().storage(), icon::MANIFEST_PATH);
    let limited = client
        .clone()
        .with_response_limit(MANIFEST_TIMEOUT, MANIFEST_LIMIT)
        .ok()?;
    let read = async |suffix: &str| {
        download(&limited, &format!("{address}{suffix}"), MANIFEST_LIMIT)
            .await
            .ok()
            .and_then(|bytes| icon::Manifest::parse(&bytes))
    };
    pin!(
        stream::iter([".gz", ""])
            .then(read)
            .filter_map(future::ready)
    )
    .next()
    .await
}

/// Checks icon names against the manifest, read once per client and only
/// when an item is an icon; an unreadable manifest is a warning.
async fn icons_checked(client: &Client, items: &[Item]) -> Result<Vec<String>> {
    let names: Vec<&str> = items
        .iter()
        .filter_map(|item| match &item.what {
            What::Made { made, .. } => icon::named(made),
            What::Sticker { .. }
            | What::Text { .. }
            | What::Shape { .. }
            | What::Frame { .. }
            | What::Arrow { .. }
            | What::Link { .. }
            | What::Image { .. }
            | What::File { .. }
            | What::Node { .. }
            | What::Stuck { .. }
            | What::Task { .. }
            | What::Drawing { .. } => None,
        })
        .collect();
    let asked = future::OptionFuture::from(names.first().map(|_| {
        client
            .icons()
            .get_or_try_init(|| async { icon_manifest(client).await.ok_or(()) })
    }));
    match asked.await {
        None => Ok(Vec::new()),
        Some(Ok(listed)) => names
            .iter()
            .try_for_each(|name| {
                listed.has(name).then_some(()).ok_or_else(|| {
                    Error::Input(InputError::NotIcon {
                        name: (*name).to_owned(),
                        close: listed.close(name),
                    })
                })
            })
            .map(|()| Vec::new()),
        Some(Err(())) => Ok(vec![
            "icons not checked: the icon list could not be read".to_owned(),
        ]),
    }
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
    /// The status and the kanban cell a task names, if it names them.
    Task {
        status: Option<String>,
        cell: Option<task::Cell>,
    },
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
                | What::Made { .. }
                | What::Stuck { .. }
                | What::Task { .. }
                | What::Drawing { .. } => Ok(Fetched::Nothing),
            }
        })
        .try_collect()
        .await
}

/// The file uploaded to `board` under a fresh key; the server names it.
pub(super) async fn uploaded(
    client: &Client,
    board: &str,
    file: &Local,
) -> Result<Upload> {
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

/// Fetched items: task statuses by id or title, and kanban cells each names.
fn with_tasks(
    items: &[Item],
    got: Vec<Fetched>,
    document: &Doc,
    scene: &Scene<'_>,
) -> Result<Vec<Fetched>> {
    let stored = task::statuses(document);
    items
        .iter()
        .zip(got)
        .try_fold(Vec::new(), |mut done, (item, got)| {
            let next = match &item.what {
                What::Task { status, ask, .. } => Fetched::Task {
                    status: status
                        .as_deref()
                        .map(|given| {
                            task::status_id(given, &stored).ok_or_else(unknown_status)
                        })
                        .transpose()?,
                    cell: ask
                        .as_ref()
                        .map(|ask| cell_of(ask, scene, document, &done))
                        .transpose()?,
                },
                What::Sticker { .. }
                | What::Text { .. }
                | What::Shape { .. }
                | What::Frame { .. }
                | What::Arrow { .. }
                | What::Link { .. }
                | What::Image { .. }
                | What::File { .. }
                | What::Node { .. }
                | What::Made { .. }
                | What::Stuck { .. }
                | What::Drawing { .. } => got,
            };
            done.push(next);
            Ok::<_, Error>(done)
        })
}

/// Kanban cell `ask` names; a new card goes last, after `done` cards too.
fn cell_of(
    ask: &Ask,
    scene: &Scene<'_>,
    document: &Doc,
    done: &[Fetched],
) -> Result<task::Cell> {
    let id = find_id(
        scene.objects().iter().map(|object| object.id.as_str()),
        &ask.kanban,
    )?;
    let holder = scene
        .get(&id)
        .filter(|object| kanban::is_kanban(object))
        .ok_or_else(|| not_value(COMMAND, "kanban", "a kanban"))?;
    let column = titled(
        "column",
        ask.column.as_deref(),
        &kanban::columns(document, holder),
    )?;
    let swimlane = titled(
        "swimlane",
        ask.swimlane.as_deref(),
        &kanban::swimlanes(document, holder),
    )?;
    let sits = |fields: &Value, key: &str, wanted: &str| {
        fields.get(key).and_then(Value::as_str) == Some(wanted)
    };
    let there = scene
        .objects()
        .iter()
        .filter(|card| card.parent.as_deref() == Some(id.as_str()))
        .filter(|card| {
            sits(&card.fields, "columnId", &column)
                && sits(&card.fields, "swimlaneId", &swimlane)
        })
        .filter_map(|card| card.fields.get("index").and_then(Value::as_f64));
    let before = done.iter().filter_map(|got| match got {
        Fetched::Task {
            cell: Some(cell), ..
        } if (&cell.kanban, &cell.column, &cell.swimlane)
            == (&id, &column, &swimlane) =>
        {
            Some(cell.index())
        }
        Fetched::Task { .. }
        | Fetched::Nothing
        | Fetched::Page(_)
        | Fetched::Upload(_) => None,
    });
    Ok(task::Cell {
        after: there.chain(before).max_by(f64::total_cmp),
        kanban: id,
        column,
        swimlane,
    })
}

/// Id of the cell `given` titles, ignoring case; the first if none is given.
fn titled(
    name: &'static str,
    given: Option<&str>,
    cells: &[(String, String)],
) -> Result<String> {
    let found = given.map_or_else(
        || cells.first(),
        |title| {
            cells
                .iter()
                .find(|(_, known)| known.to_lowercase() == title.to_lowercase())
        },
    );
    found.map(|(id, _)| id.clone()).ok_or_else(|| {
        let titles: Vec<&str> = cells.iter().map(|(_, known)| known.as_str()).collect();
        not_value(COMMAND, name, &format!("one of {}", titles.join(", ")))
    })
}

fn unknown_status() -> Error {
    let ids = task::STATUSES.map(|(id, _)| id).join(", ");
    not_value(
        COMMAND,
        "status",
        &format!("one of {ids}, or the title of a status of the board"),
    )
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
    /// A mind-map node: a root, or a child of `parent` as given.
    Node {
        branch: Branch,
        at: Option<Point>,
    },
    /// A task card; `status` is as given, an id or a title.
    Task {
        made: Made,
        status: Option<String>,
        ask: Option<Ask>,
        at: Option<Point>,
    },
    /// An object its kind's own code builds; `at` is asked for, else found.
    Made {
        made: Made,
        at: Option<Point>,
    },
    /// A reaction stamp stuck to the object `on` names.
    Stuck {
        made: Made,
        on: String,
        at: Option<Point>,
    },
    /// A freehand stroke, a [`NewObject::Drawing`] ready to write.
    Drawing(NewObject),
}

/// The kanban a task goes into and the cell, by title, as given.
#[derive(Debug, PartialEq)]
struct Ask {
    kanban: String,
    column: Option<String>,
    swimlane: Option<String>,
}

/// Where a mind-map node hangs: from `parent` on `direction`, or a root.
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
pub(super) struct Local {
    /// The name on disk, without the folders.
    pub(super) name: String,
    mime: &'static str,
    bytes: Vec<u8>,
}

impl Local {
    /// The file as the server holds it under `name`.
    pub(super) fn upload(&self, name: String) -> Upload {
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
    /// The keys only a code block takes, each with whether it is given.
    fn code_keys(&self) -> [(&'static str, bool); 2] {
        [
            ("language", self.language.is_some()),
            ("theme", self.theme.is_some()),
        ]
    }

    /// The keys only a table takes, each with whether it is given.
    fn table_keys(&self) -> [(&'static str, bool); 2] {
        [
            ("rows", self.rows.is_some()),
            ("columns", self.columns.is_some()),
        ]
    }

    /// The keys only an arrow takes, each with whether it is given.
    fn arrow_keys(&self) -> [(&'static str, bool); 6] {
        [
            ("from", self.from.is_some()),
            ("to", self.to.is_some()),
            ("line", self.line.is_some()),
            ("heads", self.heads.is_some()),
            ("stroke_style", self.stroke_style.is_some()),
            ("label", !self.label.is_empty()),
        ]
    }

    /// The keys only a drawing takes, each with whether it is given.
    fn drawing_keys(&self) -> [(&'static str, bool); 2] {
        [
            ("point", self.point.is_some()),
            ("highlighter", self.highlighter),
        ]
    }

    /// The keys only a reaction stamp takes, each with whether it is given.
    fn reaction_keys(&self) -> [(&'static str, bool); 2] {
        [
            ("on", self.on.is_some()),
            ("reaction", !self.reaction.is_empty()),
        ]
    }

    /// The keys only an icon takes, each with whether it is given.
    fn icon_keys(&self) -> [(&'static str, bool); 1] {
        [("weight", self.weight.is_some())]
    }

    /// The keys only a task takes, each with whether it is given.
    fn task_keys(&self) -> [(&'static str, bool); 5] {
        [
            ("description", self.description.is_some()),
            ("status", self.status.is_some()),
            ("start", self.start.is_some()),
            ("due", self.due.is_some()),
            ("kanban", self.kanban.is_some()),
        ]
    }

    /// The keys only a kanban takes, each with whether it is given.
    fn kanban_keys(&self) -> [(&'static str, bool); 2] {
        [
            ("column", !self.column.is_empty()),
            ("swimlane", !self.swimlane.is_empty()),
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
            AddKind::Sticker
                | AddKind::Text
                | AddKind::Shape
                | AddKind::MindMap
                | AddKind::Document
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

    /// Refuses the first key a parent or direction breaks: another kind, a
    /// corner or frame by a parent, a direction without one.
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
        self.refuse_misplaced(kind, item, frame)?;
        let rich = self.rich(kind, word(kind))?;
        let at = fields.at.as_deref().map(str::parse).transpose()?;
        let size = fields.size.as_deref().map(str::parse).transpose()?;
        let color: Option<Color> = fields
            .color
            .as_deref()
            .filter(|_| kind != AddKind::Task)
            .map(str::parse)
            .transpose()?;
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
            AddKind::Dice => self.die(fields.faces, color.as_ref(), at)?,
            AddKind::StickerStack => self.stack(fields, size, color.as_ref(), at)?,
            AddKind::Flipcard => self.card(fields, color.as_ref(), at)?,
            AddKind::Document => self.document(fields, color.as_ref(), at)?,
            AddKind::Wheel => self.wheel(&fields.choice, at)?,
            AddKind::Stamp => self.stamp(fields.text.as_deref(), at)?,
            AddKind::Code => self.code(fields, at)?,
            AddKind::Table => self.table(fields, at)?,
            AddKind::ReactionStamp => self.reaction_stamp(item, frame, at)?,
            AddKind::Icon => self.icon(fields, size, color.as_ref(), at)?,
            AddKind::Task => self.task(fields, at)?,
            AddKind::Drawing => self.drawing(fields, color.as_ref())?,
            AddKind::Kanban => What::Made {
                made: kanban::made(&fields.column, &fields.swimlane, self.ids),
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
        self.refuse_frame_slot(frame, earlier)?;
        Ok(Item {
            what,
            color,
            frame: frame.map(str::to_owned),
            format: if rich { self.format } else { TextFormat::Plain },
        })
    }

    /// Refuses a frame `#N` that is not an earlier frame of the list.
    fn refuse_frame_slot(&self, frame: Option<&str>, earlier: &[Item]) -> Result<()> {
        frame
            .filter(|reference| reference.starts_with('#'))
            .filter(|reference| {
                !slot_of(reference)
                    .and_then(|slot| earlier.get(slot))
                    .is_some_and(|before| matches!(before.what, What::Frame { .. }))
            })
            .map_or(Ok(()), |_| Err(self.bad("frame", "#N of an earlier frame")))
    }

    /// Refuses each key given for a kind that does not take it.
    fn refuse_misplaced(
        &self,
        kind: AddKind,
        item: &AddItem,
        frame: Option<&str>,
    ) -> Result<()> {
        let fields = &item.add;
        let word = word(kind);
        let refuse = |name: &'static str, given: bool| {
            given
                .then(|| self.bad(name, &format!("left out for {word}")))
                .map_or(Ok(()), Err)
        };
        let arrow = kind == AddKind::Arrow;
        let drawing = kind == AddKind::Drawing;
        self.refuse_foreign(kind, AddKind::Arrow, &fields.arrow_keys())?;
        self.refuse_foreign(kind, AddKind::Drawing, &fields.drawing_keys())?;
        refuse(
            "stroke_width",
            !(arrow || drawing) && fields.stroke_width.is_some(),
        )?;
        let linked = kind == AddKind::Link;
        let uploaded = matches!(kind, AddKind::Image | AddKind::File);
        let wheel = kind == AddKind::Wheel;
        refuse("text", textless(kind) && fields.text.is_some())?;
        refuse("faces", kind != AddKind::Dice && fields.faces.is_some())?;
        refuse("choice", !wheel && !fields.choice.is_empty())?;
        refuse("back", kind != AddKind::Flipcard && fields.back.is_some())?;
        self.refuse_node_keys(kind, fields, item.frame.is_some() || frame.is_some())?;
        refuse("at", (arrow || drawing) && fields.at.is_some())?;
        refuse("size", sizeless(kind) && fields.size.is_some())?;
        refuse("shape", kind != AddKind::Shape && fields.shape.is_some())?;
        refuse("color", colourless(kind) && fields.color.is_some())?;
        refuse("url", !linked && fields.url.is_some())?;
        refuse("path", !uploaded && fields.path.is_some())?;
        refuse("no_preview", !linked && fields.no_preview)?;
        self.refuse_foreign(kind, AddKind::Frame, &fields.frame_keys())?;
        self.refuse_foreign(kind, AddKind::Code, &fields.code_keys())?;
        self.refuse_foreign(kind, AddKind::Table, &fields.table_keys())?;
        self.refuse_foreign(kind, AddKind::ReactionStamp, &fields.reaction_keys())?;
        self.refuse_foreign(kind, AddKind::Icon, &fields.icon_keys())?;
        self.refuse_foreign(kind, AddKind::Task, &fields.task_keys())?;
        // A task takes the keys of a kanban's cells, for the cell it goes into.
        let cell_keys = fields
            .kanban_keys()
            .map(|(name, given)| (name, given && kind != AddKind::Task));
        self.refuse_foreign(kind, AddKind::Kanban, &cell_keys)?;
        [
            ("at", fields.at.is_some()),
            ("frame", item.frame.is_some() || frame.is_some()),
        ]
        .into_iter()
        .find(|(_, given)| *given && fields.kanban.is_some())
        .map_or(Ok(()), |(name, _)| {
            Err(self.bad(name, "left out with kanban"))
        })?;
        Ok(())
    }

    /// Stroke through the points, with pen or highlighter defaults; the color
    /// is a palette token written as its RGB number.
    fn drawing(&self, fields: &AddFields, color: Option<&Color>) -> Result<What> {
        let points = fields
            .point
            .iter()
            .flat_map(|given| &given.0)
            .map(|raw| raw.parse())
            .collect::<Result<Vec<Point>>>()?;
        (points.len() >= 2).then_some(()).ok_or_else(|| {
            self.bad(
                "point",
                "at least 2 points, e.g. --point 10,20 --point 120,80",
            )
        })?;
        let (width, rgb, alpha) = [PEN, HIGHLIGHTER]
            .get(usize::from(fields.highlighter))
            .copied()
            .unwrap_or(PEN);
        let width = fields.stroke_width.unwrap_or(width);
        ((1.0..=30.0).contains(&width) && (width - width.round()).abs() < f64::EPSILON)
            .then_some(())
            .ok_or_else(|| self.bad("stroke_width", "a whole number from 1 to 30"))?;
        let color = color
            .and_then(|token| palette::hex(token.as_str()))
            .and_then(|hex| u32::from_str_radix(hex.trim_start_matches('#'), 16).ok())
            .unwrap_or(rgb);
        Ok(What::Drawing(NewObject::Drawing {
            points,
            color,
            width,
            alpha,
        }))
    }

    /// The faces asked for, or the default, which the web client offers.
    fn die(
        &self,
        faces: Option<u32>,
        color: Option<&Color>,
        at: Option<Point>,
    ) -> Result<What> {
        Ok(What::Made {
            made: dice::made(self.faces(faces)?, color),
            at,
        })
    }

    fn stamp(&self, text: Option<&str>, at: Option<Point>) -> Result<What> {
        let text = text.ok_or_else(|| self.missing("text"))?;
        Ok(What::Made {
            made: stamp::made(text),
            at,
        })
    }

    /// A reaction stamp tilted by a draw, stuck to `on` when given.
    fn reaction_stamp(
        &self,
        item: &AddItem,
        frame: Option<&str>,
        at: Option<Point>,
    ) -> Result<What> {
        let fields = &item.add;
        let key = self.reaction_key(&fields.reaction)?;
        let draw = f64::from(author_number(&(self.ids)(), 0)) / f64::from(DRAWS);
        let made = reaction_stamp::made(key, draw);
        match fields.on.clone() {
            Some(_) if item.frame.is_some() || frame.is_some() => {
                Err(self.bad("frame", "left out with on"))
            }
            Some(on) => Ok(What::Stuck { made, on, at }),
            None => Ok(What::Made { made, at }),
        }
    }

    /// The stamp asked for or the default, from those the web client offers.
    fn reaction_key<'k>(&self, given: &'k [String]) -> Result<&'k str> {
        match given {
            [] => Ok(reaction_stamp::DEFAULT_KEY),
            [key] if reaction_stamp::KEYS.contains(&key.as_str()) => Ok(key),
            [_] => Err(self.bad("reaction", &one_of(&reaction_stamp::KEYS))),
            _ => Err(self.bad("reaction", "one key")),
        }
    }

    /// Icon named by the text, in a square size and a web client picker color.
    fn icon(
        &self,
        fields: &AddFields,
        size: Option<Size>,
        color: Option<&Color>,
        at: Option<Point>,
    ) -> Result<What> {
        let name = fields.text.as_deref().ok_or_else(|| self.missing("text"))?;
        icon::is_name(name).then_some(()).ok_or_else(|| {
            self.bad(
                "text",
                "lower-case letters, digits and dashes, e.g. rocket-launch",
            )
        })?;
        color
            .filter(|color| !icon::COLORS.contains(&color.as_str()))
            .map_or(Ok(()), |_| Err(self.bad("color", &one_of(&icon::COLORS))))?;
        let edge = size.map_or(Ok(icon::DEFAULT_SIDE), |given| {
            Size::new(given.width(), given.width())
                .ok()
                .filter(|square| *square == given)
                .map(|_| given.width())
                .ok_or_else(|| self.bad("size", "square, e.g. 48,48"))
        })?;
        Ok(What::Made {
            made: icon::made(name, fields.weight.unwrap_or_default(), color, edge),
            at,
        })
    }

    fn wheel(&self, choices: &[String], at: Option<Point>) -> Result<What> {
        Ok(What::Made {
            made: wheel::made(self.choices(choices)?, self.ids),
            at,
        })
    }

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

    /// A document of the text, on one of the colors the card offers.
    fn document(
        &self,
        fields: &AddFields,
        color: Option<&Color>,
        at: Option<Point>,
    ) -> Result<What> {
        let text = fields.text.as_deref().ok_or_else(|| self.missing("text"))?;
        color
            .filter(|color| !document::COLORS.contains(&color.as_str()))
            .map_or(Ok(()), |_| {
                Err(self
                    .bad("color", &format!("one of {}", document::COLORS.join(", "))))
            })?;
        Ok(What::Made {
            made: document::made(text, color),
            at,
        })
    }

    /// A code block of the text, in the language and theme the call gives.
    fn code(&self, fields: &AddFields, at: Option<Point>) -> Result<What> {
        let text = fields.text.as_deref().ok_or_else(|| self.missing("text"))?;
        fields.color.as_ref().map_or(Ok(()), |_| {
            Err(self.bad("color", "left out for a code block"))
        })?;
        Ok(What::Made {
            made: code::made(text, fields.language, fields.theme.unwrap_or_default()),
            at,
        })
    }

    /// Task titled by the text, with the given days, color and status;
    /// a start after the due day is refused.
    fn task(&self, fields: &AddFields, at: Option<Point>) -> Result<What> {
        let title = fields.text.as_deref().ok_or_else(|| self.missing("text"))?;
        let day = |name: &'static str, given: Option<&str>| {
            given
                .map(|raw| {
                    raw.parse::<Day>()
                        .map_err(|expected| self.bad(name, &expected.to_string()))
                })
                .transpose()
        };
        let start = day("start", fields.start.as_deref())?;
        let due = day("due", fields.due.as_deref())?;
        start
            .iter()
            .zip(&due)
            .find(|(start, due)| start > due)
            .map_or(Ok(()), |_| {
                Err(self.bad("start", "on or before the due day"))
            })?;
        let ask = self.ask(fields)?;
        let color = fields
            .color
            .as_deref()
            .map(|name| {
                task::tint(name).ok_or_else(|| {
                    let names = task::COLORS.map(|(known, _)| known);
                    self.bad("color", &format!("one of {}", names.join(", ")))
                })
            })
            .transpose()?;
        Ok(What::Task {
            made: task::made(
                title,
                fields.description.as_deref(),
                (start.as_ref(), due.as_ref()),
                color,
                (self.ids)(),
            ),
            status: fields.status.clone(),
            ask,
            at,
        })
    }

    /// The kanban cell a task names: one title each, and only with a kanban.
    fn ask(&self, fields: &AddFields) -> Result<Option<Ask>> {
        let one = |name: &'static str, given: &[String]| match given {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(self.bad(name, "one title for a task")),
        };
        let (column, swimlane) = (
            one("column", &fields.column)?,
            one("swimlane", &fields.swimlane)?,
        );
        [
            ("column", column.is_some()),
            ("swimlane", swimlane.is_some()),
        ]
        .into_iter()
        .find(|(_, given)| *given && fields.kanban.is_none())
        .map_or(Ok(()), |(name, _)| {
            Err(self.bad(name, "left out without kanban"))
        })?;
        Ok(fields.kanban.clone().map(|kanban| Ask {
            kanban,
            column,
            swimlane,
        }))
    }

    /// A table of the rows and columns the call gives, within the cell limit.
    fn table(&self, fields: &AddFields, at: Option<Point>) -> Result<What> {
        let count = |name: &'static str, given: Option<u32>, default: u32| match given {
            Some(0) => Err(self.bad(name, "at least 1")),
            _ => Ok(given.unwrap_or(default)),
        };
        let rows = count("rows", fields.rows, table::DEFAULT_ROWS)?;
        let columns = count("columns", fields.columns, table::DEFAULT_COLUMNS)?;
        match rows.saturating_mul(columns) {
            cells if cells > table::CELL_LIMIT => Err(self.bad(
                "rows",
                &format!("at most {} cells with the columns", table::CELL_LIMIT),
            )),
            _ => Ok(What::Made {
                made: table::made(rows, columns),
                at,
            }),
        }
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
        let path = path.ok_or_else(|| self.missing("path"))?;
        let (file, pixels) =
            local_picture(path, |expected| self.bad("path", expected))?;
        Ok(What::Image {
            size: size.map_or_else(|| fitted(pixels), Ok)?,
            pixels,
            at,
            file,
        })
    }

    fn file(&self, path: Option<&str>, at: Option<Point>) -> Result<What> {
        let path = path.ok_or_else(|| self.missing("path"))?;
        let (name, bytes) = read_local(path, || self.bad("path", "a file"))?;
        Ok(What::File {
            at,
            file: Local {
                mime: file_type(&name),
                name,
                bytes,
            },
        })
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

/// Name and bytes of the local file `path`; `refused` says what a path with
/// no file name should have been.
fn read_local(
    path: &str,
    refused: impl FnOnce() -> Error,
) -> Result<(String, Vec<u8>)> {
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(refused)?;
    Ok((name, file::read(path)?))
}

/// The local picture at `path` and its pixels; `refused` is what a path should
/// have been when it is no file or no PNG, JPEG, GIF or WebP picture.
pub(super) fn local_picture(
    path: &str,
    refused: impl Fn(&str) -> Error,
) -> Result<(Local, Size)> {
    let (name, bytes) = read_local(path, || refused("a file"))?;
    let (format, (width, height)) = match UploadKind::of(&bytes, true) {
        Some(UploadKind::Image(format)) => {
            pixel_size(&bytes).map(|pixels| (format, pixels))
        }
        Some(UploadKind::File) | None => None,
    }
    .ok_or_else(|| refused("a png, jpg, gif or webp picture"))?;
    let pixels = Size::new(f64::from(width), f64::from(height))?;
    Ok((
        Local {
            name,
            mime: format.mime(),
            bytes,
        },
        pixels,
    ))
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

/// The web's pen and highlighter: width, packed RGB and alpha.
const PEN: (f64, u32, f64) = (3.0, 0x001e_1e1e, 1.0);
const HIGHLIGHTER: (f64, u32, f64) = (22.0, 0x00fb_f040, 0.5);

/// What `author_number` is under: its draws are in `0..DRAWS`.
const DRAWS: u32 = 0x8000_0000;

/// `one of a, b or c`.
fn one_of(words: &[&str]) -> String {
    words.split_last().map_or_else(String::new, |(last, rest)| {
        format!("one of {} or {last}", rest.join(", "))
    })
}

/// Whether the kind has its own size calls cannot change, and takes no text.
const fn textless(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Arrow
            | AddKind::Link
            | AddKind::Image
            | AddKind::File
            | AddKind::Dice
            | AddKind::Wheel
            | AddKind::Table
            | AddKind::ReactionStamp
            | AddKind::Kanban
            | AddKind::Drawing
    )
}

/// Whether the kind takes no color.
const fn colourless(kind: AddKind) -> bool {
    matches!(
        kind,
        AddKind::Link
            | AddKind::Image
            | AddKind::File
            | AddKind::Wheel
            | AddKind::Stamp
            | AddKind::Table
            | AddKind::ReactionStamp
            | AddKind::Kanban
    )
}

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
            | AddKind::Document
            | AddKind::Stamp
            | AddKind::Code
            | AddKind::Table
            | AddKind::ReactionStamp
            | AddKind::Task
            | AddKind::Kanban
            | AddKind::Drawing
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
        AddKind::Document => "a document",
        AddKind::Stamp => "a stamp",
        AddKind::Code => "a code block",
        AddKind::Table => "a table",
        AddKind::ReactionStamp => "a reaction stamp",
        AddKind::Icon => "an icon",
        AddKind::Task => "a task",
        AddKind::Kanban => "a kanban",
        AddKind::Drawing => "a drawing",
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

/// New mind-map node with its text, hung from `parent`: a board node, `#N`,
/// or one of the `earlier` nodes of this write; a root's corner is left out.
fn hung_node(
    scene: &Scene<'_>,
    branch: &Branch,
    color: Option<&Color>,
    earlier: &[Object],
) -> Result<Made> {
    match branch.parent.as_deref() {
        Some(slot) if slot.starts_with('#') => {
            let objects: Vec<Object> =
                scene.objects().iter().chain(earlier).cloned().collect();
            let joined = Scene::new(&objects);
            let under = Under {
                scene: &joined,
                parent: node_of(&joined, slot)?,
                direction: branch.direction,
            };
            mind_map::made(&branch.text, color, Some(&under))
        }
        Some(_) | None => {
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
    }
}

/// The mind-map node `reference` names on the board.
fn parent_node<'a>(scene: &Scene<'a>, reference: &str) -> Result<&'a Object> {
    let id = find_id(
        scene.objects().iter().map(|object| object.id.as_str()),
        reference,
    )?;
    node_of(scene, &id)
}

fn node_of<'a>(scene: &Scene<'a>, id: &str) -> Result<&'a Object> {
    scene
        .get(id)
        .filter(|object| mind_map::is_node(object))
        .ok_or_else(|| not_value(COMMAND, "parent", "a mind-map node"))
}

/// `made` stuck to the object `on` names.
fn stuck_stamp(
    scene: &Scene<'_>,
    made: &Made,
    on: &str,
    at: Option<Point>,
) -> Result<NewObject> {
    let target = stuck_target(scene, on)?;
    reaction_stamp::stuck(made.clone(), &target, at).map(NewObject::Made)
}

/// Object `reference` names; the web client sticks a stamp to it, with its box.
fn stuck_target<'a>(scene: &Scene<'a>, reference: &str) -> Result<Target<'a>> {
    let id = find_id(
        scene.objects().iter().map(|object| object.id.as_str()),
        reference,
    )?;
    let refused =
        || not_value(COMMAND, "on", "a shape, sticker, document, task or image");
    let object = scene
        .get(&id)
        .filter(|object| {
            matches!(
                object.kind,
                ObjectKind::Shape
                    | ObjectKind::Sticker
                    | ObjectKind::Document
                    | ObjectKind::Task
                    | ObjectKind::Image
            )
        })
        .ok_or_else(refused)?;
    Ok(Target {
        id: &object.id,
        area: scene.rectangle_of(object).ok_or_else(refused)?,
        rotation: object.rotation,
    })
}

/// The new objects in order: a corner is found for those without one, among
/// the objects there and the ones before in the list. A frame `#N` is the
/// N-th item, a frame made in the same list, and is kept as `#N`.
#[expect(clippy::too_many_lines, reason = "one arm per kind of item")]
fn entries(
    items: &[Item],
    got: &[Fetched],
    scene: &Scene<'_>,
    linked: Option<&Object>,
) -> Result<Vec<NewEntry>> {
    let mut placed: Vec<Rectangle> = Vec::new();
    let mut frames: Vec<Option<Rectangle>> = Vec::new();
    // The mind-map nodes made so far, which `#N` parents hang from.
    let mut nodes: Vec<Object> = Vec::new();
    items
        .iter()
        .enumerate()
        .map(|(place, item)| {
            let (within, named) = frames_of(item, scene, &frames)?;
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
                    color: item.color.clone(),
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
                    color: item.color.clone(),
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
                    color: item.color.clone(),
                    look: *look,
                },
                What::Arrow { from, to, look } => NewObject::Arrow {
                    from: from.clone(),
                    to: to.clone(),
                    color: item.color.clone(),
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
                What::Stuck { made, on, at } => stuck_stamp(scene, made, on, *at)?,
                What::Drawing(stroke) => stroke.clone(),
                What::Made { made, at } => {
                    put(made.clone(), *at, &mut spot).map(NewObject::Made)?
                }
                What::Task { made, at, .. } => {
                    put(settled(made, got.get(place)), *at, &mut spot)
                        .map(NewObject::Made)?
                }
                What::Node { branch, at } => grown(
                    scene,
                    (branch, *at, item.color.as_ref()),
                    (place, &mut nodes),
                    &mut spot,
                )?,
            };
            frames.push(made_frame(item, &placed));
            Ok(entry_in(item, new, (named, linked)))
        })
        .collect()
}

/// A mind-map node made, and kept in `nodes` for the `#N` children to come.
fn grown(
    scene: &Scene<'_>,
    (branch, at, color): (&Branch, Option<Point>, Option<&Color>),
    (place, nodes): (usize, &mut Vec<Object>),
    spot: &mut impl FnMut(Option<Point>, Option<Size>, &ObjectKind) -> Result<Point>,
) -> Result<NewObject> {
    let node = hung_node(scene, branch, color, nodes)?;
    let fixed = branch.parent.as_ref().map(|_| node.at).or(at);
    let made = put(node, fixed, spot)?;
    nodes.extend(mind_map::stood(&made, place));
    Ok(NewObject::Made(made))
}

/// `made` at `at` or a free `spot`; a kanban-held one stays at its origin.
fn put(
    made: Made,
    at: Option<Point>,
    spot: &mut impl FnMut(Option<Point>, Option<Size>, &ObjectKind) -> Result<Point>,
) -> Result<Made> {
    Ok(match made.parent {
        Some(_) => made,
        None => Made {
            at: spot(at, made.size, &made.kind)?,
            ..made
        },
    })
}

/// The task with the status its call named, settled, if it named one.
fn settled(made: &Made, got: Option<&Fetched>) -> Made {
    match got {
        Some(Fetched::Task { status, cell }) => {
            let card = status
                .as_deref()
                .map_or_else(|| made.clone(), |id| task::with_status(made.clone(), id));
            cell.as_ref()
                .map_or(card.clone(), |cell| task::in_cell(card, cell))
        }
        Some(Fetched::Nothing | Fetched::Page(_) | Fetched::Upload(_)) | None => {
            made.clone()
        }
    }
}

/// New object as an entry in its frame, or the linked frame for arrows.
fn entry_in(
    item: &Item,
    new: NewObject,
    (named, linked): (Option<&Object>, Option<&Object>),
) -> NewEntry {
    let fresh = item.frame.as_deref().filter(|name| name.starts_with('#'));
    // An arrow or a stuck stamp goes in the frame it falls in, if unasked.
    let frame = named.or(linked
        .filter(|_| !matches!(item.what, What::Arrow { .. } | What::Stuck { .. })));
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
        Some(Fetched::Nothing | Fetched::Upload(_) | Fetched::Task { .. }) | None => {
            None
        }
    }
}

/// The file as the server holds it; as it will be named when none was sent.
fn held(file: &Local, got: Option<&Fetched>) -> Upload {
    match got {
        Some(Fetched::Upload(done)) => done.clone(),
        Some(Fetched::Nothing | Fetched::Page(_) | Fetched::Task { .. }) | None => {
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
    use yrs::{
        Any,
        ArrayPrelim,
        Doc,
        Map as _,
        MapPrelim,
        ReadTxn as _,
        StateVector,
        Transact as _,
    };

    use super::{
        Fetched,
        Item,
        add,
        added,
        entries,
        fetched,
        icons_checked,
        linked_frame,
        read_items,
        with_tasks,
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
            change::{
                receipt::ChangeReport,
                value::{Made, NewObject, Upload},
            },
            kinds::icon::MANIFEST_PATH,
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
                    NewObject::Arrow { .. } | NewObject::Drawing { .. } => None,
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
    fn a_document_needs_its_text_and_takes_only_the_colors_of_the_card() -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "document", "text": "Spec", "color": "red4"}))?,
                refused(json!({"kind": "document", "color": "white1"}))?,
                refused(json!({"kind": "document", "text": "a", "size": "9,9"}))?,
                refused(json!({"kind": "document", "text": "a", "color": "blue3"}))?,
            ],
            [
                "color of `object add` must be one of white1, gray3, red3, orange3, yellow3, green3, blue3, violet3, in item 1",
                "text of `object add` must be given, in item 1",
                "size of `object add` must be left out for a document, in item 1",
                "",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_code_block_needs_its_text_and_takes_only_its_language_and_theme() -> Result<()>
    {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "code"}))?,
                refused(json!({"kind": "code", "text": "a", "color": "red4"}))?,
                refused(json!({"kind": "code", "text": "a", "size": "9,9"}))?,
                refused(json!({"kind": "sticker", "text": "a", "theme": "dark"}))?,
                refused(
                    json!({"kind": "code", "text": "a", "language": "rust", "theme": "dark"})
                )?,
            ],
            [
                "text of `object add` must be given, in item 1",
                "color of `object add` must be left out for a code block, in item 1",
                "size of `object add` must be left out for a code block, in item 1",
                "theme of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_language_the_web_does_not_offer_is_refused_with_the_list() {
        let refused = args(json!({
            "items": [{"kind": "code", "text": "a", "language": "cobol"}]
        }))
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
        assert!(
            refused.contains("typescript") && refused.contains("plain_text"),
            "{refused}"
        );
    }

    #[test]
    fn a_table_refuses_no_rows_too_many_cells_and_keys_of_other_kinds() -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "table", "rows": 0}))?,
                refused(json!({"kind": "table", "columns": 0}))?,
                refused(json!({"kind": "table", "rows": 60, "columns": 50}))?,
                refused(json!({"kind": "table", "size": "10,10"}))?,
                refused(json!({"kind": "table", "color": "red4"}))?,
                refused(json!({"kind": "table", "text": "a"}))?,
                refused(json!({"text": "a", "rows": 2}))?,
                refused(json!({"kind": "table", "rows": 50, "columns": 50}))?,
            ],
            [
                "rows of `object add` must be at least 1, in item 1",
                "columns of `object add` must be at least 1, in item 1",
                "rows of `object add` must be at most 2500 cells with the columns, in item 1",
                "size of `object add` must be left out for a table, in item 1",
                "color of `object add` must be left out for a table, in item 1",
                "text of `object add` must be left out for a table, in item 1",
                "rows of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_table_is_made_with_its_counts_and_corner() -> Result<()> {
        let args = args(json!({
            "kind": "table", "rows": 3, "columns": 4, "at": "10,20"
        }))?;
        let scene = Scene::new(&[]);
        let found = entries(&read(&args)?, &[], &scene, None)?;
        let table = found.into_iter().find_map(|entry| made_of(entry.new));
        assert_eq!(
            table.map(|made| (
                made.kind,
                (made.at.x(), made.at.y()),
                made.size.map(|size| (size.width(), size.height())),
                made.fields
                    .iter()
                    .filter(|(name, _)| name.starts_with("row-"))
                    .count(),
            )),
            Some((ObjectKind::Table, (10.0, 20.0), Some((960.0, 180.0)), 3))
        );
        Ok(())
    }

    #[test]
    fn a_kanban_refuses_a_size_a_text_a_color_and_keys_of_other_kinds() -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "kanban", "size": "10,10"}))?,
                refused(json!({"kind": "kanban", "text": "a"}))?,
                refused(json!({"kind": "kanban", "color": "red"}))?,
                refused(json!({"kind": "kanban", "status": "done"}))?,
                refused(json!({"text": "a", "column": ["Now"]}))?,
                refused(json!({"text": "a", "swimlane": ["Team"]}))?,
                refused(
                    json!({"kind": "kanban", "column": ["Now"], "swimlane": ["Team"]})
                )?,
            ],
            [
                "size of `object add` must be left out for a kanban, in item 1",
                "text of `object add` must be left out for a kanban, in item 1",
                "color of `object add` must be left out for a kanban, in item 1",
                "status of `object add` must be left out for a kanban, in item 1",
                "column of `object add` must be left out for a sticker, in item 1",
                "swimlane of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_drawing_refuses_a_text_a_size_a_corner_and_keys_of_other_kinds() -> Result<()>
    {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let points = json!(["1,1", "2,2"]);
        assert_eq!(
            [
                refused(json!({"kind": "drawing", "point": points, "text": "a"}))?,
                refused(json!({"kind": "drawing", "point": points, "size": "9,9"}))?,
                refused(json!({"kind": "drawing", "point": points, "at": "1,1"}))?,
                refused(json!({"kind": "drawing", "point": points, "faces": 6}))?,
                refused(json!({"text": "a", "point": points}))?,
                refused(json!({"text": "a", "highlighter": true}))?,
                refused(json!({"text": "a", "stroke_width": 4}))?,
                refused(json!({"kind": "drawing", "point": ["1,1"]}))?,
                refused(
                    json!({"kind": "drawing", "point": points, "stroke_width": 2.5})
                )?,
                refused(
                    json!({"kind": "drawing", "point": points, "stroke_width": 31})
                )?,
            ],
            [
                "text of `object add` must be left out for a drawing, in item 1",
                "size of `object add` must be left out for a drawing, in item 1",
                "at of `object add` must be left out for a drawing, in item 1",
                "faces of `object add` must be left out for a drawing, in item 1",
                "point of `object add` must be left out for a sticker, in item 1",
                "highlighter of `object add` must be left out for a sticker, in item 1",
                "stroke_width of `object add` must be left out for a sticker, in item 1",
                "point of `object add` must be at least 2 points, e.g. --point 10,20 --point 120,80, in item 1",
                "stroke_width of `object add` must be a whole number from 1 to 30, in item 1",
                "stroke_width of `object add` must be a whole number from 1 to 30, in item 1",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_stroke_is_a_pen_or_a_highlighter_with_a_palette_color() -> Result<()> {
        let drawn = |item: Value| -> Result<Vec<super::What>> {
            Ok(read(&args(json!({ "items": [item] }))?)?
                .into_iter()
                .map(|item| item.what)
                .collect())
        };
        let made = |color: u32, width: f64, alpha: f64| -> Result<Vec<super::What>> {
            Ok(vec![super::What::Drawing(NewObject::Drawing {
                points: vec![
                    crate::domain::geometry::Point::new(1.0, 1.0)?,
                    crate::domain::geometry::Point::new(2.0, 2.0)?,
                ],
                color,
                width,
                alpha,
            })])
        };
        let two = json!(["1,1", "2,2"]);
        assert_eq!(
            [
                drawn(json!({"kind": "drawing", "point": two}))?,
                drawn(json!({"kind": "drawing", "point": two, "highlighter": true}))?,
                drawn(
                    json!({"kind": "drawing", "point": two, "color": "yellow4", "stroke_width": 9})
                )?,
            ],
            [
                made(0x001e_1e1e, 3.0, 1.0)?,
                made(0x00fb_f040, 22.0, 0.5)?,
                made(0x00fc_f3af, 9.0, 1.0)?,
            ]
        );
        Ok(())
    }

    #[test]
    fn a_kanban_is_made_at_its_corner_with_its_cells_and_first_card() -> Result<()> {
        let args = args(json!({
            "kind": "kanban", "column": ["Now", "Next"], "at": "10,20"
        }))?;
        let found = entries(&read(&args)?, &[], &Scene::new(&[]), None)?;
        let kanban = found.into_iter().find_map(|entry| made_of(entry.new));
        assert_eq!(
            kanban.map(|made| (
                made.kind,
                (made.at.x(), made.at.y()),
                made.parts.len(),
                made.entries.len(),
            )),
            Some((ObjectKind::Other("kanban".to_owned()), (10.0, 20.0), 1, 4))
        );
        Ok(())
    }

    #[test]
    fn a_task_refuses_no_title_bad_days_a_late_start_and_keys_of_other_kinds()
    -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"kind": "task"}))?,
                refused(json!({"kind": "task", "text": "a", "start": "soon"}))?,
                refused(json!({"kind": "task", "text": "a", "due": "2026-02-30"}))?,
                refused(json!({
                    "kind": "task", "text": "a", "start": "2026-11-07", "due": "2026-11-06"
                }))?,
                refused(json!({"kind": "task", "text": "a", "color": "red4"}))?,
                refused(json!({"kind": "task", "text": "a", "size": "10,10"}))?,
                refused(json!({"text": "a", "status": "done"}))?,
                refused(json!({"text": "a", "due": "2026-11-06"}))?,
                refused(json!({
                    "kind": "task", "text": "a", "start": "2026-11-06", "due": "2026-11-06"
                }))?,
            ],
            [
                "text of `object add` must be given, in item 1",
                "start of `object add` must be a day as YYYY-MM-DD, e.g. 2026-11-06, in item 1",
                "due of `object add` must be a day as YYYY-MM-DD, e.g. 2026-11-06, in item 1",
                "start of `object add` must be on or before the due day, in item 1",
                "color of `object add` must be one of gray, green, yellow, orange, red, violet, blue, sky, lime, pink, in item 1",
                "size of `object add` must be left out for a task, in item 1",
                "status of `object add` must be left out for a sticker, in item 1",
                "due of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        Ok(())
    }

    /// A board whose status store holds `stored`, as id and title.
    fn board_with_statuses(stored: &[(&str, &str)]) -> Doc {
        let document = Doc::new();
        let store = document.get_or_insert_map("task-card-statusesStore");
        let mut transaction = document.transact_mut();
        stored
            .iter()
            .map(|(id, title)| {
                store.insert(
                    &mut transaction,
                    *id,
                    MapPrelim::from_iter([("title", Any::from(*title))]),
                )
            })
            .for_each(drop);
        drop(transaction);
        document
    }

    /// Status `given` makes on a board storing `stored`; or the error text.
    fn status_of(given: &str, stored: &[(&str, &str)]) -> Result<Value> {
        let args = args(json!({
            "kind": "task", "text": "Ship", "status": given, "at": "10,20",
            "description": "Go live", "due": "2026-11-06", "color": "yellow"
        }))?;
        let items = read(&args)?;
        let board = board_with_statuses(stored);
        Ok(
            match with_tasks(&items, vec![Fetched::Nothing], &board, &Scene::new(&[])) {
                Ok(got) => entries(&items, &got, &Scene::new(&[]), None)?
                    .into_iter()
                    .find_map(|entry| made_of(entry.new))
                    .and_then(|made| {
                        made.fields
                            .into_iter()
                            .find(|(name, _)| *name == "statusId")
                            .map(|(_, id)| id)
                    })
                    .unwrap_or_default(),
                Err(error) => json!(error.to_string()),
            },
        )
    }

    #[test]
    fn a_task_takes_its_status_by_id_or_by_a_title_of_the_board() -> Result<()> {
        let review = [("s9", "In review")];
        assert_eq!(
            [
                status_of("in-progress", &[])?,
                status_of("Done", &[])?,
                status_of("in review", &review)?,
                status_of("s9", &review)?,
            ],
            [
                json!("in-progress"),
                json!("done"),
                json!("s9"),
                json!("s9")
            ]
        );
        Ok(())
    }

    #[test]
    fn a_task_names_the_statuses_it_can_take_when_the_status_is_unknown() -> Result<()>
    {
        assert_eq!(
            status_of("later", &[("s9", "In review")])?,
            json!(
                "status of `object add` must be one of backlog, to-do, in-progress, done, canceled, or the title of a status of the board"
            )
        );
        Ok(())
    }

    #[test]
    fn a_task_without_a_status_is_made_with_its_corner_size_and_info() -> Result<()> {
        let args = args(json!({ "kind": "task", "text": "Ship", "at": "10,20" }))?;
        let found = entries(&read(&args)?, &[], &Scene::new(&[]), None)?;
        let task = found.into_iter().find_map(|entry| made_of(entry.new));
        assert_eq!(
            task.map(|made| (
                made.kind,
                (made.at.x(), made.at.y()),
                made.size.map(|size| (size.width(), size.height())),
                made.fields.iter().any(|(name, _)| *name == "statusId"),
                made.fields
                    .iter()
                    .find(|(name, _)| *name == "taskCardInfoId")
                    .map(|(_, id)| id.clone()),
                made.entries.len(),
            )),
            Some((
                ObjectKind::Task,
                (10.0, 20.0),
                Some((300.0, 120.0)),
                false,
                Some(json!("w1")),
                6
            ))
        );
        Ok(())
    }

    /// Board of kanban `k1` with columns To do (`c1`) and Done (`c2`), one
    /// swimlane Untitled (`l1`) and card `t0` in Done.
    fn kanban_board() -> (Doc, Vec<Object>) {
        let document = Doc::new();
        let infos = document.get_or_insert_map("kanban-infoStore");
        let columns = document.get_or_insert_map("kanban-columnsStore");
        let lanes = document.get_or_insert_map("kanban-swimlanesStore");
        {
            let mut transaction = document.transact_mut();
            let info = infos.insert(
                &mut transaction,
                "i1",
                MapPrelim::from_iter([("columns", Any::from(0))]),
            );
            info.insert(
                &mut transaction,
                "columns",
                ArrayPrelim::from_iter([Any::from("c1"), Any::from("c2")]),
            );
            info.insert(
                &mut transaction,
                "swimlanes",
                ArrayPrelim::from_iter([Any::from("l1")]),
            );
            [
                (&columns, "c1", "To do", 0),
                (&columns, "c2", "Done", 10_000),
            ]
            .into_iter()
            .chain([(&lanes, "l1", "Untitled", 0)])
            .for_each(|(store, id, title, index)| {
                store.insert(
                    &mut transaction,
                    id,
                    MapPrelim::from_iter([
                        ("title", Any::from(title)),
                        ("index", Any::from(index)),
                    ]),
                );
            });
        }
        let objects = [
            json!({"id": "k1", "type": "kanban", "position": {"x": 0, "y": 0},
                   "kanbanInfoId": "i1"}),
            json!({"id": "t0", "type": "task-card", "position": {"x": 0, "y": 0},
                   "parentId": "k1", "columnId": "c2", "swimlaneId": "l1",
                   "index": 1_099_511_627_776.0}),
            json!({"id": "s1", "type": "sticker", "position": {"x": 0, "y": 0}}),
        ];
        (
            document,
            objects.into_iter().filter_map(object_of).collect(),
        )
    }

    /// Per task of `items`: holder, column, swimlane, index; or the error text.
    fn carded(items: &Value) -> Result<Value> {
        let (board, objects) = kanban_board();
        let scene = Scene::new(&objects);
        let items = read(&args(json!({ "items": items }))?)?;
        let got = items.iter().map(|_| Fetched::Nothing).collect();
        Ok(match with_tasks(&items, got, &board, &scene) {
            Ok(got) => json!(
                entries(&items, &got, &scene, None)?
                    .into_iter()
                    .filter_map(|entry| made_of(entry.new))
                    .map(|made| {
                        let field = |name: &str| {
                            made.fields
                                .iter()
                                .find(|(field, _)| *field == name)
                                .map(|(_, value)| value.clone())
                        };
                        json!([
                            made.parent,
                            field("columnId"),
                            field("swimlaneId"),
                            field("index"),
                            (made.at.x(), made.at.y())
                        ])
                    })
                    .collect::<Vec<_>>()
            ),
            Err(error) => json!(error.to_string()),
        })
    }

    #[test]
    fn a_task_goes_into_the_cell_its_kanban_column_and_swimlane_name() -> Result<()> {
        let card = |extra: Value| {
            let mut item: Map<String, Value> =
                [("kind", "task"), ("text", "Ship"), ("kanban", "k1")]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), json!(value)))
                    .collect();
            item.extend(extra.as_object().cloned().unwrap_or_default());
            Value::Object(item)
        };
        assert_eq!(
            [
                carded(&json!([card(json!({}))]))?,
                carded(&json!([card(
                    json!({"column": ["DONE"], "swimlane": ["untitled"]})
                )]))?,
                carded(&json!([
                    card(json!({"column": ["Done"]})),
                    card(json!({"column": ["Done"]})),
                    card(json!({}))
                ]))?,
            ],
            [
                json!([["k1", "c1", "l1", 1_099_511_627_776.0, [0.0, 0.0]]]),
                json!([["k1", "c2", "l1", 2_199_023_255_552.0, [0.0, 0.0]]]),
                json!([
                    ["k1", "c2", "l1", 2_199_023_255_552.0, [0.0, 0.0]],
                    ["k1", "c2", "l1", 3_298_534_883_328.0, [0.0, 0.0]],
                    ["k1", "c1", "l1", 1_099_511_627_776.0, [0.0, 0.0]]
                ]),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_task_in_a_kanban_refuses_an_unknown_cell_and_a_board_object_that_is_no_kanban()
    -> Result<()> {
        let task = |extra: (&str, Value)| json!([{"kind": "task", "text": "a", "kanban": "k1", extra.0: extra.1}]);
        assert_eq!(
            [
                carded(&task(("column", json!(["Later"]))))?,
                carded(&task(("swimlane", json!(["Team"]))))?,
                carded(&json!([{"kind": "task", "text": "a", "kanban": "s1"}]))?,
            ],
            [
                json!("column of `object add` must be one of To do, Done"),
                json!("swimlane of `object add` must be one of Untitled"),
                json!("kanban of `object add` must be a kanban"),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_task_refuses_a_kanban_beside_a_corner_a_frame_or_a_second_title() -> Result<()>
    {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        assert_eq!(
            [
                refused(json!({"text": "a", "kanban": "k1"}))?,
                refused(json!({"kind": "kanban", "kanban": "k1"}))?,
                refused(
                    json!({"kind": "task", "text": "a", "kanban": "k1", "at": "1,2"})
                )?,
                refused(
                    json!({"kind": "task", "text": "a", "kanban": "k1", "frame": "Q3"})
                )?,
                refused(json!({"kind": "task", "text": "a", "column": ["Done"]}))?,
                refused(json!({"kind": "task", "text": "a", "swimlane": ["Team"]}))?,
                refused(json!({
                    "kind": "task", "text": "a", "kanban": "k1", "column": ["a", "b"]
                }))?,
                refused(json!({
                    "kind": "task", "text": "a", "kanban": "k1", "swimlane": ["a", "b"]
                }))?,
                refused(json!({
                    "kind": "task", "text": "a", "kanban": "k1",
                    "column": ["Done"], "swimlane": ["Team"]
                }))?,
            ],
            [
                "kanban of `object add` must be left out for a sticker, in item 1",
                "kanban of `object add` must be left out for a kanban, in item 1",
                "at of `object add` must be left out with kanban, in item 1",
                "frame of `object add` must be left out with kanban, in item 1",
                "column of `object add` must be left out without kanban, in item 1",
                "swimlane of `object add` must be left out without kanban, in item 1",
                "column of `object add` must be one title for a task, in item 1",
                "swimlane of `object add` must be one title for a task, in item 1",
                "",
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
    fn a_mind_map_parent_hash_n_is_an_earlier_node_of_the_same_write() -> Result<()> {
        let board = mind_map_board(&[]);
        let map =
            |items: Value| -> Result<Vec<crate::domain::change::value::NewEntry>> {
                let args = args(json!({ "items": items }))?;
                entries(&read(&args)?, &[], &Scene::new(&board), None)
            };
        let found: Vec<_> = map(json!([
            {"kind": "mind-map", "text": "Plan", "at": "2000,2000"},
            {"kind": "mind-map", "text": "Build", "parent": "#1"},
            {"kind": "mind-map", "text": "Test", "parent": "#1"},
            {"kind": "mind-map", "text": "Ship", "parent": "#2", "direction": "down"},
        ]))?
        .into_iter()
        .filter_map(|entry| made_of(entry.new))
        .map(|made| (made.at.x(), made.at.y(), made.parent))
        .collect();
        let later = map(json!([
            {"kind": "mind-map", "text": "Build", "parent": "#2"},
            {"kind": "mind-map", "text": "Plan"},
        ]));
        assert_eq!(
            (found, later.is_err()),
            (
                vec![
                    (2000.0, 2000.0, None),
                    (2260.0, 2000.0, Some("#1".to_owned())),
                    (2260.0, 2116.0, Some("#1".to_owned())),
                    (2260.0, 2200.0, Some("#2".to_owned())),
                ],
                true
            )
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
        assert_ne!(refused("nowhere")?, "");
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

    #[test]
    fn a_reaction_stamp_refuses_what_it_cannot_take_and_what_only_it_takes()
    -> Result<()> {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let stamp = |extra: Value| {
            let mut item = json!({"kind": "reaction-stamp"});
            item.as_object_mut()
                .zip(extra.as_object())
                .into_iter()
                .for_each(|(item, extra)| item.extend(extra.clone()));
            item
        };
        assert_eq!(
            [
                refused(stamp(json!({"text": "a"})))?,
                refused(stamp(json!({"size": "10,10"})))?,
                refused(stamp(json!({"color": "red4"})))?,
                refused(stamp(json!({"on": "s1", "frame": "f1"})))?,
                refused(stamp(json!({"reaction": ["like", "heart"]})))?,
                refused(stamp(json!({"reaction": ["thumbs-up"]})))?,
                refused(json!({"text": "a", "on": "s1"}))?,
                refused(json!({"text": "a", "reaction": ["like"]}))?,
                refused(stamp(json!({"on": "s1", "at": "1,2", "reaction": ["+1"]})))?,
            ],
            [
                "text of `object add` must be left out for a reaction stamp, in item 1",
                "size of `object add` must be left out for a reaction stamp, in item 1",
                "color of `object add` must be left out for a reaction stamp, in item 1",
                "frame of `object add` must be left out with on, in item 1",
                "reaction of `object add` must be one key, in item 1",
                "reaction of `object add` must be one of like, heart, star, +1, dislike, figma-question, check or cross, in item 1",
                "on of `object add` must be left out for a sticker, in item 1",
                "reaction of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        Ok(())
    }

    #[test]
    fn a_reaction_stamp_sticks_to_its_object_outside_the_frame_it_is_in() -> Result<()>
    {
        let board = on_board();
        let scene = Scene::new(&board);
        let args =
            args(json!({"kind": "reaction-stamp", "on": "s1", "reaction": ["heart"]}))?;
        let found = entries(&read(&args)?, &[], &scene, scene.get("f1"))?;
        let stamp = found.into_iter().find_map(|entry| {
            let frame = entry.frame;
            made_of(entry.new).map(|made| (frame, made.fields))
        });
        let (frame, fields) = stamp.unzip();
        let field = |name: &str| {
            fields
                .iter()
                .flatten()
                .find(|(each, _)| *each == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            (frame, field("stampKey"), field("stickyPosition"),),
            (
                Some(None),
                Some(json!("heart")),
                Some(json!({
                    "parentId": "s1",
                    "constraints": {"x": "normalized", "y": "normalized"},
                    "x": 0.5,
                    "y": 0.5,
                })),
            )
        );
        Ok(())
    }

    #[test]
    fn a_loose_reaction_stamp_is_like_by_default_and_sticks_to_nothing() -> Result<()> {
        let args = args(json!({"kind": "reaction-stamp", "at": "10,20"}))?;
        let scene = Scene::new(&[]);
        let found = entries(&read(&args)?, &[], &scene, None)?;
        let stamp = found.into_iter().find_map(|entry| made_of(entry.new));
        assert_eq!(
            stamp.map(|made| (
                (made.at.x(), made.at.y()),
                made.fields.contains(&("stampKey", json!("like"))),
                made.fields
                    .iter()
                    .any(|(name, _)| *name == "stickyPosition"),
            )),
            Some(((10.0, 20.0), true, false))
        );
        Ok(())
    }

    #[test]
    fn a_reaction_stamp_sticks_only_to_what_the_web_sticks_it_to() -> Result<()> {
        let refused = |on: &str| -> Result<String> {
            Ok(
                placed_on(&on_board(), json!({"kind": "reaction-stamp", "on": on}))
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default(),
            )
        };
        assert_eq!(
            refused("f1")?,
            "on of `object add` must be a shape, sticker, document, task or image"
        );
        assert_ne!(refused("nowhere")?, "");
        assert_eq!(refused("s1")?, "");
        Ok(())
    }

    fn icon_made(given: Value) -> Result<Option<Made>> {
        let scene = Scene::new(&[]);
        let found = entries(&read(&args(given)?)?, &[], &scene, None)?;
        Ok(found.into_iter().find_map(|entry| made_of(entry.new)))
    }

    #[test]
    fn an_icon_takes_the_defaults_of_the_web_client_and_what_the_call_gives()
    -> Result<()> {
        let stored = |made: Option<Made>| {
            made.map(|made| {
                (
                    (made.at.x(), made.at.y()),
                    made.fields
                        .into_iter()
                        .map(|(name, value)| (name.to_owned(), value))
                        .collect::<Map<_, _>>(),
                )
            })
        };
        let plain =
            icon_made(json!({"kind": "icon", "text": "rocket-launch", "at": "10,20"}))?;
        let asked = icon_made(json!({"kind": "icon", "text": "star", "at": "0,0",
            "weight": "bold", "color": "red9", "size": "96,96"}))?;
        assert_eq!(
            [stored(plain), stored(asked)],
            [
                Some((
                    (10.0, 20.0),
                    json!({"iconName": "rocket-launch", "weight": "duotone",
                        "width": 48.0, "height": 48.0,
                        "fillColor": {"color": "gray12", "opacity": 1}})
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
                )),
                Some((
                    (0.0, 0.0),
                    json!({"iconName": "star", "weight": "bold",
                        "width": 96.0, "height": 96.0,
                        "fillColor": {"color": "red9", "opacity": 1}})
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
                )),
            ]
        );
        Ok(())
    }

    #[test]
    fn an_icon_refuses_a_name_a_color_a_size_and_a_weight_it_cannot_take() -> Result<()>
    {
        let refused = |item: Value| -> Result<String> {
            Ok(read(&args(json!({ "items": [item] }))?)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default())
        };
        let icon = |extra: Value| {
            let mut item = json!({"kind": "icon", "text": "rocket-launch"});
            item.as_object_mut()
                .zip(extra.as_object())
                .into_iter()
                .for_each(|(item, extra)| item.extend(extra.clone()));
            item
        };
        assert_eq!(
            [
                refused(icon(json!({"text": "Rocket Launch"})))?,
                refused(icon(json!({"text": "rocket_launch"})))?,
                refused(json!({"kind": "icon"}))?,
                refused(icon(json!({"color": "red4"})))?,
                refused(icon(json!({"size": "48,64"})))?,
                refused(json!({"text": "a", "weight": "bold"}))?,
                refused(icon(
                    json!({"weight": "fill", "color": "gray8", "size": "24,24"})
                ))?,
            ],
            [
                "text of `object add` must be lower-case letters, digits and dashes, e.g. rocket-launch, in item 1",
                "text of `object add` must be lower-case letters, digits and dashes, e.g. rocket-launch, in item 1",
                "text of `object add` must be given, in item 1",
                "color of `object add` must be one of gray12, gray8, red9, orange9, yellow9, lime9, green9, mint9, blue9, indigo9, violet9 or pink9, in item 1",
                "size of `object add` must be square, e.g. 48,48, in item 1",
                "weight of `object add` must be left out for a sticker, in item 1",
                "",
            ]
        );
        let heavy = args(json!({"kind": "icon", "text": "star", "weight": "heavy"}))
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            heavy.as_deref(),
            Some(
                "weight of `object add icon` must be one of thin, light, regular, bold, fill, duotone"
            )
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
            | NewObject::File { .. }
            | NewObject::Drawing { .. } => None,
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
            let mut txn = doc.transact_mut();
            fields_of
                .into_iter()
                .map(|fields| {
                    let id =
                        fields.get("id").and_then(Value::as_str).unwrap_or_default();
                    let any = Any::from_json(&fields.to_string()).unwrap_or(Any::Null);
                    store.insert(&mut txn, id, any)
                })
                .for_each(drop);
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

    /// What a dry run of two icons says, given the replies after the load.
    async fn icons_dry_run(
        between: Vec<HttpResponse>,
        names: [&str; 2],
    ) -> Result<ChangeReport> {
        let (_, client) = serving(between)?;
        let arguments = args(json!({"dry_run": true, "items": [
            {"kind": "icon", "text": names[0]},
            {"kind": "icon", "text": names[1]}]}))?;
        added(
            &client,
            Loading {
                connector: &board_document(),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &arguments,
            &read(&arguments)?,
        )
        .await
    }

    const ICON_MANIFEST: &str =
        r#"[{"n":"rocket"},{"n":"rocket-launch"},{"n":"star"}]"#;

    #[tokio::test]
    async fn an_icon_the_manifest_lacks_is_refused_with_close_names_even_in_a_dry_run()
    -> Result<()> {
        let refused =
            icons_dry_run(vec![reply(200, &[], ICON_MANIFEST)], ["star", "rokcet"])
                .await;
        assert_eq!(
            error_of(refused).as_deref(),
            Some(
                "\"rokcet\" is not an icon: expected an icon name, e.g. rocket-launch \
                (close: rocket, rocket-launch)"
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn icons_the_manifest_lists_are_written_after_one_request_for_the_manifest()
    -> Result<()> {
        let written = icons_dry_run(
            vec![reply(200, &[], ICON_MANIFEST)],
            ["star", "rocket-launch"],
        )
        .await?;
        assert_eq!(written.warnings, Vec::<String>::new());
        Ok(())
    }

    #[tokio::test]
    async fn a_manifest_that_cannot_be_read_is_a_warning_and_the_write_goes_on()
    -> Result<()> {
        let written = icons_dry_run(
            vec![reply(404, &[], ""), reply(404, &[], "")],
            ["star", "rokcet"],
        )
        .await?;
        assert_eq!(
            written.warnings,
            ["icons not checked: the icon list could not be read"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_icon_manifest_is_read_once_for_the_client_and_its_clones() -> Result<()>
    {
        let (fake, client) = client(vec![reply(200, &[], ICON_MANIFEST)])?;
        let items = read(&args(json!({"kind": "icon", "text": "star"}))?)?;
        let first = icons_checked(&client, &items).await?;
        let second = icons_checked(&client.clone(), &items).await?;
        let reads = fake
            .requests()
            .iter()
            .filter(|request| request.url.path().contains(MANIFEST_PATH))
            .count();
        assert_eq!((first, second, reads), (Vec::new(), Vec::new(), 1));
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
            | NewObject::Drawing { .. }
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
            | NewObject::Drawing { .. }
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
    async fn a_task_with_an_unknown_status_is_refused_before_a_picture_is_uploaded()
    -> Result<()> {
        let (fake, client) = serving(vec![reply(200, &[], r#"{"name":"u-2.png"}"#)])?;
        let path = local("a.png", &png(10, 10))?;
        let arguments = args(json!({"items": [
            {"kind": "image", "path": path},
            {"kind": "task", "text": "x", "status": "Nope"}]}))?;
        let refused = added(
            &client,
            Loading {
                connector: &board_document(),
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
        assert_eq!((refused.is_err(), uploads), (true, 0));
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
