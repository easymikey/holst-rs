#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::Doc;

use super::{add, board as seeded, built, edit, fields_of};
use crate::{
    domain::{
        board::{ObjectKind, shape_type::StrokeStyle},
        change::{
            media::{self, CropMask, FileDisplay, FlipState, MediaEdit},
            value::{ImageUpload, NewObject, ObjectPatch, ShapeEdit, Upload},
        },
        geometry::{Point, Size},
        response::PagesInfo,
    },
    error::{Error, Result, board_change::BoardChangeError, input::InputError},
};

fn upload(original: &str) -> Upload {
    Upload {
        name: "d033829f.bin".into(),
        original: original.into(),
        mime: "application/octet-stream".into(),
        bytes: 70,
    }
}

/// The id of a new object added to the seeded board.
fn added(document: &Doc, new: NewObject) -> Result<String> {
    let built = built(document, &add(new))?;
    Ok(built
        .objects
        .into_iter()
        .next()
        .map(|object| object.id)
        .unwrap_or_default())
}

fn image(document: &Doc, original: &str) -> Result<String> {
    added(
        document,
        NewObject::Image {
            upload: ImageUpload {
                file: upload(original),
                pixels: Size::new(16.0, 12.0)?,
            },
            at: Point::ORIGIN,
            size: None,
        },
    )
}

fn file(document: &Doc, original: &str) -> Result<String> {
    added(
        document,
        NewObject::File {
            upload: upload(original),
            at: Point::ORIGIN,
        },
    )
}

fn named(name: &str) -> ObjectPatch {
    ObjectPatch {
        media: MediaEdit {
            name: Some(name.into()),
            ..MediaEdit::default()
        },
        ..ObjectPatch::default()
    }
}

#[rstest]
#[case::extension_not_added("Logo", "Logo")]
#[case::extension_kept("Logo.png", "Logo.png")]
#[case::trimmed("  Logo  ", "Logo")]
fn an_image_is_renamed_as_the_bundle_writes_it(
    #[case] given: &str,
    #[case] stored: &str,
) -> Result<()> {
    let document = seeded()?;
    let id = image(&document, "cat.png")?;
    built(&document, &edit(id.clone(), named(given)))?;
    assert_eq!(
        fields_of(&document, &id).get("originalName"),
        Some(&json!(stored))
    );
    Ok(())
}

#[test]
fn a_file_is_renamed_by_the_same_rule() -> Result<()> {
    let document = seeded()?;
    let id = file(&document, "plan.pdf")?;
    built(&document, &edit(id.clone(), named(" Report ")))?;
    assert_eq!(
        fields_of(&document, &id).get("displayFileName"),
        Some(&json!("Report"))
    );
    Ok(())
}

#[test]
fn a_name_equal_to_the_current_one_writes_nothing() -> Result<()> {
    let document = seeded()?;
    let id = image(&document, "cat.png")?;
    built(&document, &edit(id.clone(), named("cat")))?;
    assert_eq!(
        fields_of(&document, &id).get("updated"),
        Some(&json!(null)),
        "no update stamp when the name is the same"
    );
    Ok(())
}

#[test]
fn a_sticker_has_no_name() -> Result<()> {
    let document = seeded()?;
    let refused = built(&document, &edit("seed-2".into(), named("Logo")));
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "name",
            ..
        }))
    ));
    Ok(())
}

#[rstest]
#[case::empty("")]
#[case::blank("  ")]
fn a_blank_name_is_refused(#[case] given: &str) -> Result<()> {
    let document = seeded()?;
    let id = image(&document, "cat.png")?;
    let refused = built(&document, &edit(id, named(given)));
    assert!(matches!(
        refused,
        Err(Error::Input(InputError::BlankMediaName))
    ));
    Ok(())
}

#[test]
fn the_report_lists_the_name() -> Result<()> {
    let document = seeded()?;
    let id = image(&document, "cat.png")?;
    let report = built(&document, &edit(id, named("Logo")))?;
    assert_eq!(
        report.objects.first().map(|object| object.fields.clone()),
        Some(vec!["name"])
    );
    Ok(())
}

/// An image of 400 by 300 board units at the origin.
fn picture(document: &Doc) -> Result<String> {
    added(
        document,
        NewObject::Image {
            upload: ImageUpload {
                file: upload("cat.png"),
                pixels: Size::new(16.0, 12.0)?,
            },
            at: Point::ORIGIN,
            size: Some(Size::new(400.0, 300.0)?),
        },
    )
}

