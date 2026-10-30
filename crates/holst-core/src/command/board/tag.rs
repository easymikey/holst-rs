//! `board tag`: rename, recolor or remove a tag of a board.

use std::fmt;

use serde::{Serialize, Serializer};

use crate::{
    command::{
        board::{
            TagArgs,
            show::{Loading, board_id, through_cache},
        },
        board_load::CacheUse,
        board_write::{BoardWrite, change_board},
        change::{ChangeReport as Settled, Outcome, gate},
        table::{Confirmation, not_value},
    },
    domain::{
        change::{
            receipt::ChangeReport,
            value::{BoardChange, ObjectChangeKind, TagChange},
        },
        palette::Color,
        text::{counted, joined},
    },
    effect::{
        confirmation::{ALWAYS, NEVER},
        settings::CacheSettings,
    },
    error::{
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
    },
    remote::client::Client,
};

const TAG: &str = "board tag";

/// What was asked of the tag, told by its title.
#[derive(Debug, Serialize)]
pub struct TagLine {
    pub title: String,
    pub change: ObjectChangeKind,
    pub fields: Vec<&'static str>,
}

/// A tag change: the tag, what else it touched, and what it waits for.
#[derive(Debug)]
pub struct Tagged {
    pub tag: TagLine,
    /// The objects the change touched; the tag itself is not an object.
    pub report: ChangeReport,
    /// Whether the change went out, was a dry run, or is held.
    pub outcome: Settled<()>,
}

impl Tagged {
    /// `changed tag "soon": name`, or `would remove tag "urgent"`.
    #[must_use]
    pub fn head(&self) -> String {
        let note = match self.tag.fields.as_slice() {
            [] => String::new(),
            fields => format!(": {}", joined(fields, ", ")),
        };
        format!(
            "{} tag \"{}\"{note}",
            self.tag.change.verb(self.report.receipt.delivery),
            self.tag.title
        )
    }
}

impl fmt::Display for Tagged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.head())?;
        write!(f, "{}", self.report)
    }
}

#[derive(Serialize)]
struct Said<'a> {
    #[serde(flatten)]
    outcome: &'a Settled<()>,
    tag: &'a TagLine,
}

/// Flat JSON: the report of the other objects, the `status`, and the `tag`.
impl Serialize for Tagged {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.report
            .told(Said {
                outcome: &self.outcome,
                tag: &self.tag,
            })
            .serialize(serializer)
    }
}

impl Outcome for Tagged {
    fn refusal(&self) -> Option<&Unconfirmed> {
        self.outcome.refusal()
    }
}

/// Renames, recolors or removes a tag; a removal waits for confirmation.
///
/// # Errors
/// Nothing to change; removal with a name or color; bad palette; missing tag;
/// title taken; board not found or not editable; request or write errors.
pub async fn tag(
    client: &Client,
    cache: &CacheSettings,
    args: &TagArgs,
) -> Result<Tagged> {
    through_cache(client, cache, CacheUse::Use, async |loading| {
        tagged(client, loading, args).await
    })
    .await
}

/// What `args` ask of the tag: a removal stands alone.
fn asked(args: &TagArgs) -> Result<TagChange> {
    let color = args.color.as_deref().map(str::parse::<Color>).transpose()?;
    match (args.remove, args.name.is_some() || color.is_some()) {
        (true, true) => Err(not_value(TAG, "remove", "left out with name and color")),
        (true, false) | (false, _) => Ok(TagChange {
            name: args.name.clone(),
            color,
            remove: args.remove,
        }),
    }
}

