#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::collections::HashMap;

use rstest::rstest;
use serde_json::{Value, json};
use yrs::{Doc, Transact as _, Update, updates::decoder::Decode as _};

use super::{RemoveRight, add, board, built, check};
use crate::{
    domain::{
        change::{
            build::build,
            tests::{ME, fields_of},
            value::{ChangeSources, ImageUpload, NewObject, Upload},
        },
        geometry::{Point, Size},
        id_source::SequentialIds,
        response::{EmbedInfo, LinkPreview},
        time::Timestamp,
    },
    error::Result,
};

const URL: &str = "https://example.com/pasted";

/// The object as a fresh document reads it from the v2 update.
fn written(new: NewObject) -> Result<Value> {
    let ids = SequentialIds::new("n");
    let built = build(
        &Doc::new(),
        &add(new),
        ME,
        &ChangeSources {
            built_at: Timestamp::from_millis(5),
            ids: &ids,
        },
    )?;
    let replica = Doc::new();
    replica
        .transact_mut()
        .apply_update(Update::decode_v2(&built.update)?)?;
    let id = built.objects.first().map(|object| object.id.clone());
    Ok(fields_of(&replica, id.as_deref().unwrap_or_default()))
}

/// The fields every new object has, split off: (id, layer, created, updated).
fn common(fields: &mut Value) -> (Value, bool, Value, Value) {
    let mut take = |name: &str| {
        fields
            .as_object_mut()
            .and_then(|map| map.remove(name))
            .unwrap_or_default()
    };
    (
        take("id"),
        take("zIndex").is_number(),
        take("created"),
        take("updated"),
    )
}

fn link(preview: Option<LinkPreview>, embed: bool) -> Result<NewObject> {
    Ok(NewObject::Link {
        url: URL.into(),
        at: Point::new(1000.0, 622.0)?,
        preview,
        embed,
    })
}

fn upload(name: &str, original: &str, mime: &str, bytes: u64) -> Upload {
    Upload {
        name: name.into(),
        original: original.into(),
        mime: mime.into(),
        bytes,
    }
}

#[test]
fn a_link_card_is_written_as_the_web_client_pastes_it() -> Result<()> {
    let preview = LinkPreview {
        title: Some("Example Domain".into()),
        ..LinkPreview::default()
    };
    let mut fields = written(link(Some(preview), true)?)?;
    let (id, layered, created, updated) = common(&mut fields);
    assert_eq!(
        (id, layered, created.get("t").cloned(), updated, fields),
        (
            json!("n-1"),
            true,
            Some(json!(5)),
            Value::Null,
            json!({
                "type": "link", "link": URL, "position": {"x": 1000, "y": 622},
                "displayType": 1, "parentId": null,
                "linkInfo": {
                    "description": null, "faviconKey": null, "imageKey": null,
                    "link": URL, "title": "Example Domain",
                },
            })
        )
    );
    Ok(())
}

#[test]
fn a_link_without_preview_keeps_null_link_info() -> Result<()> {
    let mut fields = written(link(None, true)?)?;
    common(&mut fields);
    assert_eq!(
        fields.get("linkInfo"),
        Some(&json!({
            "description": null, "faviconKey": null, "imageKey": null,
            "link": URL, "title": null,
        }))
    );
    assert_eq!(fields.get("displayType"), Some(&json!(1)));
    Ok(())
}

fn youtube() -> LinkPreview {
    LinkPreview {
        title: Some("YouTube".into()),
        favicon_key: Some("link-preview-image-a.ico".into()),
        embed: Some(Box::new(EmbedInfo {
            kind: Some("video".into()),
            version: Some("1.0".into()),
            title: Some("Clip".into()),
            provider_name: Some("YouTube".into()),
            thumbnail_key: Some("link-preview-image-b.jpeg".into()),
            html: Some("<iframe src=\"x\"></iframe>".into()),
            width: Some(200.0),
            height: Some(113.0),
        })),
        ..LinkPreview::default()
    }
}

