//! A board's objects indexed for geometry: the box each object takes.

use std::{collections::HashMap, ptr, sync::OnceLock};

use itertools::Itertools;
use serde_json::Value;

use crate::{
    domain::{
        area::Area,
        board::{Object, ObjectKind},
        geometry::{Rectangle, StoredPoint, StoredSize},
        lookup::rules::{FRAME_ORDER, Reference},
        text::printable,
        text_layout,
    },
    error::{Error, Result, lookup, lookup::LookupError, picture::PictureError},
    protocol::document,
};

/// A board's objects indexed for geometry; each box is measured once.
#[derive(Debug)]
pub struct Scene<'a> {
    pub(super) objects: &'a [Object],
    pub(super) by_id: HashMap<&'a str, usize>,
    measured: Vec<OnceLock<Option<Rectangle>>>,
}

impl<'a> Scene<'a> {
    #[must_use]
    pub fn new(objects: &'a [Object]) -> Self {
        Self {
            objects,
            by_id: objects
                .iter()
                .enumerate()
                .map(|(slot, o)| (o.id.as_str(), slot))
                .collect(),
            measured: objects.iter().map(|_| OnceLock::new()).collect(),
        }
    }

    /// Every object.
    #[must_use]
    pub const fn objects(&self) -> &'a [Object] {
        self.objects
    }

    /// An object by its exact id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&'a Object> {
        self.by_id.get(id).and_then(|&slot| self.objects.get(slot))
    }

    /// The object's own box before rotation: size scaled by `textScale`;
    /// `None` for arrows, groups and broken or absurd numbers.
    #[must_use]
    pub fn rectangle_of(&self, object: &Object) -> Option<Rectangle> {
        let scale = text_scale(object);
        let sized = object
            .width
            .zip(object.height)
            .map(|(width, height)| StoredSize::new(width, height));
        let stored = || {
            sized.or_else(|| {
                let size = object.kind.default_size()?;
                Some(StoredSize::new(size.width(), size.height()))
            })
        };
        let fallback =
            |width, height| Some(sized.unwrap_or(StoredSize::new(width, height)));
        let size = match &object.kind {
            ObjectKind::Arrow | ObjectKind::Group => None,
            ObjectKind::Sticker | ObjectKind::StickerStack => stored()
                .map(|size| StoredSize::new(size.width * scale, size.height * scale)),
            ObjectKind::Text => Some(text_size(object)),
            ObjectKind::Document | ObjectKind::Shape | ObjectKind::Frame => stored(),
            ObjectKind::Table => Some(table_size(&object.fields)),
            ObjectKind::TableCell => self.cell_size(object),
            ObjectKind::Code => Some(StoredSize::new(
                object.width.unwrap_or(400.0),
                object.height.unwrap_or_else(|| code_height(&object.text)),
            )),
            ObjectKind::Drawing => drawing_rectangle(&object.fields).map(|rectangle| {
                StoredSize::new(rectangle.right(), rectangle.bottom())
            }),
            ObjectKind::Dice => fallback(240.0, 240.0),
            ObjectKind::Wheel => fallback(360.0, 360.0),
            ObjectKind::Flipcard => fallback(220.0, 320.0),
            ObjectKind::Task => fallback(300.0, 120.0),
            ObjectKind::Link => fallback(300.0, 220.0),
            ObjectKind::File => fallback(240.0, 64.0),
            ObjectKind::Image | ObjectKind::Stamp | ObjectKind::Other(_) => {
                fallback(160.0, 100.0)
            }
        }?;
        let size = if drawn_at_scale(&object.kind) {
            let by = drawn_scale(object);
            StoredSize::new(size.width * by, size.height * by)
        } else {
            size
        };
        let corner = if object.kind == ObjectKind::TableCell {
            self.cell_corner(object)
        } else {
            Some(StoredPoint::new(object.x, object.y))
        }?;
        Rectangle::new(corner.x, corner.y, size.width, size.height).sane()
    }

    fn cell_corner(&self, cell: &Object) -> Option<StoredPoint> {
        let table = self.get(cell.parent.as_deref()?)?;
        let TableCell { column, row } = cell_place(cell, table)?;
        let before = |prefix, default, side, limit: f64| {
            tracks(&table.fields, prefix, default, side)
                .iter()
                .filter(|track| track.index < limit)
                .map(|track| track.size)
                .sum::<f64>()
        };
        Some(StoredPoint::new(
            table.x
                + before(
                    document::COLUMN_PREFIX,
                    240.0,
                    document::WIDTH,
                    column.index,
                ),
            table.y + before(document::ROW_PREFIX, 60.0, document::HEIGHT, row.index),
        ))
    }

    fn cell_size(&self, cell: &Object) -> Option<StoredSize> {
        let table = self.get(cell.parent.as_deref()?)?;
        let TableCell { column, row } = cell_place(cell, table)?;
        Some(StoredSize::new(column.size, row.size))
    }

    /// Everything the object covers: its rotated box, a frame's label, an
    /// arrow's route; `None` for groups and for broken or absurd numbers.
    #[must_use]
    pub fn covered_by(&self, object: &Object) -> Option<Rectangle> {
        let cached = self
            .by_id
            .get(object.id.as_str())
            .and_then(|&slot| self.objects.get(slot).zip(self.measured.get(slot)))
            .filter(|(own, _)| ptr::eq(*own, object));
        match cached {
            Some((_, cell)) => *cell.get_or_init(|| self.measure(object)),
            None => self.measure(object),
        }
    }

    fn measure(&self, object: &Object) -> Option<Rectangle> {
        let covered = match object.kind {
            // Without its fields an arrow covers at least the span of its kept ends.
            ObjectKind::Arrow => self
                .route(object)
                .map(|route| route.points)
                .or_else(|| {
                    self.drawn_ends(object).map(|ends| {
                        vec![
                            StoredPoint::new(ends.start.x(), ends.start.y()),
                            StoredPoint::new(ends.end.x(), ends.end.y()),
                        ]
                    })
                })
                .and_then(Rectangle::around)
                .map(|rectangle| rectangle.padded(stroke_width(&object.fields) + 6.0)),
            ObjectKind::Frame => self.rectangle_of(object).map(|rectangle| {
                Rectangle::new(
                    rectangle.x,
                    rectangle.y - FRAME_TITLE_BAND,
                    rectangle.width,
                    rectangle.height + FRAME_TITLE_BAND,
                )
            }),
            ObjectKind::Sticker
            | ObjectKind::StickerStack
            | ObjectKind::Text
            | ObjectKind::Shape
            | ObjectKind::Document
            | ObjectKind::Image
            | ObjectKind::File
            | ObjectKind::Stamp
            | ObjectKind::Group
            | ObjectKind::Table
            | ObjectKind::TableCell
            | ObjectKind::Code
            | ObjectKind::Drawing
            | ObjectKind::Dice
            | ObjectKind::Wheel
            | ObjectKind::Flipcard
            | ObjectKind::Task
            | ObjectKind::Link
            | ObjectKind::Other(_) => self
                .rectangle_of(object)
                .map(|rectangle| rotated(rectangle, rotation(object))),
        };
        covered.and_then(Rectangle::sane)
    }
}

