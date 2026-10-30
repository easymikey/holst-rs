//! The text and the colour flags reach the kinds the web stores them on.
#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use rstest::rstest;
use serde_json::{Value, json};
use yrs::{Doc, GetString as _, Text as _, Transact as _};

use super::{built, edit, fields_of, frame::seed};
use crate::{
    domain::{
        board::text_of,
        change::{
            check::{DOCUMENT_COLORS, ICON_COLORS, TASK_COLORS},
            kind_edit::{KindEdit, TaskEdit},
            value::ObjectPatch,
        },
        kinds::{document, icon, task},
        palette::Color,
        rich_text::TextFormat,
    },
    error::Result,
};

fn applied(stored: &Value, patch: ObjectPatch) -> Result<Doc> {
    let document = Doc::new();
    seed(&document, stored)?;
    built(&document, &edit("o".into(), patch))?;
    Ok(document)
}

fn refusal(stored: &Value, patch: ObjectPatch) -> Option<String> {
    applied(stored, patch).err().map(|error| error.to_string())
}

fn texted(text: &str, format: TextFormat) -> ObjectPatch {
    ObjectPatch {
        text: Some(text.into()),
        format,
        ..ObjectPatch::default()
    }
}

fn colored(token: &str) -> Result<ObjectPatch> {
    Ok(ObjectPatch {
        color: Some(token.parse::<Color>()?),
        ..ObjectPatch::default()
    })
}

/// The text of the document the field names, read back.
fn written(document: &Doc, field: &str) -> String {
    let id = fields_of(document, "o")
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let documents = document.get_or_insert_map("documents");
    text_of(&document.transact(), &documents, &id)
}

#[rstest]
#[case::document(json!({"id": "o", "type": "card"}), "documentId")]
#[case::mind_map(json!({"id": "o", "type": "mind-map-node"}), "documentId")]
#[case::task(json!({"id": "o", "type": "task-card"}), "titleDocumentId")]
#[case::flipcard(json!({"id": "o", "type": "flip-card"}), "frontDocumentId")]
fn a_document_text_is_written_where_the_web_stores_it(
    #[case] stored: Value,
    #[case] field: &str,
) -> Result<()> {
    let document = applied(&stored, texted("New words", TextFormat::Plain))?;
    assert_eq!(written(&document, field), "New words");
    Ok(())
}

#[test]
fn a_code_block_text_replaces_its_root_text() -> Result<()> {
    let stored = json!({"id": "o", "type": "code", "documentId": "d1"});
    let document = Doc::new();
    seed(&document, &stored)?;
    document
        .get_or_insert_text("code-mirror-d1")
        .push(&mut document.transact_mut(), "old");
    built(
        &document,
        &edit("o".into(), texted("let x = 1;", TextFormat::Plain)),
    )?;
    let root = document.get_or_insert_text("code-mirror-d1");
    assert_eq!(root.get_string(&document.transact()), "let x = 1;");
    Ok(())
}

#[test]
fn a_sticker_stack_text_is_its_title() -> Result<()> {
    let stored = json!({"id": "o", "type": "sticker-stack", "title": "Old"});
    let document = applied(&stored, texted("Ideas", TextFormat::Plain))?;
    assert_eq!(
        fields_of(&document, "o").get("title"),
        Some(&json!("Ideas"))
    );
    Ok(())
}

#[test]
fn a_text_stamp_text_is_kept_in_its_data() -> Result<()> {
    let stored =
        json!({"id": "o", "type": "stamp", "data": {"type": "textStamp", "text": "A"}});
    let document = applied(&stored, texted("Ship it", TextFormat::Plain))?;
    assert_eq!(
        fields_of(&document, "o").get("data"),
        Some(&json!({"type": "textStamp", "text": "Ship it"}))
    );
    Ok(())
}

#[test]
fn an_image_stamp_has_no_text_to_change() {
    let stored = json!({"id": "o", "type": "stamp", "data": {"type": "imageStamp"}});
    assert_eq!(
        refusal(&stored, texted("x", TextFormat::Plain)).as_deref(),
        Some("stamp objects have no text to change")
    );
}

