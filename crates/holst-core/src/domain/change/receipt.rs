//! What a board update did: built or accepted, and what it touched; no I/O.

use std::fmt;

use serde::{Serialize, Serializer, ser::SerializeStruct as _};

use crate::domain::change::value::ChangedObject;

/// Whether an update was only built or also accepted by the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Delivery {
    #[default]
    Built,
    Accepted,
}

/// The receipt of one update: only built, or sent and accepted by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateReceipt {
    pub bytes: usize,
    pub id: u64,
    pub delivery: Delivery,
}

/// Keeps `dry_run` and `accepted`, the keys scripts read.
impl Serialize for UpdateReceipt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let accepted = self.delivery == Delivery::Accepted;
        let mut receipt = serializer.serialize_struct("UpdateReceipt", 4)?;
        receipt.serialize_field("dry_run", &!accepted)?;
        receipt.serialize_field("update_bytes", &self.bytes)?;
        receipt.serialize_field("update_id", &self.id)?;
        receipt.serialize_field("accepted", &accepted)?;
        receipt.end()
    }
}

impl fmt::Display for UpdateReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            bytes: update_bytes,
            id: update_id,
            delivery,
        } = self;
        match delivery {
            Delivery::Built => write!(
                f,
                "dry run, nothing sent: update of {update_bytes} bytes built"
            ),
            Delivery::Accepted => write!(
                f,
                "update {update_id} of {update_bytes} bytes accepted by the server"
            ),
        }
    }
}

/// A board change as the CLI and the MCP server report it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeReport {
    pub board: String,
    #[serde(flatten)]
    pub receipt: UpdateReceipt,
    pub objects: Vec<ChangedObject>,
    /// What went less well than asked, e.g. a link without a preview.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub board_status: BoardStatus,
}

/// Where a board sits: among the live boards or in the trash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardStatus {
    Live,
    InTrash,
}

impl fmt::Display for ChangeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.objects.iter().try_for_each(|object| {
            write!(
                f,
                "{} {} {}{}",
                object.change.verb(self.receipt.delivery),
                object.kind.short_name(),
                object.id,
                object.fields_note()
            )?;
            object
                .link
                .as_deref()
                .map_or(Ok(()), |link| write!(f, "  {link}"))?;
            writeln!(f)
        })?;
        write!(f, "{}", self.receipt)
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        board::ObjectKind,
        change::{
            receipt::{BoardStatus, ChangeReport, Delivery, UpdateReceipt},
            value::{ChangedObject, ObjectChangeKind},
        },
    };

    fn outcome(delivery: Delivery) -> ChangeReport {
        let touched = [
            ChangedObject {
                id: "s1".into(),
                kind: ObjectKind::Text,
                change: ObjectChangeKind::Created,
                fields: Vec::new(),
                link: None,
            },
            ChangedObject {
                id: "f1".into(),
                kind: ObjectKind::Frame,
                change: ObjectChangeKind::Changed,
                fields: vec!["text", "position"],
                link: None,
            },
            ChangedObject {
                id: "a1".into(),
                kind: ObjectKind::Arrow,
                change: ObjectChangeKind::Removed,
                fields: Vec::new(),
                link: None,
            },
        ];
        ChangeReport {
            board: "b1".into(),
            receipt: UpdateReceipt {
                bytes: 527,
                id: 42,
                delivery,
            },
            objects: touched
                .iter()
                .map(|touched| touched.with_link(|id| format!("L/{id}")))
                .collect(),
            warnings: Vec::new(),
            board_status: BoardStatus::Live,
        }
    }

    #[test]
    fn the_text_has_a_line_per_object_then_the_fate() {
        insta::assert_snapshot!(outcome(Delivery::Accepted).to_string(), @r"
        created text s1  L/s1
        changed frame f1: text, position  L/f1
        removed arrow a1
        update 42 of 527 bytes accepted by the server
        ");
        insta::assert_snapshot!(outcome(Delivery::Built).to_string(), @r"
        would create text s1  L/s1
        would change frame f1: text, position  L/f1
        would remove arrow a1
        dry run, nothing sent: update of 527 bytes built
        ");
    }

    #[test]
    fn the_warnings_are_not_in_the_result_text() {
        let warned = ChangeReport {
            warnings: vec!["no preview: https://example.com".into()],
            ..outcome(Delivery::Accepted)
        };
        assert_eq!(
            warned.to_string(),
            outcome(Delivery::Accepted).to_string(),
            "the surfaces print the warnings themselves"
        );
    }

    #[test]
    fn the_json_keeps_the_keys_scripts_read() {
        insta::assert_json_snapshot!(outcome(Delivery::Built), @r#"
        {
          "board": "b1",
          "dry_run": true,
          "update_bytes": 527,
          "update_id": 42,
          "accepted": false,
          "objects": [
            {
              "id": "s1",
              "kind": "text",
              "change": "created",
              "fields": [],
              "link": "L/s1"
            },
            {
              "id": "f1",
              "kind": "frame",
              "change": "changed",
              "fields": [
                "text",
                "position"
              ],
              "link": "L/f1"
            },
            {
              "id": "a1",
              "kind": "arrow",
              "change": "removed",
              "fields": [],
              "link": null
            }
          ]
        }
        "#);
    }
}