/// How many groups inside groups a selection follows.
const GROUP_DEPTH: usize = 16;

impl<'a> Scene<'a> {
    /// The rectangle a selection covers, with `padding` around it.
    ///
    /// # Errors
    /// [`crate::error::picture::PictureError::EmptyBoard`] or
    /// [`crate::error::picture::PictureError::EmptyArea`], or a
    /// [`LookupError`] of a frame or an object.
    pub fn area_rectangle(&self, area: &Area, padding: f64) -> Result<Rectangle> {
        let covered = match area {
            Area::Board => Rectangle::bounding(
                self.objects.iter().filter_map(|o| self.covered_by(o)),
            )
            .ok_or(Error::Picture(PictureError::EmptyBoard))?,
            Area::Rectangle(rectangle) => *rectangle,
            Area::Frame(reference) => {
                let frame = self.frame(reference)?;
                self.covered_by(frame).ok_or_else(|| {
                    Error::Lookup(frame_lookup(
                        reference,
                        lookup::LookupMiss::not_found(),
                    ))
                })?
            }
            Area::Objects(specs) => {
                let rects = specs
                    .iter()
                    .map(|reference| {
                        let id = crate::domain::lookup::rules::find_id(
                            self.by_id.keys().copied(),
                            reference,
                        )?;
                        Ok(self
                            .get(&id)
                            .and_then(|object| self.covered_with_members(object, 0)))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Rectangle::bounding(rects.into_iter().flatten()).ok_or_else(|| {
                    Error::Picture(PictureError::EmptyArea {
                        reference: specs.join(", "),
                    })
                })?
            }
        };
        Ok(covered.padded(padding))
    }

    /// What a named object covers: its own box, or for a group the box of its
    /// members; `None` for broken numbers and an empty group.
    fn covered_with_members(&self, object: &Object, depth: usize) -> Option<Rectangle> {
        let members = || {
            self.objects
                .iter()
                .filter(|member| member.parent.as_deref() == Some(object.id.as_str()))
                .filter_map(|member| self.covered_with_members(member, depth + 1))
        };
        if object.kind == ObjectKind::Group && depth < GROUP_DEPTH {
            Rectangle::bounding(members())
        } else {
            self.covered_by(object)
        }
    }

    /// What an object covers as the web client counts it: its own box turned
    /// about its corner, an arrow's route, the box of a group's members.
    // BUNDLE: main.a33651a4.js 168787-168789 (`getRectBounds`).
    pub(in crate::domain) fn bounds(
        &self,
        object: &Object,
        depth: usize,
    ) -> Option<Rectangle> {
        let members = || {
            self.objects
                .iter()
                .filter(|member| member.parent.as_deref() == Some(object.id.as_str()))
                .filter_map(|member| self.bounds(member, depth + 1))
        };
        let own = || {
            self.rectangle_of(object)
                .map(|own| rotated(own, rotation(object)))
                .and_then(Rectangle::sane)
        };
        match &object.kind {
            ObjectKind::Group => (depth < GROUP_DEPTH)
                .then(|| Rectangle::bounding(members()))
                .flatten(),
            ObjectKind::Arrow => self.covered_by(object),
            ObjectKind::Sticker
            | ObjectKind::StickerStack
            | ObjectKind::Text
            | ObjectKind::Shape
            | ObjectKind::Document
            | ObjectKind::Frame
            | ObjectKind::Image
            | ObjectKind::File
            | ObjectKind::Stamp
            | ObjectKind::Table
            | ObjectKind::TableCell
            | ObjectKind::Code
            | ObjectKind::Drawing
            | ObjectKind::Dice
            | ObjectKind::Wheel
            | ObjectKind::Flipcard
            | ObjectKind::Task
            | ObjectKind::Link
            | ObjectKind::Other(_) => own(),
        }
    }

    /// A frame by id or link, else title, 8+ hex id prefix, part of title.
    ///
    /// # Errors
    /// A [`LookupError`] of a frame: none, or several by title or prefix.
    pub fn frame(&self, reference: &str) -> Result<&'a Object> {
        let frames = self.objects.iter().filter(|o| o.kind == ObjectKind::Frame);
        let found: Vec<&Object> = crate::domain::lookup::rules::find_matches(
            frames,
            |frame| (frame.id.as_str(), frame.text.as_str()),
            &Reference::object(reference),
            &FRAME_ORDER,
        )
        .into_iter()
        .sorted_by(|a, b| a.text.cmp(&b.text).then_with(|| a.id.cmp(&b.id)))
        .dedup_by(|a, b| a.id == b.id)
        .collect();
        match found.as_slice() {
            &[frame] => Ok(frame),
            [] => Err(Error::Lookup(frame_lookup(
                reference,
                lookup::LookupMiss::not_found(),
            ))),
            many => Err(Error::Lookup(frame_lookup(
                reference,
                lookup::LookupMiss::ambiguous(lookup::Candidates::capped(
                    many.iter().map(|o| lookup::Candidate {
                        name: printable(&o.text).into_owned(),
                        id: Some(printable(&o.id).into_owned()),
                        ..lookup::Candidate::default()
                    }),
                    5,
                )),
            ))),
        }
    }

    /// The object a link to `rectangle` should open: the smallest frame holding
    /// it, else the object nearest its center; the web client has no link to
    /// a bare area.
    #[must_use]
    pub fn rectangle_link_target(&self, rectangle: &Rectangle) -> Option<&'a Object> {
        let size = |covered: &Rectangle| covered.width * covered.height;
        let frame = self
            .objects
            .iter()
            .filter(|o| o.kind == ObjectKind::Frame)
            .filter_map(|o| self.rectangle_of(o).map(|covered| (o, covered)))
            .filter(|(_, covered)| covered.contains(rectangle))
            .min_by(|(_, a), (_, b)| size(a).total_cmp(&size(b)));
        let middle = rectangle.centre();
        let nearest = || {
            self.within(rectangle)
                .into_iter()
                .filter(|o| o.kind != ObjectKind::Arrow)
                .filter_map(|o| self.covered_by(o).map(|covered| (o, covered.centre())))
                .min_by(|(_, a), (_, b)| {
                    a.distance_to(middle).total_cmp(&b.distance_to(middle))
                })
        };
        frame
            .map(|(object, _)| object)
            .or_else(|| nearest().map(|(object, _)| object))
    }