#[test]
fn an_embed_preview_makes_an_embedded_card() -> Result<()> {
    let mut fields = written(link(Some(youtube()), true)?)?;
    common(&mut fields);
    assert_eq!(fields.get("displayType"), Some(&json!(2)));
    assert_eq!(
        fields.get("linkInfo"),
        Some(&json!({
            "description": null, "imageKey": null, "link": URL,
            "title": "YouTube", "faviconKey": "link-preview-image-a.ico",
            "embedInfo": {
                "type": "video", "version": "1.0", "title": "Clip",
                "providerName": "YouTube",
                "thumbnailKey": "link-preview-image-b.jpeg",
                "html": "<iframe src=\"x\"></iframe>",
                "width": 200, "height": 113,
            },
        }))
    );
    Ok(())
}

#[test]
fn no_preview_keeps_a_plain_card_for_an_embed_preview() -> Result<()> {
    let mut fields = written(link(Some(youtube()), false)?)?;
    common(&mut fields);
    assert_eq!(fields.get("displayType"), Some(&json!(1)));
    assert_eq!(fields.pointer("/linkInfo/embedInfo"), None, "{fields}");
    Ok(())
}

#[test]
fn an_image_object_holds_its_upload_and_pixels() -> Result<()> {
    let mut fields = written(NewObject::Image {
        upload: ImageUpload {
            file: upload("d033829f.png", "zz.png", "image/png", 70),
            pixels: Size::new(16.0, 12.0)?,
        },
        at: Point::new(712.0, 445.0)?,
        size: None,
    })?;
    let (_, layered, _, updated) = common(&mut fields);
    assert_eq!(
        (layered, updated, fields),
        (
            true,
            Value::Null,
            json!({
                "type": "image", "name": "d033829f.png", "originalName": "zz.png",
                "imageType": "image/png", "width": 16, "height": 12,
                "naturalWidth": 16, "naturalHeight": 12, "isLoading": false,
                "position": {"x": 712, "y": 445}, "parentId": null,
            })
        )
    );
    Ok(())
}

#[test]
fn an_image_shown_at_a_size_keeps_its_natural_pixels() -> Result<()> {
    let fields = written(NewObject::Image {
        upload: ImageUpload {
            file: upload("a.png", "a.png", "image/png", 70),
            pixels: Size::new(16.0, 12.0)?,
        },
        at: Point::ORIGIN,
        size: Some(Size::new(160.0, 120.0)?),
    })?;
    assert_eq!(
        [
            fields.get("width"),
            fields.get("height"),
            fields.get("naturalWidth"),
            fields.get("naturalHeight"),
        ],
        [
            Some(&json!(160)),
            Some(&json!(120)),
            Some(&json!(16)),
            Some(&json!(12)),
        ]
    );
    Ok(())
}

#[test]
fn a_file_object_holds_its_type_size_and_stamp_display() -> Result<()> {
    let mut fields = written(NewObject::File {
        upload: upload("4301ae87.pdf", "zz.pdf", "application/pdf", 544),
        at: Point::new(720.0, 453.0)?,
    })?;
    let (_, layered, _, updated) = common(&mut fields);
    assert_eq!(
        (layered, updated, fields),
        (
            true,
            Value::Null,
            json!({
                "type": "file", "fileName": "4301ae87.pdf",
                "displayFileName": "zz.pdf", "fileType": "application/pdf",
                "fileSize": 544, "displayType": "stamp",
                "position": {"x": 720, "y": 453}, "parentId": null,
            })
        )
    );
    Ok(())
}

const PEN: u32 = 1_973_790;

fn stroke(points: &[(f64, f64)], width: f64) -> Result<NewObject> {
    Ok(NewObject::Drawing {
        points: points
            .iter()
            .map(|&(x, y)| Point::new(x, y))
            .collect::<Result<_>>()?,
        color: PEN,
        width,
        alpha: 1.0,
    })
}

fn refused(new: NewObject) -> Option<String> {
    check(&add(new), &HashMap::new(), RemoveRight::Anyone, None)
        .err()
        .map(|error| error.to_string())
}

