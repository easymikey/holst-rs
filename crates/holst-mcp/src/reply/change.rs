//! The reply to a REST change: `verb: name @N`, a board's link under it.

use holst_core::domain::{
    change::{
        receipt::{BoardStatus, ChangeReport, Delivery},
        value::ObjectChangeKind,
    },
    text::{joined, one_line},
};

pub(crate) fn written(
    report: &ChangeReport,
    alias: &impl Fn(&str) -> String,
) -> String {
    let delivery = report.receipt.delivery;
    let (first, rest) = report
        .objects
        .split_first()
        .map_or((None, &[][..]), |(first, rest)| (Some(first), rest));
    let head = first.map(|object| {
        let link = object
            .link
            .as_deref()
            .map(|link| format!(" {link}"))
            .unwrap_or_default();
        format!(
            "{} {} {}{}{link}\n",
            object.change.verb(delivery),
            object.kind.short_name(),
            alias(&object.id),
            object.fields_note()
        )
    });
    let groups = rest.iter().fold(
        Vec::<((ObjectChangeKind, &str), usize)>::new(),
        |mut groups, object| {
            let key = (object.change, object.kind.short_name());
            match groups.iter_mut().find(|(seen, _)| *seen == key) {
                Some((_, count)) => *count += 1,
                None => groups.push((key, 1)),
            }
            groups
        },
    );
    let also = groups.iter().map(|((effect, kind), count)| {
        format!("{} {count} {kind}", effect.verb(delivery))
    });
    let also = Some(joined(also, ", ").to_string())
        .filter(|also| !also.is_empty())
        .map(|also| format!("also: {also}\n"));
    let trash = matches!(report.board_status, BoardStatus::InTrash).then(|| {
        format!(
            "warning: the board is in the trash; board_write action=restore board={} brings it \
             back\n",
            alias(&report.board)
        )
    });
    let footer = match delivery {
        Delivery::Accepted => {
            format!("update {} accepted by the server\n", report.receipt.id)
        }
        Delivery::Built => "dry run, nothing sent\n".to_owned(),
    };
    let warnings = report
        .warnings
        .iter()
        .map(|warning| format!("warning: {}\n", one_line(warning)));
    [head, also, trash, Some(footer)]
        .into_iter()
        .flatten()
        .chain(warnings)
        .collect()
}

#[cfg(test)]
mod tests {
    use holst_core::domain::{
        board::ObjectKind,
        change::{
            receipt::{BoardStatus, ChangeReport},
            value::ObjectChangeKind,
        },
    };

    use crate::reply::change::written;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Delivery {
        Confirmed,
        Unconfirmed,
    }

    fn outcome(delivery: Delivery, placement: BoardStatus) -> ChangeReport {
        let touched = |id: &str, kind, effect, fields: &[&'static str]| {
            holst_core::domain::change::value::ChangedObject {
                id: id.into(),
                kind,
                change: effect,
                fields: fields.to_vec(),
                link: None,
            }
            .with_link(|touched_id| format!("L/{touched_id}"))
        };
        ChangeReport {
            board: "b1".into(),
            receipt: holst_core::domain::change::receipt::UpdateReceipt {
                bytes: 527,
                id: 42,
                delivery: match delivery {
                    Delivery::Confirmed => {
                        holst_core::domain::change::receipt::Delivery::Accepted
                    }
                    Delivery::Unconfirmed => {
                        holst_core::domain::change::receipt::Delivery::Built
                    }
                },
            },
            objects: vec![
                touched("t1", ObjectKind::Text, ObjectChangeKind::Created, &[]),
                touched(
                    "f1",
                    ObjectKind::Frame,
                    ObjectChangeKind::Changed,
                    &["position"],
                ),
                touched(
                    "s1",
                    ObjectKind::Sticker,
                    ObjectChangeKind::Changed,
                    &["position"],
                ),
                touched(
                    "s2",
                    ObjectKind::Sticker,
                    ObjectChangeKind::Changed,
                    &["position"],
                ),
                touched("a1", ObjectKind::Arrow, ObjectChangeKind::Removed, &[]),
            ],
            warnings: Vec::new(),
            board_status: placement,
        }
    }

    #[test]
    fn the_model_reads_one_outcome() {
        let alias = |id: &str| format!("@{id}");
        let sent = outcome(Delivery::Confirmed, BoardStatus::Live);
        insta::assert_snapshot!("outcome_mcp_sent", written(&sent, &alias));
        let dry = outcome(Delivery::Unconfirmed, BoardStatus::InTrash);
        insta::assert_snapshot!("outcome_mcp_dry", written(&dry, &alias));
    }

    #[test]
    fn a_warning_of_the_report_ends_the_reply() {
        let alias = |id: &str| format!("@{id}");
        let report = ChangeReport {
            warnings: vec!["no preview: https://example.com".into()],
            ..outcome(Delivery::Confirmed, BoardStatus::Live)
        };
        let text = written(&report, &alias);
        assert!(
            text.ends_with("\nwarning: no preview: https://example.com\n"),
            "{text}"
        );
    }
}
