//! What the workspace commands print.

use holst_core::{
    command::workspace::{
        EventLog,
        JoinRequest,
        Member,
        PartRead,
        Plan,
        ShowPart,
        WorkspaceBrief,
        WorkspaceChange,
        WorkspaceRow,
    },
    domain::text::{EMPTY, counted, joined},
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
        row.members.map(|count| counted(count, "member")),
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
        shown.member_count.map(|count| counted(count, "member")),
        shown.guest_count.map(|count| counted(count, "guest")),
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
        PartRead::NeedsAdmin { error } => format!("{name}: {error}"),
        PartRead::Members(members) => listed(members_text(members)),
        PartRead::Requests(rows) => listed(requests_text(rows)),
        PartRead::Plan(plan) => plan_text(plan),
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

fn requests_text(rows: &[JoinRequest]) -> String {
    match rows {
        [] => EMPTY.to_owned(),
        _ => joined(rows.iter().map(request_line), "\n").to_string(),
    }
}

fn request_line(row: &JoinRequest) -> String {
    [
        row.name.clone().or_else(|| row.id.clone()),
        row.email.as_ref().map(|email| format!("<{email}>")),
        row.created.as_ref().map(|when| format!("asked {when}")),
        row.status.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ")
}

fn plan_text(plan: &Plan) -> String {
    [
        Some(format!("plan: {}", plan.name)),
        plan.level.map(|level| format!("level {level}")),
        plan.price_text(),
        plan.trial_days.map(|days| format!("trial {days} days")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ")
}

fn role_text(member: &Member) -> Option<String> {
    member
        .owner
        .then(|| "owner".to_owned())
        .or_else(|| member.role.clone())
        .map(|role| match member.licence {
            Some(true) => format!("{role} · licence"),
            _ => role,
        })
}

fn member_line(member: &Member) -> String {
    [
        Some(member.name.as_ref().unwrap_or(&member.id).clone()),
        member.email.as_ref().map(|email| format!("<{email}>")),
        role_text(member),
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
    let more = log.cursor.as_deref().map(|cursor| {
        format!("… {} shown; next: -p cursor={cursor}", log.events.len())
    });
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
            workspace::{
                ChangeKind,
                EventLog,
                JoinRequest,
                Member,
                Plan,
                WorkspaceChange,
                WorkspaceRow,
            },
        },
        error::confirmation::{ChangeRisk, Unconfirmed},
    };
    use rstest::rstest;

    use crate::commands::workspace::render::{
        listing_text,
        log_text,
        members_text,
        plan_text,
        requests_text,
    };

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
            owner: false,
            licence: None,
            last_active: last_active.map(str::to_owned),
        };
        assert_eq!(
            members_text(&[member(Some("Ann"), Some("t1")), member(None, None)]),
            "Ann  <a@x.io>  admin  active t1\nu1  <a@x.io>  admin"
        );
        assert_eq!(members_text(&[]), "(empty)");
    }

    #[test]
    fn the_owner_and_licence_holders_are_marked() {
        let member = |owner: bool, licence: Option<bool>| Member {
            id: "u1".to_owned(),
            name: Some("Ann".to_owned()),
            email: None,
            role: Some("admin".to_owned()),
            owner,
            licence,
            last_active: None,
        };
        assert_eq!(
            members_text(&[
                member(true, Some(true)),
                member(false, Some(true)),
                member(false, Some(false)),
                member(false, None)
            ]),
            "Ann  owner · licence\nAnn  admin · licence\nAnn  admin\nAnn  admin"
        );
    }

    #[test]
    fn a_request_line_has_who_when_and_status() {
        let row = JoinRequest {
            id: Some("r1".to_owned()),
            name: Some("Zoe".to_owned()),
            email: Some("z@x.io".to_owned()),
            created: Some("t1".to_owned()),
            status: Some("pending".to_owned()),
        };
        assert_eq!(requests_text(&[row]), "Zoe  <z@x.io>  asked t1  pending");
        assert_eq!(requests_text(&[]), "(empty)");
    }

    #[test]
    fn a_plan_without_rights_says_so() {
        let read = holst_core::command::workspace::PartRead::NeedsAdmin {
            error: "needs admin rights",
        };
        assert_eq!(super::part_text(holst_core::command::workspace::ShowPart::Plan, &read), "plan: needs admin rights");
    }

    #[test]
    fn a_plan_is_one_line_and_free_is_just_the_name() {
        let plan = |name: &str, price: Option<u64>, currency: Option<&str>| Plan {
            name: name.to_owned(),
            level: price.map(|_| 5),
            period: Some("Month".to_owned()),
            price,
            currency: currency.map(str::to_owned),
            trial_days: price.map(|_| 14),
        };
        assert_eq!(
            [
                plan_text(&plan("Free", None, None)),
                plan_text(&plan("Team", Some(1200), Some("USD"))),
                plan_text(&plan("Team", Some(1200), None)),
            ],
            [
                "plan: Free",
                "plan: Team  level 5  12.00 USD/month  trial 14 days",
                "plan: Team  level 5  price 1200  trial 14 days",
            ]
        );
    }

    #[test]
    fn a_log_page_ends_with_the_paging_line() {
        let log = EventLog {
            events: vec![serde_json::json!(1), serde_json::json!(2)],
            cursor: Some("2".to_owned()),
        };
        assert_eq!(log_text(&log), "1\n2\n… 2 shown; next: -p cursor=2");
    }
}
