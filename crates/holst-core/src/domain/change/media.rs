//! What an edit changes of an image or a file: name, crop mask, flip.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{check::rectangle_of, value::ImageUpload};
use crate::{
    domain::{
        board::ObjectKind,
        geometry::{Rectangle, Size},
        response::PagesInfo,
    },
    error::{Error, Result, board_change::BoardChangeError, input::InputError},
    protocol::document::{
        CROP_TRANSFORM,
        DISPLAY_FILE_NAME,
        HEIGHT,
        NAME,
        ORIGINAL_NAME,
        POSITION,
        ROTATION,
        WIDTH,
        X,
        Y,
    },
};

const FLIP: &str = "flip";

/// The shape an image is cut to; `none` gives back the whole picture.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CropMask {
    Circle,
    Square,
    Portrait,
    Landscape,
    Wide,
    None,
}

impl CropMask {
    /// The width and height the mask keeps; none for the whole picture.
    const fn ratio(self) -> Option<(f64, f64)> {
        match self {
            CropMask::Circle | CropMask::Square => Some((1.0, 1.0)),
            CropMask::Portrait => Some((3.0, 4.0)),
            CropMask::Landscape => Some((4.0, 3.0)),
            CropMask::Wide => Some((16.0, 9.0)),
            CropMask::None => None,
        }
    }

    /// The mask a stored fifth number of `cropTransform` names.
    pub(crate) fn of_stored(number: u64) -> Option<CropMask> {
        [
            CropMask::Circle,
            CropMask::Square,
            CropMask::Portrait,
            CropMask::Landscape,
            CropMask::Wide,
        ]
        .into_iter()
        .find(|mask| mask.stored() == json!(number))
    }

    /// The number the document stores as the fifth of `cropTransform`.
    fn stored(self) -> Value {
        match self {
            CropMask::Circle => json!(1),
            CropMask::Square => json!(2),
            CropMask::Portrait => json!(3),
            CropMask::Landscape => json!(4),
            CropMask::Wide => json!(5),
            CropMask::None => Value::Null,
        }
    }
}

/// Which axes of an image are mirrored.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FlipState {
    None,
    Horizontal,
    Vertical,
    Both,
}

impl FlipState {
    /// The axes the stored `flip` of an image mirrors.
    pub(crate) fn of_stored(fields: &Value) -> FlipState {
        match mirrored(fields) {
            (false, false) => FlipState::None,
            (true, false) => FlipState::Horizontal,
            (false, true) => FlipState::Vertical,
            (true, true) => FlipState::Both,
        }
    }
}

/// How a file is shown: as an icon card or as its content.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FileDisplay {
    Stamp,
    Content,
}

impl FileDisplay {
    /// The display a stored `displayType` names.
    pub(crate) fn of_stored(word: &str) -> Option<FileDisplay> {
        [FileDisplay::Stamp, FileDisplay::Content]
            .into_iter()
            .find(|display| <&str>::from(*display) == word)
    }
}

/// What an edit changes of an image or a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaEdit {
    /// The name shown for the file.
    pub name: Option<String>,
    /// The mask an image is cut to.
    pub crop: Option<CropMask>,
    /// The axes an image is mirrored on.
    pub flip: Option<FlipState>,
    /// The picture that takes the place of an image's file.
    pub replace: Option<ImageUpload>,
    /// How a file is shown.
    pub display: Option<FileDisplay>,
    /// The page a file of pages opens on.
    pub start_page: Option<u32>,
    /// Whether a video hides its details.
    pub hide_details: Option<bool>,
    /// The pages the page service counted, to store beside the display type.
    pub pages: Option<PagesInfo>,
}

impl MediaEdit {
    /// The names of the fields asked for, as `object edit` words them.
    #[must_use]
    pub fn fields(&self) -> Vec<&'static str> {
        self.name
            .iter()
            .map(|_| "name")
            .chain(self.crop.map(|_| "crop"))
            .chain(self.flip.map(|_| "flip"))
            .chain(self.replace.as_ref().map(|_| "path"))
            .chain(self.display.map(|_| "display"))
            .chain(self.start_page.map(|_| "start_page"))
            .chain(self.hide_details.map(|_| "hide_details"))
            .collect()
    }
}

/// The field that holds the name shown for the kind, if it has one.
fn name_field(kind: &ObjectKind) -> Option<&'static str> {
    [
        (ObjectKind::Image, ORIGINAL_NAME),
        (ObjectKind::File, DISPLAY_FILE_NAME),
    ]
    .into_iter()
    .find_map(|(stored, field)| (stored == *kind).then_some(field))
}