    /// The object a link to a picture or listing of `area` over `rectangle`
    /// opens: the selected frame itself, else the
    /// [`Scene::rectangle_link_target`] of the area.
    #[must_use]
    pub fn link_target(
        &self,
        area: &Area,
        rectangle: &Rectangle,
    ) -> Option<&'a Object> {
        let chosen = match area {
            Area::Frame(reference) => self.frame(reference).ok(),
            Area::Board | Area::Rectangle(_) | Area::Objects(_) => None,
        };
        chosen.or_else(|| self.rectangle_link_target(rectangle))
    }

    /// The objects that show inside `rectangle`, in reading order.
    #[must_use]
    pub fn within(&self, rectangle: &Rectangle) -> Vec<&'a Object> {
        self.objects
            .iter()
            .filter(|object| {
                self.covered_by(object)
                    .is_some_and(|bounds| bounds.intersects(rectangle))
            })
            .collect()
    }
}

fn frame_lookup(reference: &str, miss: lookup::LookupMiss) -> LookupError {
    LookupError {
        kind: lookup::LookupKind::Frame,
        reference: reference.to_owned(),
        miss,
    }
}

/// `textScale` of an object; 1 unless set.
#[must_use]
pub fn text_scale(object: &Object) -> f64 {
    number(&object.fields, document::TEXT_SCALE)
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0)
}

