use std::collections::HashMap;

use proptest::{
    collection::vec,
    option,
    prelude::{Strategy, any, prop_assert, prop_assert_eq, prop_oneof, proptest},
    sample::{Index, select},
};
use serde_json::json;
use yrs::{ReadTxn as _, Transact as _, updates::decoder::Decode as _};

use crate::{
    domain::{
        area::{Area, scene::Scene},
        board_file::{compression, compression::Compression},
        geometry::Rectangle,
        id_source::SequentialIds,
        picture,
        text::{is_unsafe, printable},
        text_layout,
        time::Timestamp,
    },
    laws::{config, uuid},
    protocol::hud,
};

fn outgoing() -> impl Strategy<Value = hud::OutgoingUpdate> {
    prop_oneof![
        vec(any::<u8>(), 0..64).prop_map(hud::OutgoingUpdate::Inline),
        (any::<u64>(), "[a-z0-9/-]{1,40}")
            .prop_map(|(size, link)| hud::OutgoingUpdate::Uploaded { size, link }),
    ]
}

fn colour() -> impl Strategy<Value = String> {
    (
        select(&["yellow", "blue", "gray", "white", "jade"][..]),
        1_u8..=12,
    )
        .prop_map(|(name, shade)| format!("{name}{shade}"))
}

fn note() -> impl Strategy<Value = String> {
    vec("[a-zA-Z0-9 ,.!?а-я]{0,20}", 1..4).prop_map(|lines| lines.join("\n"))
}

fn sticker(
    text: String,
    x: i32,
    y: i32,
    color: Option<crate::domain::palette::Color>,
) -> crate::domain::change::value::BoardChange {
    crate::domain::change::value::BoardChange::AddAll(vec![
        crate::domain::change::value::NewEntry {
            new: crate::domain::change::value::NewObject::Sticker {
                text,
                at: crate::domain::geometry::Point::whole(x, y),
                size: None,
                color,
            },
            frame: None,
            format: crate::domain::rich_text::TextFormat::Plain,
        },
    ])
}

