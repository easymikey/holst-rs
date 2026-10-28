#![expect(
    clippy::panic_in_result_fn,
    reason = "assertions are the point of a test"
)]

use proptest::{
    collection::vec,
    option,
    prelude::{Strategy, any, prop_assert_eq, prop_oneof, proptest},
};
use serde_json::Value;

use crate::{
    domain::{
        board::{ArrowEnds, DrawnArrowEnds, Object, ObjectKind, StoredFile},
        board_text::index::{BoardIndex, IndexBody, IndexStamp, MAGIC, VERSION},
        geometry::Point,
    },
    error::{Error, Result, local_data::LocalDataError},
    laws::config,
};

fn object(id: &str, kind: ObjectKind, text: &str) -> Object {
    Object {
        id: id.to_owned(),
        kind,
        parent: Some("frame-1".to_owned()),
        x: 1.5,
        y: -2.25,
        width: Some(192.0),
        height: None,
        z_index: 7.0,
        text: text.to_owned(),
        color: Some("yellow4".to_owned()),
        ends: None,
        author: Some("author-1".to_owned()),
        file: None,
        fields: Value::Null,
    }
}

#[test]
fn encode_then_decode_gives_the_same_objects() -> Result<()> {
    let arrow = Object {
        ends: Some(ArrowEnds {
            start: Some("a".into()),
            end: None,
            drawn: Some(DrawnArrowEnds {
                start: Point::new(96.5, -0.25)?,
                end: Point::new(1e-3, 400.0)?,
            }),
        }),
        ..object("arrow-1", ObjectKind::Arrow, "")
    };
    let image = Object {
        file: Some(StoredFile {
            name: "00000000-0000-4000-8000-0000000000a9.png".into(),
            original: Some("кот.png".into()),
        }),
        ..object("image-1", ObjectKind::Image, "")
    };
    let objects = vec![
        object("a", ObjectKind::Sticker, "Ёжик в тумане"),
        object("b", ObjectKind::TableCell, "42"),
        arrow,
        image,
    ];
    let index = BoardIndex::from_objects(&objects, "board-1", vec![1, 2, 3]);
    let decoded = BoardIndex::decode(&index.encode()?)?;
    assert_eq!(decoded, index);
    assert_eq!(decoded.objects(), objects);
    assert_eq!(
        BoardIndex::peek(&index.encode()?)?,
        IndexStamp {
            board: "board-1".into(),
            state_vector: vec![1, 2, 3]
        }
    );
    Ok(())
}

type Expected = fn(&Error) -> bool;

#[test]
fn garbage_and_other_versions_are_typed_errors() -> Result<()> {
    let encoded = BoardIndex::from_objects(&[], "b", Vec::new()).encode()?;
    let future = [
        &MAGIC[..],
        &9u16.to_le_bytes(),
        encoded.get(6..).unwrap_or_default(),
    ]
    .concat();
    let cut = encoded.get(..encoded.len() - 4).unwrap_or_default();
    let cases: [(&[u8], Expected); 5] = [
        (b"", |err| {
            matches!(err, Error::LocalData(LocalDataError::NotBoardIndex))
        }),
        (b"PK\x03\x04", |err| {
            matches!(err, Error::LocalData(LocalDataError::NotBoardIndex))
        }),
        (&future, |err| {
            matches!(
                err,
                Error::LocalData(LocalDataError::IndexVersion {
                    found: 9,
                    supported: VERSION
                })
            )
        }),
        (b"HBIX\x01", |err| {
            matches!(err, Error::LocalData(LocalDataError::IndexCorrupt { .. }))
        }),
        (cut, |err| {
            matches!(err, Error::LocalData(LocalDataError::Unpack(_)))
        }),
    ];
    let wrong: Vec<_> = cases
        .iter()
        .map(|(bytes, expected)| (bytes, BoardIndex::decode(bytes).err(), expected))
        .filter(|(_, err, expected)| !err.as_ref().is_some_and(expected))
        .map(|(bytes, err, _)| format!("{bytes:?}: {err:?}"))
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
    Ok(())
}

#[test]
fn numbers_outside_the_tables_are_refused() -> Result<()> {
    let whole = BoardIndex::from_objects(
        &[object("a", ObjectKind::Text, "x")],
        "b",
        Vec::new(),
    );
    let index = BoardIndex {
        body: IndexBody {
            parents: vec![Some(99)],
            ..whole.body
        },
        ..whole
    };
    let err = BoardIndex::decode(&index.encode()?).err();
    assert!(
        matches!(
            err,
            Some(Error::LocalData(LocalDataError::IndexCorrupt {
                error: None
            }))
        ),
        "{err:?}"
    );
    Ok(())
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof![any::<String>(), "[а-яёА-ЯЁa-z \\n\u{301}]{0,40}"]
}

fn id() -> impl Strategy<Value = String> {
    let uuid = any::<u128>()
        .prop_map(|bits| uuid::Uuid::from_u128(bits).hyphenated().to_string());
    prop_oneof![
        uuid.clone(),
        uuid.prop_map(|id| id.to_ascii_uppercase()),
        proptest::prelude::Just(uuid::Uuid::nil().hyphenated().to_string()),
        "[a-f0-9-]{1,36}",
    ]
}

fn any_object() -> impl Strategy<Value = Object> {
    let kind = prop_oneof![
        "sticker|frame|arrow|simple-text|shape|card|sticker-stack"
            .prop_map(|n| ObjectKind::of(&n)),
        "[a-z-]{1,12}".prop_map(|n| ObjectKind::of(&n)),
    ];
    let ends = option::of((option::of(id()), option::of(id())));
    let file = option::of((text(), option::of(text())))
        .prop_map(|file| file.map(|(name, original)| StoredFile { name, original }));
    (
        (id(), kind, option::of(id()), file),
        (
            any::<f64>(),
            any::<f64>(),
            option::of(any::<f64>()),
            option::of(any::<f64>()),
        ),
        (
            any::<f64>(),
            text(),
            option::of("[a-z0-9]{1,8}"),
            ends,
            option::of(any::<String>()),
        ),
    )
        .prop_map(
            |(
                (id, kind, parent, file),
                (x, y, width, height),
                (z_index, text, color, ends, author),
            )| {
                Object {
                    id,
                    kind,
                    parent,
                    x,
                    y,
                    width,
                    height,
                    z_index,
                    text,
                    color,
                    ends: ends.map(|(start, end)| ArrowEnds {
                        start,
                        end,
                        drawn: None,
                    }),
                    author,
                    file,
                    fields: Value::Null,
                }
            },
        )
}

proptest! {
    #![proptest_config(config())]

    /// Decoding gives back every object bit for bit, NaN coordinates too.
    #[test]
    fn encode_decode_round_trip(
        objects in vec(any_object(), 0..24),
        state_vector in vec(any::<u8>(), 0..32),
    ) {
        let index = BoardIndex::from_objects(&objects, "board", state_vector.clone());
        let decoded = BoardIndex::decode(&index.encode()?)?;
        let bits = |listed: Vec<Object>| {
            listed
                .into_iter()
                .map(|object| format!("{object:?}|{}|{}|{}", object.x.to_bits(), object.y.to_bits(), object.z_index.to_bits()))
                .collect::<Vec<_>>()
        };
        prop_assert_eq!(bits(decoded.objects()), bits(objects));
        prop_assert_eq!(decoded.stamp().state_vector.clone(), state_vector);
    }
}