async fn tagged(
    client: &Client,
    loading: Loading<'_>,
    args: &TagArgs,
) -> Result<Tagged> {
    let change = asked(args)?;
    let remove = change.remove;
    let renamed = change
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let line = TagLine {
        title: renamed.unwrap_or(&args.tag).to_owned(),
        change: match change {
            TagChange { remove: true, .. } => ObjectChangeKind::Removed,
            TagChange { .. } => ObjectChangeKind::Changed,
        },
        fields: [
            renamed.filter(|_| !remove).map(|_| "name"),
            change.color.as_ref().map(|_| "color"),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        CacheUse::Use,
    )
    .await?;
    let release = match (remove, args.confirm == Confirmation::Given) {
        (true, false) => NEVER,
        (true, true) | (false, _) => ALWAYS,
    };
    let mut report = change_board(
        client,
        loading,
        BoardWrite {
            board: &board,
            change: BoardChange::Tag {
                tag: args.tag.clone(),
                change,
            },
            release,
            guard: None,
            mode: args.dry_run,
        },
    )
    .await?;
    // the domain reports the tag first, then the objects it was taken off
    report.objects = report.objects.into_iter().skip(1).collect();
    let unconfirmed = remove.then(|| {
        Unconfirmed::new(
            format!(
                "removing the tag \"{}\" from {}",
                line.title,
                counted(report.objects.len(), "object")
            ),
            ChangeRisk::Irreversible,
        )
    });
    let outcome = gate(unconfirmed, (args.dry_run, args.confirm), || ())
        .err()
        .unwrap_or(Settled::Sent);
    Ok(Tagged {
        tag: line,
        report,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};

    use crate::{
        Result,
        command::{
            board::tag::{Tagged, tagged},
            fixture::{STICKER, arguments, board_tagged_of, objects, over},
        },
        domain::change::receipt::Delivery,
        error::{Error, input::InputError},
        remote::socket::fake::signed_in,
    };

    /// The test board with the sticker carrying the tag `tag1`, `urgent`.
    fn carrying() -> Vec<Value> {
        objects()
            .into_iter()
            .map(|object| match object.get("id").and_then(Value::as_str) {
                Some(STICKER) => Value::Object(
                    object
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .chain([("tagIds".to_owned(), json!(["tag1"]))])
                        .collect(),
                ),
                _ => object,
            })
            .collect()
    }

    async fn tag(given: Value) -> Result<Tagged> {
        let (_, client) = signed_in([])?;
        let socket = board_tagged_of(&carrying(), &[("tag1", "urgent")]);
        tagged(&client, over(&socket), &arguments(given)?).await
    }

    #[tokio::test]
    async fn a_rename_is_told_by_the_new_title() -> Result<()> {
        let done = tag(json!({"tag": "urgent", "name": "soon"})).await?;
        assert_eq!(done.head(), "changed tag \"soon\": name");
        assert_eq!(done.report.receipt.delivery, Delivery::Accepted);
        assert_eq!(
            serde_json::to_value(&done)?.pointer("/status"),
            Some(&json!("sent"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_report_carries_no_link_for_the_tag() -> Result<()> {
        let done = tag(json!({"tag": "urgent", "color": "red5"})).await?;
        let flat = serde_json::to_value(&done)?;
        assert_eq!(
            (
                flat.pointer("/tag/link"),
                flat.pointer("/objects")
                    .and_then(Value::as_array)
                    .map(Vec::len)
            ),
            (None, Some(0))
        );
        assert_eq!(done.head(), "changed tag \"urgent\": color");
        Ok(())
    }

    #[tokio::test]
    async fn a_removal_waits_for_the_confirmation_and_counts_the_objects() -> Result<()>
    {
        let done = tag(json!({"tag": "urgent", "remove": true})).await?;
        assert_eq!(done.report.receipt.delivery, Delivery::Built);
        assert_eq!(
            serde_json::to_value(&done.outcome)?,
            json!({
                "status": "held",
                "needs": "removing the tag \"urgent\" from 1 object cannot be undone"
            })
        );
        assert_eq!(done.head(), "would remove tag \"urgent\"");
        Ok(())
    }

    #[tokio::test]
    async fn a_confirmed_removal_goes() -> Result<()> {
        let done =
            tag(json!({"tag": "urgent", "remove": true, "confirm": true})).await?;
        assert_eq!(done.report.receipt.delivery, Delivery::Accepted);
        assert_eq!(done.head(), "removed tag \"urgent\"");
        Ok(())
    }

    #[tokio::test]
    async fn a_dry_run_removal_asks_for_no_confirmation() -> Result<()> {
        let done =
            tag(json!({"tag": "urgent", "remove": true, "dry_run": true})).await?;
        assert_eq!(
            serde_json::to_value(&done.outcome)?,
            json!({"status": "dry_run"})
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_removal_beside_a_name_or_color_is_refused() {
        let named = tag(json!({"tag": "urgent", "remove": true, "name": "x"})).await;
        let colored =
            tag(json!({"tag": "urgent", "remove": true, "color": "red5"})).await;
        assert!(matches!(
            (named, colored),
            (
                Err(Error::Input(InputError::NotArgumentValue {
                    name: "remove",
                    ..
                })),
                Err(Error::Input(InputError::NotArgumentValue {
                    name: "remove",
                    ..
                }))
            )
        ));
    }
}