pub(super) fn fits(
    kind: &ObjectKind,
    fields: Option<&Value>,
    media: &MediaEdit,
) -> Result<()> {
    let only = |given: bool, held: bool, field| {
        (!given || held).then_some(()).ok_or_else(|| {
            Error::BoardChange(BoardChangeError::NotChangeable {
                kind: kind.as_str().to_owned(),
                field,
            })
        })
    };
    let image = *kind == ObjectKind::Image;
    let file = *kind == ObjectKind::File;
    match (&media.name, name_field(kind)) {
        (Some(_), None) => only(true, false, "name"),
        (Some(name), Some(_)) if name.trim().is_empty() => {
            Err(Error::Input(InputError::BlankMediaName))
        }
        _ => Ok(()),
    }
    .and(only(media.crop.is_some(), image, "crop"))
    .and(only(media.flip.is_some(), image, "flip"))
    .and(only(media.replace.is_some(), image, "path"))
    .and(only(media.display.is_some(), file, "display"))
    .and(only(media.start_page.is_some(), file, "start_page"))
    .and(only(media.hide_details.is_some(), file, "hide_details"))
    .and(view_fits(fields.unwrap_or(&Value::Null), media))
}

/// The stored name of a page file whose pages were never counted.
///
/// BUNDLE: the page service is asked only for a name it converts and only when
/// the object has no `pagesInfo` (`filePagesInfo`, `preparePagePreview`,
/// main.a33651a4.js 209839-209870; `VN`, 169688).
pub(crate) fn uncounted(fields: &Value) -> Option<&str> {
    fields
        .get("fileName")
        .and_then(Value::as_str)
        .filter(|name| {
            let lower = name.to_lowercase();
            PAGE_EXTENSIONS.iter().any(|end| lower.ends_with(end))
        })
        .filter(|_| fields.get("pagesInfo").is_none_or(Value::is_null))
}

/// `fields` with the pages the service counted in place of the stored ones.
fn counted<'a>(fields: &'a Value, media: &MediaEdit) -> Cow<'a, Value> {
    media.pages.map_or(Cow::Borrowed(fields), |pages| {
        let mut owned = fields.as_object().cloned().unwrap_or_default();
        owned.insert("pagesInfo".to_owned(), json!(pages));
        Cow::Owned(Value::Object(owned))
    })
}

/// What a file shows as its content, by the extension of its stored name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileView {
    Pages,
    Audio,
    Video,
    Plain,
}

const PAGE_EXTENSIONS: [&str; 5] = [".pdf", ".docx", ".doc", ".pptx", ".ppt"];
const AUDIO_EXTENSIONS: [&str; 7] =
    [".mp3", ".m4a", ".wav", ".ogg", ".aac", ".flac", ".webm"];
const VIDEO_EXTENSIONS: [&str; 4] = [".mp4", ".m4v", ".mov", ".webm"];