/// Whether the kind is drawn at its `scale` (a drawing's points are scaled
/// on their own).
#[must_use]
pub fn drawn_at_scale(kind: &ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Code
            | ObjectKind::Dice
            | ObjectKind::Wheel
            | ObjectKind::Flipcard
            | ObjectKind::Task
            | ObjectKind::Link
            | ObjectKind::File
    )
}

/// `scale` of an object; unset, a die is drawn at 0.5 and a wheel at 1.5
/// (main.a33651a4.js 169136, 200219), anything else at 1.
#[must_use]
pub fn drawn_scale(object: &Object) -> f64 {
    let unset = [(ObjectKind::Dice, 0.5), (ObjectKind::Wheel, 1.5)]
        .into_iter()
        .find_map(|(kind, scale)| (kind == object.kind).then_some(scale))
        .unwrap_or(1.0);
    number(&object.fields, document::SCALE)
        .filter(|scale| *scale > 0.0)
        .unwrap_or(unset)
}

/// Rotation in degrees, clockwise about the stored position.
#[must_use]
pub fn rotation(object: &Object) -> f64 {
    object.rotation
}

/// The `strokeWidth` field of an arrow or a shape's outline, 2 unless set.
#[must_use]
pub fn stroke_width(fields: &Value) -> f64 {
    number(fields, document::STROKE_WIDTH).unwrap_or(2.0)
}

/// A number field.
#[must_use]
pub fn number(fields: &Value, name: &str) -> Option<f64> {
    fields
        .get(name)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

/// The line height of free text: 150% rounded to whole pixels.
#[must_use]
pub fn line_height(font_size: f64) -> f64 {
    (1.5 * font_size).round().max(1.0)
}

/// The wrap width of free text before `textScale`.
#[must_use]
pub fn free_text_wrap(object: &Object) -> f64 {
    let fixed = object
        .fields
        .get(document::FIXED_WIDTH)
        .and_then(Value::as_bool)
        .unwrap_or_default();
    object.width.filter(|_| fixed).unwrap_or(FREE_TEXT_WRAP)
}

fn text_size(object: &Object) -> StoredSize {
    let scale = text_scale(object);
    let wrapped =
        text_layout::wrap(&object.text, free_text_wrap(object), FREE_TEXT_FONT_SIZE);
    let widest = wrapped
        .lines
        .iter()
        .map(|line| text_layout::width(line, FREE_TEXT_FONT_SIZE))
        .fold(0.0, f64::max);
    let width = object
        .width
        .filter(|_| free_text_wrap(object) < FREE_TEXT_WRAP)
        .unwrap_or(widest.max(FREE_TEXT_FONT_SIZE));
    let height =
        text_layout::count(wrapped.lines.len()) * line_height(FREE_TEXT_FONT_SIZE);
    StoredSize::new(width * scale, height * scale)
}

fn code_height(text: &str) -> f64 {
    text_layout::count(text_layout::paragraphs(text).count()) * 21.0 + 24.0
}

/// One row or column of a table: its place in the order and its length.
#[derive(Debug, Clone, Copy)]
struct Track {
    index: f64,
    size: f64,
}

/// The column and the row a table cell sits in.
struct TableCell {
    column: Track,
    row: Track,
}

fn tracks(fields: &Value, prefix: &str, default: f64, side: &str) -> Vec<Track> {
    fields
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, track)| {
            key.strip_prefix(prefix)?;
            Some(Track {
                index: number(track, document::INDEX)?,
                size: number(track, side).unwrap_or(default),
            })
        })
        .collect()
}

