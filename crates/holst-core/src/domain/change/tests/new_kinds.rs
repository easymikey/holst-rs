#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use serde_json::{Value, json};
use yrs::{Doc, Transact as _, Update, updates::decoder::Decode as _};

use super::add;
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
                "displayType": 1,
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
                "position": {"x": 712, "y": 445},
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
                "position": {"x": 720, "y": 453},
            })
        )
    );
    Ok(())
}
