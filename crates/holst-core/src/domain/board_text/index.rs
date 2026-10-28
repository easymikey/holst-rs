//! A board's objects as a compact, versioned index: header, gzip, postcard.

#[cfg(test)] mod tests;

use std::{borrow::Cow, collections::HashMap};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use yrs::{Doc, ReadTxn as _, Transact as _, updates::encoder::Encode as _};

use crate::{
    domain::{
        board::{ArrowEnds, DrawnArrowEnds, Object, ObjectKind, StoredFile},
        board_file::{compression, compression::Compression},
        geometry::Point,
        tree::name_index::build::{canonical_uuid, saturating_u32, widen},
    },
    error::{Error, Result, local_data::LocalDataError},
};

pub const MAGIC: [u8; 4] = *b"HBIX";

pub const VERSION: u16 = 4;

/// What an index was built from: the board and its document's state vector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStamp {
    pub board: String,
    pub state_vector: Vec<u8>,
}

type StringNumber = u32;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct IndexedArrow {
    object: u32,
    start: Option<StringNumber>,
    end: Option<StringNumber>,
    drawn: Option<[IndexNumber; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct IndexedFile {
    object: u32,
    name: StringNumber,
    original: Option<StringNumber>,
}

const MANTISSA_BITS: u32 = 52;
const EXPONENT_BIAS: u32 = 1023;
const EXPONENT_MASK: u64 = 0x7ff;
const IMPLICIT_ONE: u64 = 1 << MANTISSA_BITS;

/// The `i32` a rounded `value` equals, read from its bits (no cast, no text);
/// `None` for NaN, infinity or a magnitude beyond `i32`.
fn whole_i32(value: f64) -> Option<i32> {
    let bits = value.to_bits();
    let exponent = u32::try_from((bits >> MANTISSA_BITS) & EXPONENT_MASK).ok()?;
    let magnitude = match exponent.checked_sub(EXPONENT_BIAS) {
        // Below one a rounded value is zero.
        None => 0,
        Some(power) => ((bits & (IMPLICIT_ONE - 1)) | IMPLICIT_ONE)
            .checked_shr(MANTISSA_BITS.checked_sub(power)?)?,
    };
    let magnitude = i32::try_from(magnitude).ok()?;
    if bits >> 63 == 0 {
        Some(magnitude)
    } else {
        magnitude.checked_neg()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum IndexNumber {
    Hundredths(i32),
    Float(f64),
}

impl IndexNumber {
    fn of(value: f64) -> Self {
        let rounded = (value * 100.0).round();
        let hundredths = if rounded.abs() < f64::from(i32::MAX) {
            whole_i32(rounded)
        } else {
            None
        };
        hundredths
            .filter(|&hundredths| {
                (f64::from(hundredths) / 100.0).to_bits() == value.to_bits()
            })
            .map_or(IndexNumber::Float(value), IndexNumber::Hundredths)
    }

    fn value(self) -> f64 {
        match self {
            IndexNumber::Hundredths(hundredths) => f64::from(hundredths) / 100.0,
            IndexNumber::Float(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
struct IndexBody {
    uuids: Uuids,
    names: String,
    name_ends: Vec<u32>,
    ids: Vec<StringNumber>,
    kinds: Vec<StringNumber>,
    parents: Vec<Option<StringNumber>>,
    xs: Vec<IndexNumber>,
    ys: Vec<IndexNumber>,
    widths: Vec<Option<IndexNumber>>,
    heights: Vec<Option<IndexNumber>>,
    z_indexes: Vec<f64>,
    colors: Vec<Option<StringNumber>>,
    authors: Vec<Option<StringNumber>>,
    locked: Vec<bool>,
    arrows: Vec<IndexedArrow>,
    files: Vec<IndexedFile>,
    text_ends: Vec<u32>,
    texts: String,
}

const UUID_BYTES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Uuids(Vec<u8>);

impl Uuids {
    const fn len(&self) -> usize {
        self.0.len() / UUID_BYTES
    }

    fn get(&self, at: usize) -> Option<u128> {
        let start = at.checked_mul(UUID_BYTES)?;
        let bytes = self.0.get(start..start.checked_add(UUID_BYTES)?)?;
        <[u8; UUID_BYTES]>::try_from(bytes)
            .ok()
            .map(u128::from_be_bytes)
    }
}

impl Serialize for Uuids {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Uuids {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        <&'de [u8]>::deserialize(deserializer).map(|bytes| Uuids(bytes.to_vec()))
    }
}

/// The compact index of one board: what `board objects` lists and search reads.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardIndex {
    stamp: IndexStamp,
    body: IndexBody,
}

#[derive(Debug, Default)]
struct StringTables<'a> {
    uuid_numbers: HashMap<u128, u32>,
    name_numbers: HashMap<&'a str, u32>,
    body: IndexBody,
}

impl<'a> StringTables<'a> {
    fn number(&mut self, name: &'a str) -> StringNumber {
        canonical_uuid(name).map_or_else(
            || {
                let next = saturating_u32(self.body.name_ends.len());
                let names = &mut self.body.names;
                let ends = &mut self.body.name_ends;
                let number = *self.name_numbers.entry(name).or_insert_with(|| {
                    names.push_str(name);
                    ends.push(saturating_u32(names.len()));
                    next
                });
                number * 2 + 1
            },
            |uuid| {
                let next = saturating_u32(self.body.uuids.len());
                let uuids = &mut self.body.uuids;
                let number = *self.uuid_numbers.entry(uuid).or_insert_with(|| {
                    uuids.0.extend(uuid.to_be_bytes());
                    next
                });
                number * 2
            },
        )
    }

    fn push(&mut self, at: usize, object: &'a Object) {
        let id = self.number(&object.id);
        let kind = self.number(object.kind.as_str());
        let parent = object.parent.as_deref().map(|parent| self.number(parent));
        let color = object.color.as_deref().map(|color| self.number(color));
        let author = object.author.as_deref().map(|author| self.number(author));
        let arrow = object.ends.as_ref().map(|ends| {
            let drawn = ends.drawn.map(|DrawnArrowEnds { start, end }| {
                [start.x(), start.y(), end.x(), end.y()].map(IndexNumber::of)
            });
            IndexedArrow {
                object: saturating_u32(at),
                start: ends.start.as_deref().map(|start| self.number(start)),
                end: ends.end.as_deref().map(|end| self.number(end)),
                drawn,
            }
        });
        let file = object.file.as_ref().map(|file| IndexedFile {
            object: saturating_u32(at),
            name: self.number(&file.name),
            original: file
                .original
                .as_deref()
                .map(|original| self.number(original)),
        });
        let body = &mut self.body;
        body.files.extend(file);
        body.ids.push(id);
        body.kinds.push(kind);
        body.parents.push(parent);
        body.xs.push(IndexNumber::of(object.x));
        body.ys.push(IndexNumber::of(object.y));
        body.widths.push(object.width.map(IndexNumber::of));
        body.heights.push(object.height.map(IndexNumber::of));
        body.z_indexes.push(object.z_index);
        body.colors.push(color);
        body.authors.push(author);
        body.locked.push(object.locked);
        body.arrows.extend(arrow);
        body.texts.push_str(&object.text);
        body.text_ends.push(saturating_u32(body.texts.len()));
    }
}

impl BoardIndex {
    /// The index of a board's objects, stamped with its id and state vector.
    #[must_use]
    pub fn from_objects(
        objects: &[Object],
        board: &str,
        state_vector: Vec<u8>,
    ) -> Self {
        let empty = StringTables {
            body: IndexBody {
                texts: String::with_capacity(
                    objects.iter().map(|object| object.text.len()).sum(),
                ),
                ..IndexBody::default()
            },
            ..StringTables::default()
        };
        let tables =
            objects
                .iter()
                .enumerate()
                .fold(empty, |mut tables, (at, object)| {
                    tables.push(at, object);
                    tables
                });
        Self {
            stamp: IndexStamp {
                board: board.to_owned(),
                state_vector,
            },
            body: tables.body,
        }
    }

    /// The index of a loaded board document, stamped with its state vector.
    #[must_use]
    pub fn from_document(doc: &Doc, board: &str) -> Self {
        let state_vector = doc.transact().state_vector().encode_v1();
        Self::from_objects(&crate::domain::board::objects(doc), board, state_vector)
    }

    /// The board and the document state the index was built from.
    #[must_use]
    pub const fn stamp(&self) -> &IndexStamp {
        &self.stamp
    }

    /// The board id.
    #[must_use]
    pub fn board(&self) -> &str {
        &self.stamp.board
    }

    /// How many objects it holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.body.ids.len()
    }

    /// Whether the board has no objects.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.body.ids.is_empty()
    }

    /// Every object as the listing shows it; `fields` stay empty.
    #[must_use]
    pub fn objects(&self) -> Vec<Object> {
        (0..self.len()).filter_map(|at| self.object(at)).collect()
    }

    /// Object number `at` as the listing shows it.
    #[must_use]
    pub fn object(&self, at: usize) -> Option<Object> {
        let ends = self
            .body
            .arrows
            .binary_search_by_key(&at, |arrow| widen(arrow.object))
            .ok()
            .and_then(|found| self.body.arrows.get(found));
        let file = self
            .body
            .files
            .binary_search_by_key(&at, |file| widen(file.object))
            .ok()
            .and_then(|found| self.body.files.get(found));
        self.object_with(at, ends.copied(), file.copied())
    }

    fn object_with(
        &self,
        at: usize,
        arrow: Option<IndexedArrow>,
        file: Option<IndexedFile>,
    ) -> Option<Object> {
        let body = &self.body;
        let name = |number: Option<StringNumber>| {
            number.map(|number| self.name(number).into_owned())
        };
        let drawn = |[start_x, start_y, end_x, end_y]: [IndexNumber; 4]| {
            let point =
                |x: IndexNumber, y: IndexNumber| Point::new(x.value(), y.value()).ok();
            point(start_x, start_y)
                .zip(point(end_x, end_y))
                .map(|(start, end)| DrawnArrowEnds { start, end })
        };
        Some(Object {
            id: self.name(*body.ids.get(at)?).into_owned(),
            kind: ObjectKind::of(&self.name(*body.kinds.get(at)?)),
            parent: name(*body.parents.get(at)?),
            x: body.xs.get(at)?.value(),
            y: body.ys.get(at)?.value(),
            width: body.widths.get(at)?.map(IndexNumber::value),
            height: body.heights.get(at)?.map(IndexNumber::value),
            z_index: *body.z_indexes.get(at)?,
            text: self.text(at).to_owned(),
            color: name(*body.colors.get(at)?),
            look: None,
            ends: arrow.map(
                |IndexedArrow {
                     start,
                     end,
                     drawn: points,
                     ..
                 }| {
                    ArrowEnds {
                        start: name(start),
                        end: name(end),
                        drawn: points.and_then(drawn),
                    }
                },
            ),
            author: name(*body.authors.get(at)?),
            locked: *body.locked.get(at)?,
            file: file.map(
                |IndexedFile {
                     name: stored,
                     original,
                     ..
                 }| StoredFile {
                    name: self.name(stored).into_owned(),
                    original: name(original),
                },
            ),
            fields: Value::Null,
        })
    }

    /// The id of object number `at`; empty past the end.
    #[must_use]
    pub fn id(&self, at: usize) -> Cow<'_, str> {
        self.body
            .ids
            .get(at)
            .map_or(Cow::Borrowed(""), |&id| self.name(id))
    }

    /// The type name of object number `at`, as [`ObjectKind::as_str`] gives it.
    #[must_use]
    pub fn kind_name(&self, at: usize) -> Cow<'_, str> {
        self.body
            .kinds
            .get(at)
            .map_or(Cow::Borrowed(""), |&kind| self.name(kind))
    }

    /// The kind of every object, in order.
    pub fn kinds(&self) -> impl Iterator<Item = ObjectKind> + Clone {
        (0..self.body.kinds.len()).map(|at| ObjectKind::of(&self.kind_name(at)))
    }

    /// The text of object number `at`; empty past the end.
    #[must_use]
    pub fn text(&self, at: usize) -> &str {
        piece(&self.body.texts, &self.body.text_ends, at)
    }

    fn name(&self, number: StringNumber) -> Cow<'_, str> {
        let at = widen(number / 2);
        match number % 2 {
            0 => self.body.uuids.get(at).map_or(Cow::Borrowed(""), |uuid| {
                Cow::Owned(uuid::Uuid::from_u128(uuid).hyphenated().to_string())
            }),
            _ => Cow::Borrowed(piece(&self.body.names, &self.body.name_ends, at)),
        }
    }

    /// `MAGIC`, the version, the stamp in postcard, then the objects in gzip.
    ///
    /// # Errors
    /// [`LocalDataError::IndexEncode`] or [`LocalDataError::Pack`]; both write
    /// to memory, so they should not happen.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let stamp =
            postcard::to_allocvec(&self.stamp).map_err(LocalDataError::IndexEncode)?;
        let body =
            postcard::to_allocvec(&self.body).map_err(LocalDataError::IndexEncode)?;
        let packed = compression::compressed(&body, Compression::Gzip)?;
        Ok([&MAGIC[..], &VERSION.to_le_bytes(), &stamp, &packed].concat())
    }

    /// Only the stamp: enough to tell whether an index is current.
    ///
    /// # Errors
    /// [`LocalDataError::NotBoardIndex`], [`LocalDataError::IndexVersion`] or
    /// [`LocalDataError::IndexCorrupt`].
    pub fn peek(bytes: &[u8]) -> Result<IndexStamp> {
        header(bytes).map(|(stamp, _)| stamp)
    }

    /// The index back from [`BoardIndex::encode`]; other bytes fail typed.
    ///
    /// # Errors
    /// As [`BoardIndex::peek`]; a broken gzip part is
    /// [`LocalDataError::Unpack`].
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (stamp, packed) = header(bytes)?;
        (if compression::is_compressed(packed) {
            Ok(())
        } else {
            Err(LocalDataError::IndexCorrupt { error: None })
        })?;
        let body: IndexBody = postcard::from_bytes(&compression::uncompressed(packed)?)
            .map_err(|err| LocalDataError::IndexCorrupt { error: Some(err) })?;
        if body.adds_up() {
            Ok(Self { stamp, body })
        } else {
            Err(Error::LocalData(LocalDataError::IndexCorrupt {
                error: None,
            }))
        }
    }
}