#[test]
fn a_pen_stroke_is_written_as_the_web_pen_writes_one() -> Result<()> {
    let mut fields = written(stroke(
        &[(300.0, 500.0), (450.0, 550.0), (600.0, 600.0)],
        3.0,
    )?)?;
    let (_, layered, _, updated) = common(&mut fields);
    assert_eq!(
        (layered, updated, fields),
        (
            true,
            Value::Null,
            json!({
                "type": "drawing", "algorithm": "lazy", "path": "0,0,150,50,300,100",
                "pivot": {"x": 0, "y": 0}, "position": {"x": 300, "y": 500},
                "strokeWidth": 3, "color": 1_973_790, "alpha": 1, "parentId": null,
            })
        )
    );
    Ok(())
}

#[test]
fn a_highlighter_stroke_keeps_its_color_width_and_alpha() -> Result<()> {
    let fields = written(NewObject::Drawing {
        points: vec![Point::ORIGIN, Point::new(40.0, 0.0)?],
        color: 16_511_040,
        width: 22.0,
        alpha: 0.5,
    })?;
    assert_eq!(
        [
            fields.get("color"),
            fields.get("strokeWidth"),
            fields.get("alpha"),
            fields.get("width"),
            fields.get("height"),
        ],
        [
            Some(&json!(16_511_040)),
            Some(&json!(22)),
            Some(&json!(0.5)),
            None,
            None,
        ]
    );
    Ok(())
}

#[rstest]
#[case::up_left(&[(300.0, 500.0), (200.0, 400.0)], "0,0,-100,-100", (-100.0, -100.0), (200.0, 400.0))]
#[case::rounded_to_a_tenth(&[(300.04, 500.06), (450.26, 550.34)], "0,0,150.2,50.3", (0.0, 0.0), (300.04, 500.06))]
#[case::one_point_repeats(&[(10.0, 20.0)], "0,0,0,0", (0.0, 0.0), (10.0, 20.0))]
fn a_stroke_is_relative_to_its_first_point_and_positioned_by_its_pivot(
    #[case] points: &[(f64, f64)],
    #[case] path: &str,
    #[case] pivot: (f64, f64),
    #[case] position: (f64, f64),
) -> Result<()> {
    let fields = written(stroke(points, 3.0)?)?;
    let number = |pointer: &str| fields.pointer(pointer).and_then(Value::as_f64);
    assert_eq!(
        (
            fields.get("path").and_then(Value::as_str),
            number("/pivot/x").zip(number("/pivot/y")),
            number("/position/x").zip(number("/position/y")),
        ),
        (Some(path), Some(pivot), Some(position))
    );
    Ok(())
}

#[rstest]
#[case::centre_inside(&[(-100.0, 100.0), (200.0, 100.0)], Some("seed-1"))]
#[case::centre_outside(&[(-900.0, 100.0), (100.0, 100.0)], None)]
fn a_stroke_goes_into_the_frame_around_its_centre(
    #[case] points: &[(f64, f64)],
    #[case] parent: Option<&str>,
) -> Result<()> {
    let document = board()?;
    let update = built(&document, &add(stroke(points, 3.0)?))?;
    let id = update.objects.first().map(|object| object.id.clone());
    let fields = fields_of(&document, id.as_deref().unwrap_or_default());
    assert_eq!(fields.get("parentId"), Some(&json!(parent)));
    Ok(())
}

#[rstest]
#[case::none(
    0,
    3.0,
    Some("\"0\" is not a valid drawing point count: expected 1 to 10000")
)]
#[case::most(10_000, 3.0, None)]
#[case::too_many(
    10_001,
    3.0,
    Some("\"10001\" is not a valid drawing point count: expected 1 to 10000")
)]
#[case::thin(
    2,
    0.5,
    Some("\"0.5\" is not a valid drawing stroke width: expected 1 to 30")
)]
#[case::thinnest(2, 1.0, None)]
#[case::widest(2, 30.0, None)]
#[case::wide(
    2,
    31.0,
    Some("\"31\" is not a valid drawing stroke width: expected 1 to 30")
)]
fn a_stroke_has_up_to_10000_points_and_a_width_of_1_to_30(
    #[case] count: usize,
    #[case] width: f64,
    #[case] error: Option<&str>,
) {
    let refusal = refused(NewObject::Drawing {
        points: vec![Point::ORIGIN; count],
        color: PEN,
        width,
        alpha: 1.0,
    });
    assert_eq!(refusal.as_deref(), error);
}
