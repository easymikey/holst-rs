//! Replies and bodies of the server that are shown as they came.

use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::{Map, Value};

/// The fields of a body that nothing names; shells show them as they came.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OtherFields(Map<String, Value>);

impl OtherFields {
    /// The text of a field, if the body has it as text.
    #[must_use]
    pub fn text(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str)
    }
}

impl From<Map<String, Value>> for OtherFields {
    fn from(fields: Map<String, Value>) -> Self {
        Self(fields)
    }
}

/// The answer to a change; its body is not read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "IgnoredAny")]
pub struct IgnoredBody;

impl From<IgnoredAny> for IgnoredBody {
    fn from(_: IgnoredAny) -> Self {
        Self
    }
}

/// The answer to an upload; any object is taken, the name's key is unverified.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uploaded {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub other: OtherFields,
}

/// What the page service reports of a file: pages and whether they show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagesInfo {
    pub count: u32,
    pub valid: bool,
}

/// What a site offers to embed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmbedInfo {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
}

/// The answer to a link preview: what a card shows of the page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LinkPreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_key: Option<String>,
    #[serde(rename = "embedInfo", skip_serializing_if = "Option::is_none")]
    pub embed: Option<Box<EmbedInfo>>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::domain::response::{EmbedInfo, IgnoredBody, LinkPreview, Uploaded};

    #[test]
    fn a_link_preview_reads_the_wire_names_and_tolerates_missing_fields() {
        let reply = json!({
            "title": "YouTube", "faviconKey": "a.ico",
            "embedInfo": { "type": "video", "providerName": "YouTube", "width": 200 },
        });
        assert_eq!(
            serde_json::from_value::<LinkPreview>(reply).ok(),
            Some(LinkPreview {
                title: Some("YouTube".into()),
                favicon_key: Some("a.ico".into()),
                embed: Some(Box::new(EmbedInfo {
                    kind: Some("video".into()),
                    provider_name: Some("YouTube".into()),
                    width: Some(200.0),
                    ..EmbedInfo::default()
                })),
                ..LinkPreview::default()
            })
        );
        assert_eq!(
            serde_json::from_value::<LinkPreview>(json!({})).ok(),
            Some(LinkPreview::default())
        );
    }

    #[test]
    fn upload_reply_keeps_name_and_other_fields() {
        let reply = json!({ "name": "a1b2.png", "size": 4 });
        let uploaded = serde_json::from_value::<Uploaded>(reply.clone());
        assert_eq!(
            uploaded.as_ref().ok().and_then(|u| u.name.as_deref()),
            Some("a1b2.png")
        );
        assert_eq!(
            uploaded.ok().and_then(|u| serde_json::to_value(u).ok()),
            Some(reply)
        );
    }

    #[test]
    fn upload_reply_without_a_name_is_shown_as_it_came() {
        let reply = json!({ "fileName": "a1b2.png" });
        let uploaded = serde_json::from_value::<Uploaded>(reply.clone());
        assert_eq!(
            uploaded.ok().and_then(|u| serde_json::to_value(u).ok()),
            Some(reply)
        );
    }

    #[test]
    fn receipt_accepts_any_body() {
        let read = |body: serde_json::Value| {
            serde_json::from_value::<IgnoredBody>(body).is_ok()
        };
        assert!(
            read(json!(null)) && read(json!({ "ok": true })) && read(json!("done"))
        );
    }
}