fn cropped(mask: CropMask) -> ObjectPatch {
    ObjectPatch {
        media: MediaEdit {
            crop: Some(mask),
            ..MediaEdit::default()
        },
        ..ObjectPatch::default()
    }
}

fn flipped(state: FlipState) -> ObjectPatch {
    ObjectPatch {
        media: MediaEdit {
            flip: Some(state),
            ..MediaEdit::default()
        },
        ..ObjectPatch::default()
    }
}

/// The numbers of a stored field, `null` as none.
fn numbers(fields: &Value, name: &str) -> Vec<Option<f64>> {
    fields
        .get(name)
        .and_then(Value::as_array)
        .map(|values| values.iter().map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// Corner, width and height as stored.
fn frame_of(fields: &Value) -> Vec<Option<f64>> {
    let corner = |axis: &str| {
        fields
            .get("position")
            .and_then(|position| position.get(axis))
            .and_then(Value::as_f64)
    };
    [
        corner("x"),
        corner("y"),
        fields.get("width").and_then(Value::as_f64),
        fields.get("height").and_then(Value::as_f64),
    ]
    .to_vec()
}

fn some(values: &[f64]) -> Vec<Option<f64>> {
    values.iter().copied().map(Some).collect()
}

#[test]
fn a_square_crop_takes_the_largest_square_and_none_gives_the_picture_back() -> Result<()>
{
    let document = seeded()?;
    let id = picture(&document)?;
    built(&document, &edit(id.clone(), cropped(CropMask::Square)))?;
    let square = fields_of(&document, &id);
    built(&document, &edit(id.clone(), cropped(CropMask::None)))?;
    let whole = fields_of(&document, &id);
    assert_eq!(
        (
            frame_of(&square),
            numbers(&square, "cropTransform"),
            frame_of(&whole),
            whole.get("cropTransform").cloned(),
        ),
        (
            some(&[50.0, 0.0, 300.0, 300.0]),
            vec![Some(0.125), Some(0.0), Some(0.75), Some(1.0), Some(2.0)],
            some(&[0.0, 0.0, 400.0, 300.0]),
            Some(json!([0, 0, 1, 1, null])),
        )
    );
    Ok(())
}

#[test]
fn a_turned_image_keeps_its_centre_when_cropped() -> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    built(
        &document,
        &edit(
            id.clone(),
            ObjectPatch {
                rotation: Some(90.0),
                ..ObjectPatch::default()
            },
        ),
    )?;
    built(&document, &edit(id.clone(), cropped(CropMask::Square)))?;
    assert_eq!(
        frame_of(&fields_of(&document, &id)),
        some(&[350.0, 0.0, 300.0, 300.0])
    );
    Ok(())
}

#[test]
fn a_flipped_image_reads_its_crop_from_the_other_side() {
    let fields = json!({
        "type": "image", "position": {"x": 87.5, "y": 0}, "width": 225, "height": 300,
        "cropTransform": [0.3, 0, 0.5625, 1, 3], "flip": {"x": -1, "y": 1},
    });
    let written = media::changes(
        &ObjectKind::Image,
        &MediaEdit {
            crop: Some(CropMask::Square),
            ..MediaEdit::default()
        },
        &fields,
    );
    assert_eq!(
        written
            .into_iter()
            .find(|(name, _)| *name == "cropTransform")
            .map(|(_, value)| value),
        Some(json!([0.3, 0.125, 0.5625, 0.75, 2]))
    );
}

#[rstest]
#[case::horizontal(json!({"x": 1, "y": 1}), FlipState::Horizontal, Some(json!({"x": -1, "y": 1})))]
#[case::vertical(json!({"x": 1, "y": 1}), FlipState::Vertical, Some(json!({"x": 1, "y": -1})))]
#[case::both(json!({"x": 1, "y": 1}), FlipState::Both, Some(json!({"x": -1, "y": -1})))]
#[case::none(json!({"x": -1, "y": 1}), FlipState::None, Some(json!({"x": 1, "y": 1})))]
#[case::already(json!({"x": -1, "y": 1}), FlipState::Horizontal, None)]
fn a_flip_is_set_to_the_state_asked(
    #[case] now: Value,
    #[case] state: FlipState,
    #[case] written: Option<Value>,
) {
    let fields = json!({ "type": "image", "flip": now });
    let changed = media::changes(
        &ObjectKind::Image,
        &MediaEdit {
            flip: Some(state),
            ..MediaEdit::default()
        },
        &fields,
    );
    assert_eq!(changed.into_iter().next().map(|(_, value)| value), written);
}

#[rstest]
#[case::crop(cropped(CropMask::Circle), "sticker objects have no crop to change")]
#[case::flip(flipped(FlipState::Both), "sticker objects have no flip to change")]
fn a_sticker_has_no_crop_or_flip(
    #[case] patch: ObjectPatch,
    #[case] message: &str,
) -> Result<()> {
    let document = seeded()?;
    let refused = built(&document, &edit("seed-2".into(), patch));
    assert_eq!(
        refused.err().map(|error| error.to_string()).as_deref(),
        Some(message)
    );
    Ok(())
}

#[test]
fn a_repeated_flip_writes_nothing_more() -> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    built(&document, &edit(id.clone(), flipped(FlipState::Horizontal)))?;
    let first = fields_of(&document, &id);
    built(&document, &edit(id.clone(), flipped(FlipState::Horizontal)))?;
    assert_eq!(
        (first.get("flip").cloned(), fields_of(&document, &id)),
        (Some(json!({"x": -1, "y": 1})), first)
    );
    Ok(())
}