pub(crate) fn table_size(fields: &Value) -> StoredSize {
    let sum = |prefix, default, side| {
        tracks(fields, prefix, default, side)
            .iter()
            .map(|track| track.size)
            .sum::<f64>()
    };
    StoredSize::new(
        sum(document::COLUMN_PREFIX, 240.0, document::WIDTH),
        sum(document::ROW_PREFIX, 60.0, document::HEIGHT),
    )
}

fn cell_place(cell: &Object, table: &Object) -> Option<TableCell> {
    let track = |id_field: &str, prefix: &str, default: f64, side: &str| {
        let id = cell.fields.get(id_field).and_then(Value::as_str)?;
        let track = table.fields.get(format!("{prefix}{id}"))?;
        Some(Track {
            index: number(track, document::INDEX)?,
            size: number(track, side).unwrap_or(default),
        })
    };
    Some(TableCell {
        column: track(
            document::COLUMN_ID,
            document::COLUMN_PREFIX,
            240.0,
            document::WIDTH,
        )?,
        row: track(
            document::ROW_ID,
            document::ROW_PREFIX,
            60.0,
            document::HEIGHT,
        )?,
    })
}

/// The numbers of a drawing's stored path, read from a list or from path data.
fn path_numbers(path: &str) -> Vec<f64> {
    path.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
        .filter_map(|part| part.parse::<f64>().ok())
        .collect()
}

/// The numbers of a stored path that is only a list of them (`x,y,x,y,…`), not
/// path data.
pub(crate) fn number_list(path: &str) -> Option<Vec<f64>> {
    path.chars()
        .all(|c| {
            c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(c, '.' | '-' | 'e' | ',')
        })
        .then(|| path_numbers(path))
}

fn drawing_rectangle(fields: &Value) -> Option<Rectangle> {
    let scale = number(fields, document::SCALE).unwrap_or(1.0);
    let numbers: Vec<f64> = match fields.get(document::PATH)? {
        Value::String(path) => path_numbers(path),
        Value::Array(values) => values.iter().filter_map(Value::as_f64).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
            Vec::new()
        }
    };
    let points = numbers.chunks_exact(2).filter_map(|pair| match pair {
        [x, y] => Some(StoredPoint::new(x * scale, y * scale)),
        _ => None,
    });
    Rectangle::around(points).map(|rectangle| {
        Rectangle::new(
            0.0,
            0.0,
            rectangle.right().max(1.0),
            rectangle.bottom().max(1.0),
        )
    })
}

/// The box around `rectangle` turned by `degrees` about its top-left corner.
#[must_use]
pub fn rotated(rectangle: Rectangle, degrees: f64) -> Rectangle {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let turn = |corner: StoredPoint| {
        StoredPoint::new(
            rectangle.x + corner.x * cos - corner.y * sin,
            rectangle.y + corner.x * sin + corner.y * cos,
        )
    };
    let corners = [
        StoredPoint::new(0.0, 0.0),
        StoredPoint::new(rectangle.width, 0.0),
        StoredPoint::new(rectangle.width, rectangle.height),
        StoredPoint::new(0.0, rectangle.height),
    ];
    if degrees == 0.0 {
        rectangle
    } else {
        Rectangle::around(corners.map(turn)).unwrap_or(rectangle)
    }
}

pub const FRAME_TITLE_BAND: f64 = 31.0;

pub const FREE_TEXT_WRAP: f64 = 600.0;

pub const FREE_TEXT_FONT_SIZE: f64 = 14.0;