proptest! {
    #![proptest_config(config())]

    /// A client update frame reads back as written.
    #[test]
    fn update_frame_round_trips(document in uuid(), id in 0_u64..(1 << 52), payload in outgoing()) {
        let frame = hud::update_frame(&document, id, &payload);
        prop_assert_eq!(hud::read_update_frame(&frame).ok(), Some((document, id, payload)));
    }

    /// Update ids stay below 2^52 and author numbers below 2^31.
    #[test]
    fn ids_fit_javascript(fresh in "\\PC{0,40}", salt in any::<u32>()) {
        prop_assert!(crate::domain::change::build::update_id(&fresh) < 1 << 52);
        prop_assert!(crate::domain::change::build::author_number(&fresh, salt) < 1 << 31);
    }

    /// Points, sizes and colors read back from how they print.
    #[test]
    fn change_values_round_trip(
        (x, y, width, height) in (-1e6_f64..1e6, -1e6_f64..1e6, 1e-3_f64..1e5, 1e-3_f64..1e5),
        token in colour(),
    ) {
        let point = crate::domain::geometry::Point::new(x, y)?;
        let size = crate::domain::geometry::Size::new(width, height)?;
        prop_assert_eq!(point.to_string().parse::<crate::domain::geometry::Point>().ok(), Some(point));
        prop_assert_eq!(size.to_string().parse::<crate::domain::geometry::Size>().ok(), Some(size));
        let parsed = token.parse::<crate::domain::palette::Color>().map(|c| c.to_string()).ok();
        prop_assert_eq!(parsed, Some(token));
    }

    /// The built update, replayed on a copy of the board as loaded, yields the
    /// asked sticker on top of every other object.
    #[test]
    fn built_sticker_replays_on_the_loaded_board(
        before in vec((note(), -500_i32..500, -500_i32..500), 0..4),
        (text, x, y) in (note(), -500_i32..500, -500_i32..500), token in colour(),
    ) {
        let doc = yrs::Doc::new();
        let ids = SequentialIds::new("o");
        let fresh = crate::domain::change::value::ChangeSources { built_at: Timestamp::from_millis(7), ids: &ids };
        before.into_iter().try_for_each(|(earlier, left, top)| {
            crate::domain::change::build::build(&doc, &sticker(earlier, left, top, None), "u", &fresh).map(drop)
        })?;
        let loaded = doc.transact().encode_state_as_update_v2(&yrs::StateVector::default());
        let color = token.parse::<crate::domain::palette::Color>()?;
        let built = crate::domain::change::build::build(&doc, &sticker(text.clone(), x, y, Some(color)), "u", &fresh)?;
        let replica = yrs::Doc::new();
        replica.transact_mut().apply_update(yrs::Update::decode_v2(&loaded)?)?;
        replica.transact_mut().apply_update(yrs::Update::decode_v2(&built.update)?)?;
        let objects = crate::domain::board::objects(&replica);
        let id = built.objects.first().map(|t| t.id.clone()).unwrap_or_default();
        let made = objects.iter().find(|o| o.id == id);
        prop_assert_eq!(
            made.map(|o| (o.text.clone(), o.x, o.y, o.color.clone())),
            Some((text, f64::from(x), f64::from(y), Some(token)))
        );
        let top = objects.iter().map(|o| o.z_index).fold(f64::MIN, f64::max);
        prop_assert_eq!(made.map(|o| o.z_index), Some(top));
    }

    /// On a board with earlier deletions, the sent update replayed on a copy of the loaded board gives the same state as the local change and deletes only what the change deleted.
    #[test]
    fn built_change_replays_exactly(
        history in vec((note(), -500_i32..500, -500_i32..500, any::<bool>()), 1..6),
        (remove, pick) in (any::<bool>(), any::<Index>()),
        (text, x, y) in (note(), -500_i32..500, -500_i32..500),
    ) {
        let server = yrs::Doc::with_client_id(9);
        let ids = SequentialIds::new("o");
        let fresh = crate::domain::change::value::ChangeSources { built_at: Timestamp::from_millis(7), ids: &ids };
        let made = history
            .iter()
            .map(|(earlier, left, top, _)| {
                crate::domain::change::build::build(&server, &sticker(earlier.clone(), *left, *top, None), "u", &fresh)
                    .map(|built| built.objects.first().map(|t| t.id.clone()).unwrap_or_default())
            })
            .collect::<crate::error::Result<Vec<_>>>()?;
        made.iter().zip(&history).filter(|(_, (.., gone))| *gone).try_for_each(|(id, _)| {
            crate::domain::change::build::build(&server, &crate::domain::change::value::BoardChange::RemoveAll(vec![id.clone()]), "u", &fresh).map(drop)
        })?;
        let state = server.transact().encode_state_as_update_v2(&yrs::StateVector::default());
        let loaded = yrs::Doc::with_client_id(7);
        loaded.transact_mut().apply_update(yrs::Update::decode_v2(&state)?)?;
        let replica = yrs::Doc::with_client_id(8);
        replica.transact_mut().apply_update(yrs::Update::decode_v2(&state)?)?;
        let alive = crate::domain::board::objects(&loaded);
        let op = alive.first().filter(|_| remove).map_or_else(
            || sticker(text, x, y, None),
            |_| crate::domain::change::value::BoardChange::RemoveAll(vec![pick.get(&alive).id.clone()]),
        );
        let before = loaded.transact().snapshot();
        let built = crate::domain::change::build::build(&loaded, &op, "u", &fresh)?;
        let after = loaded.transact().snapshot();
        replica.transact_mut().apply_update(yrs::Update::decode_v2(&built.update)?)?;
        prop_assert_eq!(crate::domain::board::objects(&replica), crate::domain::board::objects(&loaded));
        prop_assert_eq!(&replica.transact().snapshot(), &after);
        let sent = yrs::Update::decode_v2(&built.update)?;
        let fresh_deletions = after.delete_set.diff(&before.delete_set);
        prop_assert!(sent.delete_set().intersect(&before.delete_set).is_empty());
        prop_assert!(sent.delete_set().diff(&fresh_deletions).is_empty());
        prop_assert!(fresh_deletions.diff(sent.delete_set()).is_empty());
    }
}

type DrawableObject = (usize, i32, i32, u16, u16, i32, String);

fn scene_objects() -> impl Strategy<Value = Vec<crate::domain::board::Object>> {
    named_objects(String::new())
}

