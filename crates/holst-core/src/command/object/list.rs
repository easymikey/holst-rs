//! `object ls`: the objects of a board or a part of it.

use std::collections::BTreeMap;

use itertools::Itertools as _;
use serde::Serialize;
use yrs::Doc;

use crate::{
    command::{
        board::show::{
            Loading,
            board_id,
            cache_use,
            chosen_area,
            explicit_area,
            through_cache,
        },
        board_load::{self, RemoteBoard},
        object::LsArgs,
    },
    domain::{
        area::{
            listed_area,
            scene::{Scene, drawn_at_scale, text_scale},
        },
        board::{
            DrawnArrowEnds,
            EditRight,
            Object,
            ObjectKind,
            detail::{LinkCard, Reactions, TagNames},
            markdown_text,
            objects as board_objects,
            query::ObjectQuery,
            text_marks,
        },
        geometry::Rectangle,
        id,
        kinds::{self, Detail},
        limit::Limit,
        picture::BoardLength,
        rich_text::TextFormat,
        text::{one_line, shortened},
        tree::name_index::build::normalized,
    },
    effect::settings::CacheSettings,
    error::Result,
    protocol::document::{COLOR, FIXED_WIDTH, TEXT_BACKGROUND_COLOR, TEXT_COLOR},
    remote::client::Client,
};

const COMMAND: &str = "object ls";

/// A page of a board's objects that match the filters.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub board: String,
    /// A link to the board, or to the selected frame or object.
    pub link: String,
    pub edit_right: EditRight,
    /// The part listed with its padding; `None` for the whole board.
    pub rectangle: Option<Rectangle>,
    /// How many objects match, all pages together.
    pub matched: usize,
    /// How many matches the page skips.
    pub from: usize,
    pub objects: Vec<ListedObject>,
    /// The offset of the next page, when matches are left.
    pub next: Option<usize>,
}

/// An object with its link, its drawn ends, its file and what it shows beside.
#[derive(Debug, Serialize)]
pub struct ListedObject {
    #[serde(flatten)]
    pub object: Object,
    pub link: String,
    #[serde(flatten)]
    pub drawn: Option<DrawnArrowEnds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<ListedFile>,
    /// The page of a link card: `url` and `title`.
    #[serde(flatten)]
    pub card: Option<LinkCard>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// How often each reaction was given: `{"like": 2}`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub reactions: BTreeMap<String, usize>,
    #[serde(flatten)]
    pub style: TextStyle,
    /// The words the object's kind adds: `{"rows": 2, "columns": 3}`.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_detail"
    )]
    pub detail: Vec<(&'static str, serde_json::Value)>,
}

impl ListedObject {
    /// The listing line, without the link: `name` writes an id, the text is
    /// cut after `max_chars` characters.
    #[must_use]
    pub fn line(&self, name: impl Fn(&str) -> String, max_chars: usize) -> String {
        let object = &self.object;
        let corner = format!("({:.0}, {:.0})", object.x, object.y);
        let size = object
            .width
            .zip(object.height)
            .map(|(width, height)| format!(" {width:.0}×{height:.0}"))
            .unwrap_or_default();
        let inside = object
            .parent
            .as_deref()
            .map(|parent| format!(" in {}", name(parent)))
            .unwrap_or_default();
        let color = object
            .color
            .as_deref()
            .map(|color| format!("  {color}"))
            .unwrap_or_default();
        let look = object
            .look
            .as_ref()
            .map(|look| format!("  {}", look.words()))
            .unwrap_or_default();
        let style = self.style.line();
        let ends = object
            .ends
            .as_ref()
            .map(|ends| {
                let end = |end: &Option<String>| {
                    end.as_deref().map_or_else(|| "·".to_owned(), &name)
                };
                format!("  {} → {}", end(&ends.start), end(&ends.end))
            })
            .unwrap_or_default();
        let tags = if self.tags.is_empty() {
            String::new()
        } else {
            format!("  #{}", self.tags.join(" #"))
        };
        let reactions = if self.reactions.is_empty() {
            String::new()
        } else {
            let words = self
                .reactions
                .iter()
                .map(|(reaction, count)| format!("{reaction}:{count}"));
            format!("  {}", words.collect::<Vec<_>>().join(" "))
        };
        let padlock = if object.locked { "  locked" } else { "" };
        let turn = if object.rotation == 0.0 {
            String::new()
        } else {
            format!("  {}°", (object.rotation * 100.0).round() / 100.0)
        };
        let linked = object
            .link_to
            .as_deref()
            .map_or_else(String::new, |link| format!("  link-to={link}"));
        let detail = if self.detail.is_empty() {
            String::new()
        } else {
            format!(
                "  [{}]",
                named(&self.detail, &name).iter().map(detail_word).join(" ")
            )
        };
        let text = shortened(&one_line(&object.text), max_chars);
        let quoted = if text.is_empty() {
            String::new()
        } else {
            format!("  \"{text}\"")
        };
        format!(
            "{} {:<7} {corner}{size}{inside}{color}{look}{style}{detail}{ends}{padlock}{turn}{linked}{tags}{reactions}{quoted}",
            name(&object.id),
            object.kind.short_name(),
        )
    }
}