fn outlined(outline: ShapeEdit) -> ObjectPatch {
    ObjectPatch {
        shape: outline,
        ..ObjectPatch::default()
    }
}

fn rounded(radius: u32) -> ObjectPatch {
    outlined(ShapeEdit {
        radius: Some(radius),
        ..ShapeEdit::default()
    })
}

#[test]
fn an_image_takes_an_outline_and_round_corners_clamped_to_half_the_shorter_side()
-> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    built(
        &document,
        &edit(
            id.clone(),
            outlined(ShapeEdit {
                stroke: Some("gray12".parse()?),
                stroke_width: Some(4.0),
                stroke_style: Some(StrokeStyle::Dashed),
                radius: Some(20),
                ..ShapeEdit::default()
            }),
        ),
    )?;
    let kept = fields_of(&document, &id);
    built(&document, &edit(id.clone(), rounded(500)))?;
    let clamped = fields_of(&document, &id);
    assert_eq!(
        (
            kept.get("strokeColor").cloned(),
            kept.get("strokeWidth").cloned(),
            kept.get("strokeStyle").cloned(),
            kept.get("borderRadius").cloned(),
            clamped.get("borderRadius").cloned(),
        ),
        (
            Some(json!({"color": "gray12", "opacity": 1})),
            Some(json!(4)),
            Some(json!("dashed")),
            Some(json!(20)),
            Some(json!(150)),
        )
    );
    Ok(())
}

#[test]
fn a_circle_cropped_image_has_no_corner_radius() -> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    built(&document, &edit(id.clone(), cropped(CropMask::Circle)))?;
    let refused = built(&document, &edit(id, rounded(10)));
    assert_eq!(
        refused.err().map(|error| error.to_string()).as_deref(),
        Some("circle-cropped image objects have no radius to change")
    );
    Ok(())
}

#[test]
fn a_radius_beside_a_circle_crop_is_refused_and_beside_another_mask_is_not()
-> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    let with = |mask| ObjectPatch {
        media: MediaEdit {
            crop: Some(mask),
            ..MediaEdit::default()
        },
        ..rounded(10)
    };
    let circle = built(&document, &edit(id.clone(), with(CropMask::Circle)));
    let square = built(&document, &edit(id, with(CropMask::Square)));
    assert_eq!((circle.is_err(), square.is_ok()), (true, true));
    Ok(())
}

#[test]
fn a_sticker_still_has_no_outline() -> Result<()> {
    let document = seeded()?;
    let refused = built(
        &document,
        &edit(
            "seed-2".into(),
            outlined(ShapeEdit {
                stroke: Some("gray12".parse()?),
                ..ShapeEdit::default()
            }),
        ),
    );
    assert_eq!(
        refused.err().map(|error| error.to_string()).as_deref(),
        Some("sticker objects have no stroke to change")
    );
    Ok(())
}