/// BUNDLE: a file is a page file by its extension unless its page info is
/// invalid, audio or video by its extension, a `.webm` by the info it holds
/// (`isPageFile`, `sL.U`, main.a33651a4.js 117089-117101, 169625-169646).
fn view_of(fields: &Value) -> FileView {
    let name = fields
        .get("fileName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let ends = |list: &[&str]| list.iter().any(|end| name.ends_with(end));
    let held = |key: &str| fields.get(key).is_some_and(|info| !info.is_null());
    let valid = fields
        .get("pagesInfo")
        .is_none_or(|info| info.get("valid").and_then(Value::as_bool) == Some(true));
    match (
        ends(&PAGE_EXTENSIONS) && valid,
        ends(&AUDIO_EXTENSIONS),
        ends(&VIDEO_EXTENSIONS),
    ) {
        (true, _, _) => FileView::Pages,
        (_, true, true) => match (held("audioInfo"), held("videoInfo")) {
            (true, false) => FileView::Audio,
            _ => FileView::Video,
        },
        (_, true, false) => FileView::Audio,
        (_, false, true) => FileView::Video,
        _ => FileView::Plain,
    }
}

/// The page count the file holds, when its pages are valid.
pub(crate) fn page_count(fields: &Value) -> Option<u32> {
    fields
        .get("pagesInfo")
        .filter(|info| info.get("valid").and_then(Value::as_bool) == Some(true))
        .and_then(|info| info.get("count"))
        .and_then(Value::as_u64)
        .and_then(|count| u32::try_from(count).ok())
}

/// BUNDLE: content needs the info the view shows (main.a33651a4.js
/// 116314-116316, 116869-116870, 116938-116939); the start page is one of the
/// pages (117168); details are offered for a video shown as content (117266).
fn view_fits(stored: &Value, media: &MediaEdit) -> Result<()> {
    let fields = &*counted(stored, media);
    let refuse = |kind: &str, field| {
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            kind: kind.to_owned(),
            field,
        }))
    };
    let view = view_of(fields);
    let held = |key: &str| fields.get(key).is_some_and(|info| !info.is_null());
    // An uncounted page file is counted before the write, so it passes here.
    let asking =
        media.display == Some(FileDisplay::Content) && uncounted(fields).is_some();
    let measured = match view {
        FileView::Pages => page_count(fields).is_some() || asking,
        FileView::Audio => held("audioInfo"),
        FileView::Video => held("videoInfo"),
        FileView::Plain => false,
    };
    let content = media.display.map_or_else(
        || stored_display(fields) == "content",
        |wanted| wanted == FileDisplay::Content,
    );
    let page = media.start_page.map(|page| (page, page_count(fields)));
    match (media.display, page, media.hide_details) {
        (Some(FileDisplay::Content), _, _) if !measured => {
            refuse("file without the info to show its content", "display")
        }
        (_, Some((_, None)), _) if !asking => {
            refuse("file without page info", "start_page")
        }
        (_, Some((page, Some(count))), _) if !(1..=count).contains(&page) => {
            Err(Error::BoardChange(BoardChangeError::NoPage { page, count }))
        }
        (_, _, Some(_)) if view != FileView::Video => {
            refuse("file that is not a video", "hide_details")
        }
        (_, _, Some(_)) if !content => refuse("video shown as a stamp", "hide_details"),
        _ => Ok(()),
    }
}

/// `stamp` when the file has no display type.
fn stored_display(fields: &Value) -> &str {
    fields
        .get("displayType")
        .and_then(Value::as_str)
        .unwrap_or("stamp")
}

/// BUNDLE: the toolbar offers a corner radius only when the mask is not a
/// circle (main.a33651a4.js 109372); the mask is the one the same edit sets.
pub(super) fn corners_fit(
    kind: &ObjectKind,
    fields: Option<&Value>,
    media: &MediaEdit,
    radius: Option<u32>,
) -> Result<()> {
    let stored = fields
        .and_then(|fields| fields.get(CROP_TRANSFORM))
        .and_then(|crop| crop.get(4))
        .and_then(Value::as_f64)
        == CropMask::Circle.stored().as_f64();
    let circle = media.crop.map_or(stored, |mask| mask == CropMask::Circle);
    match (circle, radius, *kind == ObjectKind::Image) {
        (true, Some(_), true) => {
            Err(Error::BoardChange(BoardChangeError::NotChangeable {
                kind: "circle-cropped image".to_owned(),
                field: "radius",
            }))
        }
        _ => Ok(()),
    }
}

/// The text after the last dot, as the web client reads an extension.
fn extension(name: &str) -> Option<&str> {
    name.trim_end()
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
}

/// BUNDLE: the toolbar trims the name, compares it with the current one
/// after adding the current extension when the name does not end with it, and
/// writes the trimmed name without the extension (`getUpdate`, main.a33651a4.js
/// 108918-108926); both kinds go through it.
pub(super) fn changes(
    kind: &ObjectKind,
    media: &MediaEdit,
    fields: &Value,
) -> Vec<(&'static str, Value)> {
    let image = *kind == ObjectKind::Image;
    let name =
        name_field(kind)
            .zip(media.name.as_deref())
            .and_then(|(field, given)| {
                let current = fields
                    .get(field)
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let kept = extension(current).unwrap_or_default();
                let given = given.trim();
                let named = given
                    .strip_suffix(kept)
                    .map_or_else(|| format!("{given}.{kept}"), |_| given.to_owned());
                (named != current).then_some((field, json!(given)))
            });
    name.into_iter()
        .chain(
            media
                .crop
                .filter(|_| image)
                .into_iter()
                .flat_map(|mask| crop_changes(mask, fields)),
        )
        .chain(
            media
                .flip
                .filter(|_| image)
                .and_then(|state| flip_change(state, fields)),
        )
        .chain(
            media
                .replace
                .iter()
                .filter(|_| image)
                .flat_map(|upload| replace_changes(upload, fields)),
        )
        .chain(view_changes(kind, media, fields))
        .collect()
}

