//! What the workspace commands print.

use holst_core::{
    command::workspace::{
        EventLog,
        Member,
        PartRead,
        ShowPart,
        WorkspaceBrief,
        WorkspaceChange,
        WorkspaceRow,
    },
    domain::text::{EMPTY, joined},
};
use serde_json::Value;

use crate::{
    error::Result,
    output::{self, OutputFormat},
};

pub(super) fn listing(format: OutputFormat, rows: &[WorkspaceRow]) -> Result<()> {
    output::print(format, &listing_text(rows), &rows)
}

pub(super) fn shown(format: OutputFormat, shown: &WorkspaceBrief) -> Result<()> {
    output::print(format, &shown_text(shown), shown)
}

pub(super) fn change(format: OutputFormat, change: &WorkspaceChange) -> Result<()> {
    output::print(format, &change.phrase(|id| format!(" ({id})")), change)
}

pub(super) fn log(format: OutputFormat, log: &EventLog) -> Result<()> {
    output::print(format, &log_text(log), log)
}

fn listing_text(rows: &[WorkspaceRow]) -> String {
    if rows.is_empty() {
        EMPTY.to_owned()
    } else {
        joined(rows.iter().map(row_line), "\n").to_string()
    }
}

fn row_line(row: &WorkspaceRow) -> String {
    let details: Vec<String> = [
        row.organization.clone(),
        row.role.clone(),
        row.members.map(|count| format!("{count} members")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if details.is_empty() {
        format!("{}  {}", row.id, row.name)
    } else {
        format!("{}  {}  ({})", row.id, row.name, details.join(", "))
    }
}

fn shown_text(shown: &WorkspaceBrief) -> String {
    let counts: Vec<String> = [
        shown.member_count.map(|count| format!("{count} members")),
        shown.guest_count.map(|count| format!("{count} guests")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let head = if counts.is_empty() {
        format!("{} ({})", shown.name, shown.id)
    } else {
        format!("{} ({}): {}", shown.name, shown.id, counts.join(", "))
    };
    joined(
        std::iter::once(head).chain(
            shown
                .parts
                .iter()
                .map(|(part, fetched)| part_text(*part, fetched)),
        ),
        "\n",
    )
    .to_string()
}

fn part_text(part: ShowPart, fetched: &PartRead) -> String {
    let name = format!("{part:?}").to_lowercase();
    let listed =
        |body: String| format!("{name}:\n{}", joined(body.lines().map(indented), "\n"));
    match fetched {
        PartRead::NotRead { error } => format!("{name}: could not be read: {error}"),
        PartRead::Members(members) => listed(members_text(members)),
        PartRead::Found(value) => listed(format!("{value:#}")),
    }
}

fn indented(line: &str) -> String {
    format!("  {line}")
}

fn members_text(members: &[Member]) -> String {
    if members.is_empty() {
        EMPTY.to_owned()
    } else {
        joined(members.iter().map(member_line), "\n").to_string()
    }
}

fn member_line(member: &Member) -> String {
    [
        Some(member.name.as_ref().unwrap_or(&member.id).clone()),
        member.email.as_ref().map(|email| format!("<{email}>")),
        member.role.clone(),
        member
            .last_active
            .as_ref()
            .map(|when| format!("active {when}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ")
}

fn log_text(log: &EventLog) -> String {
    let more = log
        .cursor
        .as_deref()
        .map(|cursor| format!("more: -q cursor={cursor}"));
    let lines: Vec<String> = log
        .events
        .iter()
        .map(Value::to_string)
        .chain(more)
        .collect();
    if lines.is_empty() {
        EMPTY.to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use holst_core::{
        command::{
            change::{ChangeReport, PlannedRequest},
            workspace::{ChangeKind, Member, WorkspaceChange, WorkspaceRow},
        },
        error::confirmation::{ChangeRisk, Unconfirmed},
    };
    use rstest::rstest;

    use crate::commands::workspace::render::{listing_text, members_text};

    fn planned() -> PlannedRequest {
        PlannedRequest {
            method: "Patch",
            path: "workspace/w1".to_owned(),
            query: Vec::new(),
            body: None,
        }
    }

    fn dry_run() -> ChangeReport {
        ChangeReport::DryRun { request: planned() }
    }

    fn held() -> ChangeReport {
        ChangeReport::Held {
            request: planned(),
            needs: String::new(),
            refusal: Unconfirmed::new("renaming", ChangeRisk::Everyone),
        }
    }

    fn change(kind: ChangeKind, report: ChangeReport) -> WorkspaceChange {
        WorkspaceChange {
            change: kind,
            id: Some("w1".to_owned()),
            name: "Crew".to_owned(),
            was: Some("Easy".to_owned()),
            report,
        }
    }

    #[rstest]
    #[case::created(ChangeKind::Create, ChangeReport::Sent, "created Crew (w1)")]
    #[case::planned(ChangeKind::Create, dry_run(), "would create Crew")]
    #[case::renamed(
        ChangeKind::Rename,
        ChangeReport::Sent,
        "renamed Easy to Crew (w1)"
    )]
    #[case::held(ChangeKind::Rename, held(), "would rename Easy to Crew (w1)")]
    fn changes_are_told_in_lower_case(
        #[case] kind: ChangeKind,
        #[case] report: ChangeReport,
        #[case] text: &str,
    ) {
        assert_eq!(change(kind, report).phrase(|id| format!(" ({id})")), text);
    }

    #[test]
    fn a_listing_has_a_line_per_workspace() {
        let row = |id: &str, role: Option<&str>| WorkspaceRow {
            id: id.to_owned(),
            name: "Team".to_owned(),
            organization: None,
            role: role.map(str::to_owned),
            members: None,
        };
        assert_eq!(
            listing_text(&[row("w1", Some("owner")), row("w2", None)]),
            "w1  Team  (owner)\nw2  Team"
        );
        assert_eq!(listing_text(&[]), "(empty)");
    }

    #[test]
    fn a_member_line_has_what_the_member_has() {
        let member = |name: Option<&str>, last_active: Option<&str>| Member {
            id: "u1".to_owned(),
            name: name.map(str::to_owned),
            email: Some("a@x.io".to_owned()),
            role: Some("admin".to_owned()),
            last_active: last_active.map(str::to_owned),
        };
        assert_eq!(
            members_text(&[member(Some("Ann"), Some("t1")), member(None, None)]),
            "Ann  <a@x.io>  admin  active t1\nu1  <a@x.io>  admin"
        );
        assert_eq!(members_text(&[]), "(empty)");
    }
}