#[test]
fn an_image_takes_no_fill_opacity() -> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    let refused = built(
        &document,
        &edit(
            id,
            outlined(ShapeEdit {
                opacity: Some(0.5),
                ..ShapeEdit::default()
            }),
        ),
    );
    assert_eq!(
        refused.err().map(|error| error.to_string()).as_deref(),
        Some("image objects have no opacity to change")
    );
    Ok(())
}

fn replaced_by(pixels: Size) -> ObjectPatch {
    ObjectPatch {
        media: MediaEdit {
            replace: Some(ImageUpload {
                file: Upload {
                    name: "9f3a.png".into(),
                    original: "dog.png".into(),
                    mime: "image/png".into(),
                    bytes: 70,
                },
                pixels,
            }),
            ..MediaEdit::default()
        },
        ..ObjectPatch::default()
    }
}

#[test]
fn a_new_picture_covers_the_old_box_and_leaves_the_box_alone() -> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    built(
        &document,
        &edit(id.clone(), replaced_by(Size::new(100.0, 100.0)?)),
    )?;
    let after = fields_of(&document, &id);
    assert_eq!(
        (
            numbers(&after, "cropTransform"),
            frame_of(&after),
            after.get("naturalWidth").and_then(Value::as_f64),
            after.get("naturalHeight").and_then(Value::as_f64),
            after.get("name").cloned(),
            after.get("originalName").cloned(),
            after.get("imageType").cloned(),
        ),
        (
            vec![Some(0.0), Some(0.125), Some(1.0), Some(0.75), None],
            some(&[0.0, 0.0, 400.0, 300.0]),
            Some(100.0),
            Some(100.0),
            Some(json!("9f3a.png")),
            Some(json!("dog.png")),
            Some(json!("image/png")),
        )
    );
    Ok(())
}

#[test]
fn a_wider_picture_is_cut_at_the_sides_and_the_mask_is_kept() -> Result<()> {
    let document = seeded()?;
    let id = picture(&document)?;
    built(&document, &edit(id.clone(), cropped(CropMask::Square)))?;
    built(
        &document,
        &edit(id.clone(), replaced_by(Size::new(600.0, 300.0)?)),
    )?;
    assert_eq!(
        numbers(&fields_of(&document, &id), "cropTransform"),
        some(&[0.25, 0.0, 0.5, 1.0, 2.0])
    );
    Ok(())
}

#[test]
fn a_sticker_takes_no_new_picture() -> Result<()> {
    let document = seeded()?;
    let refused = built(
        &document,
        &edit("seed-2".into(), replaced_by(Size::new(10.0, 10.0)?)),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "path",
            ..
        }))
    ));
    Ok(())
}

fn file_with(name: &str, extra: &Value) -> Value {
    let mut fields = json!({
        "type": "file", "fileName": name, "displayFileName": "shown",
        "fileType": "application/octet-stream", "displayType": "stamp",
    });
    fields.as_object_mut().into_iter().for_each(|map| {
        extra
            .as_object()
            .into_iter()
            .flatten()
            .for_each(|(key, value)| {
                map.insert(key.clone(), value.clone());
            });
    });
    fields
}

fn viewed(
    display: Option<FileDisplay>,
    start_page: Option<u32>,
    hide_details: Option<bool>,
) -> MediaEdit {
    MediaEdit {
        display,
        start_page,
        hide_details,
        ..MediaEdit::default()
    }
}

fn pages_of(count: u32) -> Value {
    json!({"pagesInfo": {"count": count, "valid": true}})
}

#[test]
fn a_pdf_with_pages_is_shown_as_content_from_page_two() -> Result<()> {
    let fields = file_with("a1.pdf", &pages_of(3));
    let view = viewed(Some(FileDisplay::Content), Some(2), None);
    media::fits(&ObjectKind::File, Some(&fields), &view)?;
    assert_eq!(
        media::changes(&ObjectKind::File, &view, &fields),
        vec![("displayType", json!("content")), ("pinnedPage", json!(2))]
    );
    Ok(())
}

#[test]
fn a_start_page_past_the_last_page_is_refused() {
    let fields = file_with("a1.pdf", &pages_of(3));
    let refused = media::fits(
        &ObjectKind::File,
        Some(&fields),
        &viewed(None, Some(4), None),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NoPage {
            page: 4,
            count: 3
        }))
    ));
}

#[test]
fn a_start_page_needs_the_page_count() {
    let fields = file_with("a1.pdf", &json!({}));
    let refused = media::fits(
        &ObjectKind::File,
        Some(&fields),
        &viewed(None, Some(1), None),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "start_page",
            ..
        }))
    ));
}