/// The display type, start page and details the file does not hold yet.
fn view_changes(
    kind: &ObjectKind,
    media: &MediaEdit,
    fields: &Value,
) -> Vec<(&'static str, Value)> {
    let file = *kind == ObjectKind::File;
    let pages = media.pages.map(|pages| ("pagesInfo", json!(pages)));
    let display = media.display.and_then(|wanted| {
        let word = match wanted {
            FileDisplay::Stamp => "stamp",
            FileDisplay::Content => "content",
        };
        (stored_display(fields) != word).then(|| ("displayType", json!(word)))
    });
    let page = media.start_page.and_then(|page| {
        let stored = fields
            .get("pinnedPage")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        (stored != u64::from(page)).then(|| ("pinnedPage", json!(page)))
    });
    let details = media.hide_details.and_then(|hidden| {
        let stored = fields
            .get("hideDetails")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (stored != hidden).then(|| ("hideDetails", json!(hidden)))
    });
    [pages, display, page, details]
        .into_iter()
        .flatten()
        .filter(|_| file)
        .collect()
}

/// BUNDLE: a picture pasted over an image covers the old box keeping its own
/// proportions, centred; the crop is the box seen over that cover, under the
/// mask the image has; corner, width and height stay
/// (`pasteImageToSelectedImage`, main.a33651a4.js 128727-128765).
fn replace_changes(upload: &ImageUpload, fields: &Value) -> Vec<(&'static str, Value)> {
    let pixels = upload.pixels;
    let crop =
        rectangle_of(fields).map(|now| (CROP_TRANSFORM, covered(now, pixels, fields)));
    [
        ("naturalWidth", json!(pixels.width())),
        ("naturalHeight", json!(pixels.height())),
        ("imageType", json!(upload.file.mime)),
        (ORIGINAL_NAME, json!(upload.file.original)),
        (NAME, json!(upload.file.name)),
    ]
    .into_iter()
    .chain(crop)
    .collect()
}

/// The `cropTransform` that shows the box `now` over a picture of `pixels`
/// laid to cover it.
fn covered(now: Rectangle, pixels: Size, fields: &Value) -> Value {
    let ratio = pixels.width() / pixels.height();
    let (wide, high) = (now.height * ratio, now.width / ratio);
    let cover = pick(
        now.width / now.height > ratio,
        Rectangle::new(now.x, now.y - (high - now.height) / 2.0, now.width, high),
        Rectangle::new(now.x - (wide - now.width) / 2.0, now.y, wide, now.height),
    );
    let [x, y, width, height] = [
        (now.x - cover.x) / cover.width,
        (now.y - cover.y) / cover.height,
        now.width / cover.width,
        now.height / cover.height,
    ]
    .map(round_places);
    let mask = fields
        .get(CROP_TRANSFORM)
        .and_then(|crop| crop.get(4))
        .cloned()
        .unwrap_or(Value::Null);
    json!([x, y, width, height, mask])
}

/// BUNDLE: `flip` is `{x, y}` of 1 or -1; `horizontal` is `x` (main.a33651a4.js
/// 109124-109134, 109238-109240). The state is set, not toggled.
fn flip_change(state: FlipState, fields: &Value) -> Option<(&'static str, Value)> {
    let wanted = match state {
        FlipState::None => (false, false),
        FlipState::Horizontal => (true, false),
        FlipState::Vertical => (false, true),
        FlipState::Both => (true, true),
    };
    let sign = |mirrored: bool| pick(mirrored, -1, 1);
    (wanted != mirrored(fields))
        .then(|| (FLIP, json!({ (X): sign(wanted.0), (Y): sign(wanted.1) })))
}

/// `flipped` when `mirrored`, else `plain`: a mirrored axis reads from its far
/// side.
fn pick<T>(mirrored: bool, flipped: T, plain: T) -> T {
    match mirrored {
        on if on => flipped,
        _ => plain,
    }
}

/// The axes `flip` mirrors.
fn mirrored(fields: &Value) -> (bool, bool) {
    let on = |axis: &str| {
        fields
            .get(FLIP)
            .and_then(|flip| flip.get(axis))
            .and_then(Value::as_f64)
            == Some(-1.0)
    };
    (on(X), on(Y))
}

/// The four numbers of `cropTransform`; the whole picture when it is missing
/// or invalid (main.a33651a4.js 109267-109273, 209110-209116).
fn crop_of(fields: &Value) -> [f64; 4] {
    fields
        .get(CROP_TRANSFORM)
        .and_then(Value::as_array)
        .filter(|values| (4..=5).contains(&values.len()))
        .and_then(|values| {
            values
                .iter()
                .take(4)
                .map(Value::as_f64)
                .collect::<Option<Vec<f64>>>()
        })
        .and_then(|numbers| <[f64; 4]>::try_from(numbers).ok())
        .filter(|crop| crop[2] > 0.0 && crop[3] > 0.0)
        .unwrap_or([0.0, 0.0, 1.0, 1.0])
}