#[rstest]
#[case::code(json!({"id": "o", "type": "code"}), "code")]
#[case::task(json!({"id": "o", "type": "task-card"}), "task-card")]
#[case::flipcard(json!({"id": "o", "type": "flip-card"}), "flip-card")]
#[case::sticker_stack(json!({"id": "o", "type": "sticker-stack"}), "sticker-stack")]
#[case::stamp(json!({"id": "o", "type": "stamp", "data": {"type": "textStamp"}}), "stamp")]
fn markdown_is_refused_where_the_text_is_plain(
    #[case] stored: Value,
    #[case] name: &str,
) {
    assert_eq!(
        refusal(&stored, texted("# x", TextFormat::Markdown)),
        Some(format!("{name} objects have no text format to change"))
    );
}

#[test]
fn markdown_reaches_a_document() -> Result<()> {
    let document = applied(
        &json!({"id": "o", "type": "card"}),
        texted("# Head", TextFormat::Markdown),
    )?;
    assert_eq!(written(&document, "documentId"), "Head");
    Ok(())
}

#[rstest]
#[case::document("card", "blue3", "colorToken", json!("blue3"))]
#[case::mind_map("mind-map-node", "blue3", "nodeColor", json!({"color": "blue3", "opacity": 1}))]
#[case::flipcard("flip-card", "blue3", "fillColor", json!({"color": "blue3", "opacity": 1}))]
#[case::dice("dice", "blue3", "fillColor", json!({"color": "blue3", "opacity": 1}))]
#[case::sticker_stack("sticker-stack", "blue3", "fillColor", json!({"color": "blue3", "opacity": 1}))]
#[case::icon("phosphor-icon", "blue9", "fillColor", json!({"color": "blue9", "opacity": 1}))]
fn a_colour_is_stored_where_the_web_stores_it(
    #[case] kind: &str,
    #[case] token: &str,
    #[case] field: &str,
    #[case] stored: Value,
) -> Result<()> {
    let document = applied(&json!({"id": "o", "type": kind}), colored(token)?)?;
    assert_eq!(fields_of(&document, "o").get(field), Some(&stored));
    Ok(())
}

#[rstest]
#[case::document(
    "card",
    "blue9",
    "white1, gray3, red3, orange3, yellow3, green3, blue3, violet3"
)]
#[case::icon(
    "phosphor-icon",
    "blue3",
    "gray12, gray8, red9, orange9, yellow9, lime9, green9, mint9, blue9, indigo9, violet9, pink9"
)]
fn a_colour_outside_the_picker_is_refused_with_the_set(
    #[case] kind: &str,
    #[case] token: &str,
    #[case] set: &str,
) -> Result<()> {
    let said = refusal(&json!({"id": "o", "type": kind}), colored(token)?);
    assert_eq!(
        said,
        Some(format!(
            "\"{token}\" is not a valid color: expected one of {set}"
        ))
    );
    Ok(())
}

#[test]
fn a_task_colour_is_a_name_of_its_palette_and_fills_the_card() -> Result<()> {
    let stored = json!({"id": "o", "type": "task-card", "color": 0x00b3_b4b4,
        "displayOptions": {"statusLikeCheckbox": true, "fillBackground": false}});
    let patch = ObjectPatch {
        kind_edits: vec![KindEdit::Task(TaskEdit {
            color: Some(0x00e8_726a),
            ..TaskEdit::default()
        })],
        ..ObjectPatch::default()
    };
    let document = applied(&stored, patch)?;
    let fields = fields_of(&document, "o");
    assert_eq!(
        (fields.get("color"), fields.get("displayOptions")),
        (
            Some(&json!(0x00e8_726a)),
            Some(&json!({"statusLikeCheckbox": true, "fillBackground": true}))
        )
    );
    Ok(())
}

#[test]
fn a_palette_token_is_refused_on_a_task_with_the_names() -> Result<()> {
    let said = refusal(&json!({"id": "o", "type": "task-card"}), colored("red4")?);
    assert_eq!(
        said,
        Some(
            "\"red4\" is not a valid color: expected one of gray, green, yellow, orange, red, violet, blue, sky, lime, pink"
                .into()
        )
    );
    Ok(())
}

#[test]
fn the_sets_a_refusal_names_are_the_sets_the_kinds_offer() {
    let said = |names: &[&str]| format!("one of {}", names.join(", "));
    let tasks = task::COLORS.map(|(name, _)| name);
    assert_eq!(
        (
            DOCUMENT_COLORS.to_owned(),
            ICON_COLORS.to_owned(),
            TASK_COLORS.to_owned()
        ),
        (said(&document::COLORS), said(&icon::COLORS), said(&tasks))
    );
}