fn piece<'a>(arena: &'a str, ends: &[u32], at: usize) -> &'a str {
    let start = at
        .checked_sub(1)
        .and_then(|before| ends.get(before))
        .map_or(0, |&end| widen(end));
    ends.get(at)
        .and_then(|&end| arena.get(start..widen(end)))
        .unwrap_or_default()
}

fn header(bytes: &[u8]) -> Result<(IndexStamp, &[u8])> {
    let rest = bytes
        .strip_prefix(&MAGIC)
        .ok_or(LocalDataError::NotBoardIndex)?;
    let (version, rest) = rest
        .split_first_chunk::<2>()
        .ok_or(LocalDataError::IndexCorrupt { error: None })?;
    let found = u16::from_le_bytes(*version);
    (if found == VERSION {
        Ok(())
    } else {
        Err(LocalDataError::IndexVersion {
            found,
            supported: VERSION,
        })
    })?;
    Ok(postcard::take_from_bytes::<IndexStamp>(rest)
        .map_err(|err| LocalDataError::IndexCorrupt { error: Some(err) })?)
}

impl IndexBody {
    fn adds_up(&self) -> bool {
        let count = self.ids.len();
        let uuids = self.uuids.len();
        let whole_uuids = self.uuids.0.len().is_multiple_of(UUID_BYTES);
        let names = self.name_ends.len();
        let known = |number: Option<StringNumber>| {
            number.is_none_or(|number| match number % 2 {
                0 => widen(number / 2) < uuids,
                _ => widen(number / 2) < names,
            })
        };
        let columns = [
            self.kinds.len(),
            self.parents.len(),
            self.xs.len(),
            self.ys.len(),
            self.widths.len(),
            self.heights.len(),
            self.z_indexes.len(),
            self.colors.len(),
            self.authors.len(),
            self.locked.len(),
            self.text_ends.len(),
        ];
        let arrow_objects = self.arrows.iter().map(|arrow| arrow.object);
        let file_objects = self.files.iter().map(|file| file.object);
        let references = self
            .ids
            .iter()
            .chain(&self.kinds)
            .map(|&number| Some(number))
            .chain(
                self.parents
                    .iter()
                    .chain(&self.colors)
                    .chain(&self.authors)
                    .copied(),
            )
            .chain(
                self.arrows
                    .iter()
                    .flat_map(|&IndexedArrow { start, end, .. }| [start, end]),
            )
            .chain(self.files.iter().flat_map(
                |&IndexedFile { name, original, .. }| [Some(name), original],
            ));
        whole_uuids
            && columns.iter().all(|&len| len == count)
            && objects_fit(arrow_objects, count)
            && objects_fit(file_objects, count)
            && ends_fit(self.name_ends.iter().copied(), &self.names)
            && ends_fit(self.text_ends.iter().copied(), &self.texts)
            && references.into_iter().all(known)
    }
}

fn objects_fit(numbers: impl Iterator<Item = u32> + Clone, count: usize) -> bool {
    numbers
        .clone()
        .zip(numbers.clone().skip(1))
        .all(|(number, next)| number < next)
        && numbers.into_iter().all(|number| widen(number) < count)
}

fn ends_fit(ends: impl Iterator<Item = u32> + Clone, arena: &str) -> bool {
    ends.clone()
        .zip(ends.clone().skip(1))
        .all(|(end, next)| end <= next)
        && ends
            .into_iter()
            .all(|end| arena.is_char_boundary(widen(end)))
}