/// The whole picture as laid under the box `now` (`getCropBackgroundRect`,
/// main.a33651a4.js 109315-109330).
fn background(fields: &Value, now: Rectangle) -> Rectangle {
    let [left, top, shown_width, shown_height] = crop_of(fields);
    let (width, height) = (now.width / shown_width, now.height / shown_height);
    let (flip_x, flip_y) = mirrored(fields);
    let place = |flipped: bool, (start, length, share, whole): (f64, f64, f64, f64)| {
        pick(
            flipped,
            start + length + whole * share - whole,
            start - whole * share,
        )
    };
    Rectangle::new(
        place(flip_x, (now.x, now.width, left, width)),
        place(flip_y, (now.y, now.height, top, height)),
        width,
        height,
    )
}

/// The largest box of the ratio `wide`:`high` centred in `now`
/// (main.a33651a4.js 171980-171989).
fn largest(now: Rectangle, (wide, high): (f64, f64)) -> Rectangle {
    let width = now.height * wide / high;
    let height = now.width * high / wide;
    pick(
        now.width * high > now.height * wide,
        Rectangle::new(now.x + (now.width - width) / 2.0, now.y, width, now.height),
        Rectangle::new(
            now.x,
            now.y + (now.height - height) / 2.0,
            now.width,
            height,
        ),
    )
}

fn intersection(first: Rectangle, second: Rectangle) -> Option<Rectangle> {
    let (x, y) = (first.x.max(second.x), first.y.max(second.y));
    let width = first.right().min(second.right()) - x;
    let height = first.bottom().min(second.bottom()) - y;
    (width > 0.0 && height > 0.0).then(|| Rectangle::new(x, y, width, height))
}

/// `C.wP`: the places `cropTransform` is rounded to (main.a33651a4.js 216971).
fn round_places(value: f64) -> f64 {
    format!("{value}e6")
        .parse::<f64>()
        .ok()
        .and_then(|scaled| format!("{}e-6", scaled.round()).parse().ok())
        .unwrap_or(value)
}

/// BUNDLE: a mask cuts the largest box of its ratio out of the box now, about
/// its centre; `none` lays the whole picture there. The corner moves so that
/// the box keeps its centre as drawn, turned about the corner by `rotation`
/// (main.a33651a4.js 171921-171966, `x.w` 209199-209208).
fn crop_changes(mask: CropMask, fields: &Value) -> Vec<(&'static str, Value)> {
    rectangle_of(fields).map_or_else(Vec::new, |now| laid(mask, fields, now))
}

/// The changes that lay `mask` over the box `now`.
fn laid(mask: CropMask, fields: &Value, now: Rectangle) -> Vec<(&'static str, Value)> {
    let whole = background(fields, now);
    let target = mask.ratio().map_or(whole, |ratio| largest(now, ratio));
    let (flip_x, flip_y) = mirrored(fields);
    let share = |flipped: bool, (start, length, at, extent): (f64, f64, f64, f64)| {
        pick(
            flipped,
            (start + length - at - extent) / length,
            (at - start) / length,
        )
    };
    let seen = intersection(whole, target).map_or([0.0, 0.0, 1.0, 1.0], |part| {
        [
            share(flip_x, (whole.x, whole.width, part.x, part.width)),
            share(flip_y, (whole.y, whole.height, part.y, part.height)),
            part.width / whole.width,
            part.height / whole.height,
        ]
    });
    let crop = mask.ratio().map_or_else(
        || json!([0, 0, 1, 1, null]),
        |_| {
            let [x, y, width, height] = seen.map(round_places);
            json!([x, y, width, height, mask.stored()])
        },
    );
    let (sin, cos) = fields
        .get(ROTATION)
        .and_then(Value::as_f64)
        .unwrap_or_default()
        .to_radians()
        .sin_cos();
    let (along, down) = (
        (now.width - target.width) / 2.0,
        (now.height - target.height) / 2.0,
    );
    vec![
        (
            POSITION,
            json!({ (X): now.x + along * cos - down * sin, (Y): now.y + along * sin + down * cos }),
        ),
        (WIDTH, json!(target.width)),
        (HEIGHT, json!(target.height)),
        (CROP_TRANSFORM, crop),
    ]
}