fn named_objects(
    prefix: String,
) -> impl Strategy<Value = Vec<crate::domain::board::Object>> {
    vec(
        (0_usize..6, -2000_i32..2000, -2000_i32..2000, 1_u16..600, 1_u16..600, -50_i32..50, note()),
        0..12,
    )
    .prop_map(move |drawn: Vec<DrawableObject>| {
        let count = drawn.len();
        drawn
            .into_iter()
            .enumerate()
            .map(|(at, (kind, x, y, width, height, z, text))| {
                let name = ["sticker", "simple-text", "shape", "frame", "arrow", "image"]
                    .get(kind)
                    .copied()
                    .unwrap_or("sticker");
                let other = (at + 1) % count.max(1);
                let before = (at + count.max(1) - 1) % count.max(1);
                let style = ["dashed", "dotted", "solid"].get(usize::try_from(z.rem_euclid(3)).unwrap_or_default());
                let fields = json!({
                    "id": format!("{prefix}o{at}"), "type": name, "position": {"x": x, "y": y},
                    "width": width, "height": height, "zIndex": z, "name": "img.png",
                    "start": {"objectId": format!("{prefix}o{before}"), "point": {"x": x, "y": y}},
                    "end": {"objectId": format!("{prefix}o{other}"), "point": {"x": y, "y": x}},
                    "shapeType": "ellipse", "strokeStyle": style, "fillColor": {"color": "yellow4", "opacity": 1},
                });
                crate::domain::board::Object {
                    id: format!("{prefix}o{at}"),
                    kind: crate::domain::board::ObjectKind::of(name),
                    parent: None,
                    x: f64::from(x),
                    y: f64::from(y),
                    width: (name != "simple-text").then_some(f64::from(width)),
                    height: (name != "simple-text").then_some(f64::from(height)),
                    z_index: f64::from(z),
                    text,
                    color: None,
                    look: None,
                    ends: None,
                    author: None,
                    locked: false,
                    rotation: 0.0,
                    file: crate::domain::board::stored_file(&crate::domain::board::ObjectKind::of(name), &fields),
                    fields,
                }
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(config())]

    /// An area lists only board objects, each overlapping it; a larger area
    /// lists at least as many.
    #[test]
    fn area_listing_is_a_subset_of_the_board(
        objects in scene_objects(),
        (x, y, width, height) in (-3000_i32..3000, -3000_i32..3000, 1_u16..4000, 1_u16..4000),
    ) {
        let scene = Scene::new(&objects);
        let area = Rectangle::new(f64::from(x), f64::from(y), f64::from(width), f64::from(height));
        let inside = scene.within(&area);
        prop_assert!(inside.iter().all(|o| objects.iter().any(|b| b.id == o.id)));
        prop_assert!(inside.iter().all(|o| scene.covered_by(o).is_some_and(|b| b.intersects(&area))));
        prop_assert!(scene.within(&area.padded(100.0)).len() >= inside.len());
    }

    /// The whole board's area holds every object that has a place.
    #[test]
    fn board_area_holds_everything(objects in scene_objects()) {
        let scene = Scene::new(&objects);
        let placed = objects.iter().filter(|o| scene.covered_by(o).is_some()).count();
        let listed = scene
            .area_rectangle(&Area::Board, 0.0)
            .map(|area| scene.within(&area).len())
            .unwrap_or_default();
        prop_assert_eq!(listed, placed);
    }

    /// Rendering is deterministic, and every object in the area is drawn.
    #[test]
    fn picture_draws_each_object_once_and_the_same_way(objects in scene_objects()) {
        let inlined = HashMap::new();
        let assets = picture::PictureImages { storage: "https://s/", replacements: &inlined };
        let request = picture::PictureRequest::new(Area::Board, None, 1.0, 2000.0)?;
        let first = picture::draw_with(&objects, &request, &assets).map(|drawn| drawn.picture);
        let again = picture::draw_with(&objects, &request, &assets).map(|drawn| drawn.picture);
        prop_assert_eq!(first.as_ref().ok(), again.as_ref().ok());
        let scene = Scene::new(&objects);
        first.iter().try_for_each(|picture| {
            prop_assert!(picture.width <= 2000 && picture.height <= 2000);
            scene.within(&picture.rectangle).iter().try_for_each(|o| {
                let id = format!("id=\"o-{}\"", o.id);
                prop_assert_eq!(picture.svg.matches(&id).count(), 1);
                Ok(())
            })
        })?;
    }

    /// Whatever text, ids and image names a board holds, the snapshot is well-formed XML.
    #[test]
    fn picture_svg_is_xml_for_any_board_text(
        objects in hostile_text().prop_flat_map(named_objects),
        texts in vec(hostile_text(), 13),
    ) {
        let stamp = crate::domain::board::Object {
            id: "stamp".into(),
            kind: crate::domain::board::ObjectKind::of("stamp"),
            parent: None,
            x: 0.0,
            y: 0.0,
            width: Some(40.0),
            height: Some(40.0),
            z_index: 0.0,
            text: String::new(),
            color: None,
            look: None,
            ends: None,
            author: None,
            locked: false,
            rotation: 0.0,
            file: None,
            fields: json!({"data": {"type": "textStamp", "text": texts.last()}}),
        };
        let hostile: Vec<crate::domain::board::Object> = objects
            .into_iter()
            .zip(&texts)
            .map(|(mut object, text)| {
                object.fields.pointer_mut("/name").into_iter().for_each(|name| {
                    *name = json!(text);
                });
                object.text.clone_from(text);
                object
            })
            .chain([stamp])
            .collect();
        let inlined = HashMap::new();
        let assets = picture::PictureImages { storage: "https://s/", replacements: &inlined };
        let request = picture::PictureRequest::new(Area::Board, None, 1.0, 2000.0)?;
        let picture = picture::draw_with(&hostile, &request, &assets)?.picture;
        prop_assert!(picture::is_xml(&picture.svg), "{}", picture.svg);
    }

    /// Printable text has nothing a terminal acts on and keeps everything else as it was.
    #[test]
    fn printable_text_only_replaces_unsafe_characters(raw in hostile_text()) {
        let shown = printable(&raw);
        prop_assert!(!shown.chars().any(is_unsafe));
        prop_assert_eq!(shown.chars().count(), raw.chars().count());
        prop_assert!(raw.chars().zip(shown.chars()).all(|(a, b)| a == b || is_unsafe(a)));
    }
}

fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[\\x00-\\x1f\\x7f-\\x9f<>&\"' a\\x{fffe}\\x{ffff}\\x{202e}\\x{2028}]{0,16}",
    ]
}

proptest! {
    #![proptest_config(config())]

    /// Gzip round-trips any bytes; plain bytes and non-gzip input pass through as they are.
    #[test]
    fn compression_round_trips_and_plain_is_untouched(bytes in vec(any::<u8>(), 0..4096)) {
        let gz = compression::compressed(&bytes, Compression::Gzip)?;
        prop_assert!(compression::is_compressed(&gz));
        prop_assert_eq!(compression::uncompressed(&gz)?.into_owned(), bytes.clone());
        prop_assert_eq!(compression::compressed(&bytes, Compression::Plain)?.into_owned(), bytes.clone());
        let plain = !compression::is_compressed(&bytes);
        prop_assert!(!plain || compression::uncompressed(&bytes)?.as_ref() == bytes.as_slice());
    }

    /// The sticker size search finds what trying every size largest first finds.
    #[test]
    fn sticker_font_search_matches_trying_every_size(
        text in prop_oneof![hostile_text(), "([a-zа-я]{1,12} ?){0,40}( [A-Z0-9]{10,30})?\n?[a-z ]{0,20}"],
        width in 20.0_f64..400.0,
        height in 20.0_f64..400.0,
        forced in option::of(4.0_f64..80.0),
    ) {
        let linear = text_layout::sticker_font_sizes()
            .iter()
            .copied()
            .find(|size| {
                let wrapped = text_layout::wrap(&text, width, *size);
                let tall = text_layout::count(wrapped.lines.len()) * size * text_layout::STICKER_LINE_HEIGHT;
                tall <= height && !wrapped.split_short
            })
            .unwrap_or(4.0);
        let expected = forced.map_or(linear, |forced| forced.min(linear));
        let found = text_layout::sticker_font_size(&text, width, height, forced);
        prop_assert_eq!(found.to_bits(), expected.to_bits());
    }

    /// A packed board state restores to the same state vector and objects.
    #[test]
    fn a_board_file_restores_the_same_board(
        xs in vec(-1000_i32..1000, 0..40),
        removed in vec(any::<Index>(), 0..5),
    ) {
        let doc = yrs::Doc::with_client_id(3);
        let objects = doc.get_or_insert_map("objects");
        xs.iter().enumerate().for_each(|(i, x)| {
            let fields = json!({"id": format!("o{i}"), "type": "sticker", "position": {"x": x, "y": i}});
            let value = yrs::Any::from_json(&fields.to_string()).unwrap_or(yrs::Any::Null);
            yrs::Map::insert(&objects, &mut doc.transact_mut(), format!("o{i}"), value);
        });
        removed.iter().filter(|_| !xs.is_empty()).for_each(|at| {
            yrs::Map::remove(&objects, &mut doc.transact_mut(), &format!("o{}", at.index(xs.len())));
        });
        let full = crate::domain::board_file::document::encoded(&doc);
        let copy = crate::domain::board_file::document::decoded(&compression::uncompressed(&compression::compressed(&full, Compression::Gzip)?)?)?;
        prop_assert_eq!(copy.transact().state_vector(), doc.transact().state_vector());
        prop_assert_eq!(crate::domain::board::objects(&copy), crate::domain::board::objects(&doc));
    }
}