#[test]
fn a_video_with_info_hides_its_details_as_content() -> Result<()> {
    let fields = file_with("v1.mp4", &json!({"videoInfo": {"duration": 3}}));
    let view = viewed(Some(FileDisplay::Content), None, Some(true));
    media::fits(&ObjectKind::File, Some(&fields), &view)?;
    assert_eq!(
        media::changes(&ObjectKind::File, &view, &fields),
        vec![
            ("displayType", json!("content")),
            ("hideDetails", json!(true))
        ]
    );
    Ok(())
}

#[test]
fn details_of_a_video_shown_as_a_stamp_are_refused() {
    let fields = file_with("v1.mp4", &json!({"videoInfo": {"duration": 3}}));
    let refused = media::fits(
        &ObjectKind::File,
        Some(&fields),
        &viewed(None, None, Some(true)),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "hide_details",
            ..
        }))
    ));
}

#[rstest]
#[case::audio_without_info("a1.mp3", json!({}))]
#[case::video_without_info("v1.mp4", json!({}))]
#[case::pages_not_valid("a1.pdf", json!({"pagesInfo": {"count": 3, "valid": false}}))]
#[case::a_file_with_no_view("a1.zip", json!({}))]
fn content_needs_the_info_its_kind_shows(#[case] name: &str, #[case] extra: Value) {
    let fields = file_with(name, &extra);
    let refused = media::fits(
        &ObjectKind::File,
        Some(&fields),
        &viewed(Some(FileDisplay::Content), None, None),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "display",
            ..
        }))
    ));
}

#[test]
fn the_stamp_always_works_on_a_file() -> Result<()> {
    let shown = file_with("a1.mp3", &json!({"displayType": "content"}));
    let view = viewed(Some(FileDisplay::Stamp), None, None);
    media::fits(&ObjectKind::File, Some(&shown), &view)?;
    let plain = file_with("a1.mp3", &json!({}));
    media::fits(&ObjectKind::File, Some(&plain), &view)?;
    assert_eq!(
        (
            media::changes(&ObjectKind::File, &view, &shown),
            media::changes(&ObjectKind::File, &view, &plain)
        ),
        (vec![("displayType", json!("stamp"))], vec![])
    );
    Ok(())
}

#[test]
fn a_sticker_has_no_file_view() {
    let refused = media::fits(
        &ObjectKind::Sticker,
        None,
        &viewed(Some(FileDisplay::Stamp), None, None),
    );
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "display",
            ..
        }))
    ));
}

fn counted(count: u32, valid: bool) -> MediaEdit {
    MediaEdit {
        pages: Some(PagesInfo { count, valid }),
        ..viewed(Some(FileDisplay::Content), None, None)
    }
}

#[test]
fn a_counted_pdf_is_shown_as_content_with_its_pages_stored() -> Result<()> {
    let fields = file_with("a1.pdf", &json!({}));
    let view = counted(3, true);
    media::fits(&ObjectKind::File, Some(&fields), &view)?;
    assert_eq!(
        media::changes(&ObjectKind::File, &view, &fields),
        vec![
            ("pagesInfo", json!({"count": 3, "valid": true})),
            ("displayType", json!("content"))
        ]
    );
    Ok(())
}

#[test]
fn pages_that_cannot_be_shown_are_not_shown_as_content() {
    let fields = file_with("a1.pdf", &json!({}));
    let refused = media::fits(&ObjectKind::File, Some(&fields), &counted(1, false));
    assert!(matches!(
        refused,
        Err(Error::BoardChange(BoardChangeError::NotChangeable {
            field: "display",
            ..
        }))
    ));
}

#[test]
fn an_uncounted_pdf_passes_the_check_to_be_counted_before_the_write() -> Result<()> {
    let fields = file_with("a1.pdf", &json!({}));
    let view = viewed(Some(FileDisplay::Content), Some(2), None);
    media::fits(&ObjectKind::File, Some(&fields), &view)?;
    assert_eq!(media::uncounted(&fields), Some("a1.pdf"));
    let counted = file_with("a1.pdf", &pages_of(3));
    assert_eq!(media::uncounted(&counted), None);
    let plain = file_with("a1.zip", &json!({}));
    assert_eq!(media::uncounted(&plain), None);
    Ok(())
}