/// The detail with the word that holds an object id (`on`) written as `name`
/// writes an object.
fn named(
    detail: &[(&'static str, serde_json::Value)],
    name: impl Fn(&str) -> String,
) -> Vec<(&'static str, serde_json::Value)> {
    detail
        .iter()
        .map(|(word, value)| {
            let shown = value.as_str().filter(|_| *word == "on").map_or_else(
                || value.clone(),
                |id| serde_json::Value::String(name(id)),
            );
            (*word, shown)
        })
        .collect()
}

/// A word of the kind's detail: `rows=2`; a text is on one line like the
/// listed text, and quoted with a space; a list is its length.
fn detail_word((word, value): &(&'static str, serde_json::Value)) -> String {
    let shown = match value {
        serde_json::Value::String(text) => one_line(text).into_owned(),
        serde_json::Value::Array(items) => items.len().to_string(),
        value @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::Object(_)) => value.to_string(),
    };
    match shown {
        text if text.contains(' ') => format!("{word}=\"{text}\""),
        text => format!("{word}={text}"),
    }
}

fn serialize_detail<S: serde::Serializer>(
    detail: &[(&'static str, serde_json::Value)],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_map(detail.iter().map(|(word, value)| (word, value)))
}

/// The marks all of the text carries and the colors set on the object.
#[derive(Debug, Default, Serialize)]
pub struct TextStyle {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_background: Option<String>,
    /// The width a text is held to as drawn; none when it wraps by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_width: Option<f64>,
}

impl TextStyle {
    fn of(document: &Doc, object: &Object) -> Self {
        let color = |name: &str| {
            object
                .fields
                .get(name)
                .and_then(|paint| paint.get(COLOR))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Self {
            marks: text_marks(document, object),
            text_color: color(TEXT_COLOR),
            text_background: color(TEXT_BACKGROUND_COLOR),
            text_width: object
                .fields
                .get(FIXED_WIDTH)
                .and_then(serde_json::Value::as_bool)
                .filter(|fixed| *fixed)
                .and(object.width)
                .map(|stored| (stored * text_scale(object) * 100.0).round() / 100.0),
        }
    }

    /// The word of a listing line, e.g. `{bold,text:red6,bg:yellow4}`; empty
    /// for a plain text.
    #[must_use]
    pub fn line(&self) -> String {
        let marks = self.marks.iter().map(|mark| (*mark).to_owned());
        let text = self.text_color.as_ref().map(|name| format!("text:{name}"));
        let background = self
            .text_background
            .as_ref()
            .map(|name| format!("bg:{name}"));
        let width = self.text_width.map(|width| format!("width:{width}"));
        let words: Vec<String> =
            marks.chain(text).chain(background).chain(width).collect();
        if words.is_empty() {
            String::new()
        } else {
            format!("  {{{}}}", words.join(","))
        }
    }
}

/// The stored file of an image or a file object, with its address.
#[derive(Debug, Serialize)]
pub struct ListedFile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    pub url: String,
}

/// Lists the objects the arguments ask for.
///
/// # Errors
/// An area that is not valid, `--refresh` with `--offline`, a kind nobody has,
/// a board that is not found, and errors of the requests.
pub async fn ls(
    client: &Client,
    cache: &CacheSettings,
    args: &LsArgs,
) -> Result<Listing> {
    let cache_use = cache_use(&args.caching, COMMAND)?;
    through_cache(client, cache, cache_use, async |loading| {
        listing(client, loading, args).await
    })
    .await
}

async fn listing(
    client: &Client,
    loading: Loading<'_>,
    args: &LsArgs,
) -> Result<Listing> {
    let selection = &args.selection;
    let asked = explicit_area(selection)?;
    let padding = BoardLength::optional(selection.padding)?;
    let kind = args.kind.as_deref().and_then(ObjectKind::typed);
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
    let objects = board_objects(&loaded.document);
    let area = chosen_area(&asked, id::object_in(&args.board), &objects)?;
    let selected = listed_area(&objects, &area, padding)?;
    let tags = TagNames::of(&loaded.document);
    let reactions = Reactions::of(&loaded.document);
    let tag = args.tag.as_deref().filter(|tag| !tag.trim().is_empty());
    let query = ObjectQuery {
        kind,
        text: args
            .contains
            .as_deref()
            .filter(|text| !normalized(text).is_empty()),
        offset: args.offset.map_or(0, widen),
        limit: Limit::from(args.limit.map_or(0, widen)),
    };
    let page = query.page_where(&selected.objects, |object| {
        tag.is_none_or(|tag| tags.has(object, tag))
    })?;
    let scene = Scene::new(&objects);
    let target = selected.target.map(|object| object.id.as_str());
    let base = client.base_url();
    let storage = base.storage();
    Ok(Listing {
        link: base.link(&board, target),
        edit_right: loaded.right,
        rectangle: selected.rectangle,
        matched: page.matched,
        from: page.from,
        next: page.next(),
        objects: page
            .shown
            .iter()
            .map(|&object| {
                let Detail { text, words } = kinds::detail(&loaded.document, object);
                let text = text.unwrap_or_else(|| {
                    listed_text(&loaded.document, object, args.text_format)
                });
                ListedObject {
                    link: base.object_link(&board, &object.id),
                    drawn: object.ends.as_ref().and_then(|ends| ends.drawn),
                    file: object.file.as_ref().map(|file| ListedFile {
                        name: file.name.clone(),
                        original: file.original.clone(),
                        url: format!("{storage}{}", file.name),
                    }),
                    card: LinkCard::of(object),
                    tags: tags.of_object(object),
                    reactions: reactions.of_object(object),
                    style: TextStyle::of(&loaded.document, object),
                    detail: words,
                    object: Object {
                        fields: serde_json::Value::Null,
                        text,
                        ..drawn_size(&scene, object)
                    },
                }
            })
            .collect(),
        board,
    })
}

/// The text of the object: its markdown when asked for and the object has one.
fn listed_text(document: &Doc, object: &Object, format: Option<TextFormat>) -> String {
    match format {
        Some(TextFormat::Markdown) => markdown_text(document, object),
        Some(TextFormat::Plain) | None => None,
    }
    .unwrap_or_else(|| object.text.clone())
}

/// The object, with the size it is drawn at when a scale sets that.
fn drawn_size(scene: &Scene<'_>, object: &Object) -> Object {
    let scaled = drawn_at_scale(&object.kind)
        || matches!(
            object.kind,
            ObjectKind::Sticker | ObjectKind::StickerStack | ObjectKind::Table
        );
    let rectangle = scene.rectangle_of(object).filter(|_| scaled);
    Object {
        width: rectangle.map_or(object.width, |rect| Some(rect.width)),
        height: rectangle.map_or(object.height, |rect| Some(rect.height)),
        ..object.clone()
    }
}

fn widen(count: u32) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Any, Doc, Map as _, ReadTxn as _, StateVector, Transact as _};

    use super::{detail_word, named};
    use crate::{
        Result,
        command::{
            board::show::Loading,
            board_load::CacheUse,
            fixture::{arguments, channel, client},
            object::{
                LsArgs,
                list::{Listing, listing},
            },
        },
        domain::{board::EditRight, id::short_id},
        protocol::hud::{DocumentResponse, UpdatePayload},
        remote::socket::fake::{Scripted, answer, script, update},
    };

    fn objects() -> Vec<Value> {
        let at = |x: i32, y: i32| json!({"x": x, "y": y});
        vec![
            json!({"id": "s1", "type": "sticker", "position": at(0, 0), "width": 192,
                   "height": 192, "labelText": "Buy milk", "tagIds": ["t1"]}),
            json!({"id": "s2", "type": "sticker", "position": at(400, 0), "width": 192,
                   "height": 192, "labelText": "Roadmap"}),
            json!({"id": "l1", "type": "link", "position": at(1000, 0),
                   "link": "https://example.com", "title": "Example Domain"}),
            json!({"id": "f1", "type": "frame", "position": at(0, 500), "width": 800,
                   "height": 600, "labelText": "Q3"}),
            json!({"id": "s3", "type": "sticker", "position": at(50, 550), "width": 192,
                   "height": 192, "labelText": "Inside"}),
            json!({"id": "tb1", "type": "table", "position": at(2000, 0),
                   "row-1": {"index": 0}, "row-2": {"index": 1},
                   "column-1": {"index": 0}, "column-2": {"index": 1},
                   "column-3": {"index": 2}}),
        ]
    }

    /// A board of `objects`, one tag, `t1`, named "Urgent", and two likes and
    /// a fire on `s2`.
    fn board() -> Scripted {
        let doc = Doc::with_client_id(4);
        let (store, tags, reactions) = (
            doc.get_or_insert_map("objects"),
            doc.get_or_insert_map("task-card-tagsStore"),
            doc.get_or_insert_map("reactionsStore"),
        );
        {
            // imperative: yrs inserts through one `&mut` transaction
            let mut txn = doc.transact_mut();
            let any =
                |value: &Value| Any::from_json(&value.to_string()).unwrap_or(Any::Null);
            // imperative: yrs inserts through one `&mut` transaction
            for fields in objects() {
                let id = fields.get("id").and_then(Value::as_str).unwrap_or_default();
                store.insert(&mut txn, id, any(&fields));
            }
            tags.insert(&mut txn, "t1", any(&json!({"id": "t1", "title": "Urgent"})));
            // imperative: yrs inserts through one `&mut` transaction
            for (entry, reaction) in [("r1", "like"), ("r2", "like"), ("r3", "fire")] {
                let fields =
                    json!({"id": entry, "targetId": "s2", "reactionId": reaction});
                reactions.insert(&mut txn, entry, any(&fields));
            }
        }
        let state = doc
            .transact()
            .encode_state_as_update_v2(&StateVector::default());
        script([Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update(0, UpdatePayload::Inline(state)),
        ])])
    }

    async fn run(given: Value) -> Result<Listing> {
        listing(
            &client([channel()])?.1,
            Loading {
                connector: &board(),
                cache: None,
                cache_use: CacheUse::Use,
            },
            &arguments::<LsArgs>(given)?,
        )
        .await
    }

    fn ids(listing: &Listing) -> Vec<&str> {
        listing
            .objects
            .iter()
            .map(|listed| listed.object.id.as_str())
            .collect()
    }

    #[tokio::test]
    async fn a_tag_filters_by_name_and_is_shown() -> Result<()> {
        let tagged = run(json!({ "tag": "urgent" })).await?;
        let none = run(json!({ "tag": "later" })).await?;
        assert_eq!(
            (
                ids(&tagged),
                tagged.objects.first().map(|listed| listed.tags.clone()),
                none.matched
            ),
            (vec!["s1"], Some(vec!["Urgent".to_owned()]), 0)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_link_card_keeps_its_address_and_title() -> Result<()> {
        let listing = run(json!({ "kind": "link" })).await?;
        let json = serde_json::to_value(&listing)?;
        let card = json.pointer("/objects/0");
        assert_eq!(
            (
                card.and_then(|card| card.get("url")),
                card.and_then(|card| card.get("title")),
            ),
            (
                Some(&json!("https://example.com")),
                Some(&json!("Example Domain"))
            )
        );
        Ok(())
    }

    #[test]
    fn a_list_in_the_detail_is_shown_as_its_length() {
        assert_eq!(
            [
                detail_word(&("choices", json!(["Pizza", "Sushi", "Soup"]))),
                detail_word(&("selected", json!("none"))),
                detail_word(&("selected", json!("Hot soup"))),
            ],
            ["choices=3", "selected=none", "selected=\"Hot soup\""]
        );
    }

    #[test]
    fn the_object_a_detail_word_names_is_written_as_the_caller_writes_objects() {
        let detail = [("reaction", json!("like")), ("on", json!("a1b2c3d4e5f6"))];
        let shown: Vec<String> = named(&detail, |id| format!("@{}", short_id(id)))
            .iter()
            .map(detail_word)
            .collect();
        assert_eq!(shown, ["reaction=like", "on=@a1b2c3d4"]);
    }

    #[test]
    fn a_text_of_several_lines_in_the_detail_stays_on_one_line() {
        assert_eq!(
            [
                detail_word(&("back", json!("a\nb"))),
                detail_word(&("back", json!("a\n\nb\nc"))),
            ],
            ["back=\"a / b\"", "back=\"a / b / c\""]
        );
    }

    #[tokio::test]
    async fn a_table_shows_its_rows_and_columns() -> Result<()> {
        let listing = run(json!({ "kind": "table" })).await?;
        let json = serde_json::to_value(&listing)?;
        let line = listing
            .objects
            .first()
            .map(|listed| listed.line(str::to_owned, 40));
        assert_eq!(
            (json.pointer("/objects/0/detail"), line.as_deref()),
            (
                Some(&json!({"rows": 2, "columns": 3})),
                Some("tb1 table   (2000, 0) 720×120  [rows=2 columns=3]")
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn reactions_are_counted_per_object_in_the_word_and_the_json() -> Result<()> {
        let listing = run(json!({ "contains": "Roadmap" })).await?;
        let json = serde_json::to_value(&listing)?;
        let plain = run(json!({ "contains": "Buy milk" })).await?;
        let line = listing
            .objects
            .first()
            .map(|listed| listed.line(str::to_owned, 40));
        assert_eq!(
            (
                json.pointer("/objects/0/reactions"),
                line.as_deref(),
                serde_json::to_value(&plain)?
                    .pointer("/objects/0/reactions")
                    .cloned(),
            ),
            (
                Some(&json!({"like": 2, "fire": 1})),
                Some("s2 sticker (400, 0) 192×192  fire:1 like:2  \"Roadmap\""),
                None
            )
        );
        Ok(())
    }
}
