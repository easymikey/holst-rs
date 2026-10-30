//! The `workspace` group: list, show, create, rename and the event log.

use std::collections::BTreeSet;

use futures_util::future::{OptionFuture, join_all};
use holst_macros::CommandArgs;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    command::{
        account,
        change::{ChangeReport, settle},
        lookup,
        table::{
            Command,
            CommandArgs,
            Confirmation,
            EnumArgument,
            query_object,
            visible,
        },
    },
    domain::account::account_workspaces,
    effect::{board_write::WriteMode, request::QueryParameter},
    error::{
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        http::HttpError,
        input::InputError,
        lookup::{Candidate, Candidates},
    },
    protocol::workspace::{
        CurrentUser,
        EventsReply,
        JoinRequestsReply,
        JoinableReply,
        Organization,
        SubscriptionReply,
        WorkspaceReply,
    },
    remote::{api, client::Client},
};

/// The verbs of `holst workspace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum WorkspaceVerb {
    Ls,
    Show,
    Create,
    Rename,
    Log,
}

impl WorkspaceVerb {
    pub(crate) const fn command(self) -> &'static Command {
        match self {
            WorkspaceVerb::Ls => &ListArgs::COMMAND,
            WorkspaceVerb::Show => &ShowArgs::COMMAND,
            WorkspaceVerb::Create => &CreateArgs::COMMAND,
            WorkspaceVerb::Rename => &RenameArgs::COMMAND,
            WorkspaceVerb::Log => &LogArgs::COMMAND,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Workspace,
    verb = "ls",
    risk = Read,
    block = General,
    about = "List your workspaces or the ones you can join",
    help = "List your workspaces, with their organization, or with --joinable the ones you can join.\n\
            Prints one line per workspace with its id; give the id or part of its name in the next command.",
    example = "holst ws ls",
    example = "holst ws ls --joinable"
)]
pub struct ListArgs {
    #[argument(help = "List the workspaces you can join, not your own")]
    pub joinable: bool,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Workspace,
    verb = "show",
    risk = Read,
    block = Targeted,
    about = "Show a workspace and the parts asked for",
    help = "Show a workspace and the parts asked for; the parts are fetched at the same time.\n\
            Without --with it prints the name, id and member count.",
    example = "holst ws show",
    example = "holst ws show easy --with members,activity",
    example = "holst ws show easy --with created --user 1a2b3c4d"
)]
pub struct ShowArgs {
    #[argument(
        positional,
        value_name = "ws",
        help = "The workspace: id or part of its name; your only workspace if left out",
        tool_doc = "Id, @N or part of its name; the only one by default."
    )]
    pub workspace: Option<String>,
    #[argument(
        value_name = "parts",
        help = "Also show these parts, comma separated: members, requests, activity, plan, plans, sso, domains, created; none by default"
    )]
    pub with: Vec<ShowPart>,
    #[argument(
        value_name = "user",
        help = "Only this member in members, activity and created: a user id; everyone by default",
        tool_doc = "created: whose."
    )]
    pub user: Option<String>,
}

/// A part `workspace show` adds to its report.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ShowPart {
    Members,
    Requests,
    Activity,
    Plan,
    Plans,
    Sso,
    Domains,
    Created,
}

impl EnumArgument for ShowPart {}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Workspace,
    verb = "create",
    risk = Write,
    block = General,
    about = "Create a workspace",
    help = "Create a workspace with this name.\n\
            It prints the new workspace with its id.",
    example = "holst ws create \"Design team\"",
    example = "holst ws create \"Design team\" -n"
)]
pub struct CreateArgs {
    #[argument(positional, help = "The name of the new workspace")]
    pub name: String,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Workspace,
    verb = "rename",
    risk = Destroy,
    block = Targeted,
    about = "Rename a workspace for all its members",
    help = "Rename a workspace for all its members.\n\
            Every member sees the new name, so it needs -y; without it only the dry run is shown.",
    example = "holst ws rename easy \"Easy design\" -y",
    example = "holst ws rename Easy \"Easy design\" -n"
)]
pub struct RenameArgs {
    #[argument(
        positional,
        value_name = "ws",
        help = "The workspace: id or part of its name",
        tool_doc = "Id, @N or part of its name; the only one by default."
    )]
    pub workspace: String,
    #[argument(positional, help = "The new name")]
    pub name: String,
    #[argument(
        letter = 'y',
        help = "Confirm a risky change; a terminal asks instead; without one only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show what would change; change nothing")]
    pub dry_run: WriteMode,
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Workspace,
    verb = "log",
    risk = Read,
    block = Targeted,
    about = "Show the workspace event log",
    help = "Show the workspace event log, newest first.\n\
            The server filters it by the query parameters you give with -p.",
    example = "holst ws log",
    example = "holst ws log easy -p limit=50"
)]
pub struct LogArgs {
    #[argument(
        positional,
        value_name = "ws",
        help = "The workspace: id or part of its name; your only workspace if left out",
        tool_doc = "Id, @N or part of its name; the only one by default."
    )]
    pub workspace: Option<String>,
    #[argument(
        letter = 'p',
        value_name = "key=value",
        help = "A query parameter of the log request, repeatable, e.g. -p limit=50"
    )]
    #[serde(default, deserialize_with = "query_object")]
    pub param: Vec<QueryParameter>,
}

/// A workspace of `workspace ls`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub organization: Option<String>,
    pub role: Option<String>,
    /// Known for the joinable ones.
    pub members: Option<u64>,
}

/// A member of a workspace, as `workspace get` lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: Option<String>,
    /// The admin who made the workspace.
    pub owner: bool,
    /// `None` when the reply has no list of licences.
    pub licence: Option<bool>,
    pub last_active: Option<String>,
}

/// A request to join the workspace that waits for an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JoinRequest {
    pub id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub created: Option<String>,
    pub status: Option<String>,
}

/// The plan of a workspace; `Free` when the reply names none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub name: String,
    pub level: Option<u32>,
    pub period: Option<String>,
    /// In cents.
    pub price: Option<u64>,
    pub currency: Option<String>,
    pub trial_days: Option<u32>,
}

impl Plan {
    fn of(reply: SubscriptionReply) -> Self {
        reply
            .subscription
            .and_then(|subscription| {
                subscription.plan.map(|plan| Self {
                    name: plan.name.unwrap_or_default(),
                    level: plan.level,
                    period: plan.period,
                    price: plan.price,
                    currency: plan.currency.or(subscription.currency),
                    trial_days: plan.trial_days,
                })
            })
            .unwrap_or_else(|| Self {
                name: "Free".to_owned(),
                level: None,
                period: None,
                price: None,
                currency: None,
                trial_days: None,
            })
    }

    /// `4.99 USD/month`; `price 499` while the reply names no currency.
    #[must_use]
    pub fn price_text(&self) -> Option<String> {
        let cents = self.price?;
        Some(self.currency.as_ref().map_or_else(
            || format!("price {cents}"),
            |currency| {
                let period = self
                    .period
                    .as_deref()
                    .map(|period| format!("/{}", period.to_lowercase()))
                    .unwrap_or_default();
                format!("{}.{:02} {currency}{period}", cents / 100, cents % 100)
            },
        ))
    }
}

/// What came of one part: its reply, or why it failed; a part fails alone.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PartRead {
    Members(Vec<Member>),
    Requests(Vec<JoinRequest>),
    Plan(Plan),
    Found(Value),
    NotRead {
        error: String,
    },
    /// The plan is only read for those who manage the licence.
    NeedsAdmin {
        error: &'static str,
    },
}

/// A workspace with the parts asked for; a guest's nulls are left out.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct WorkspaceBrief {
    pub id: String,
    pub name: String,
    pub member_count: Option<u64>,
    pub guest_count: Option<u64>,
    pub parts: std::collections::BTreeMap<ShowPart, PartRead>,
}

/// The server's event log page.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct EventLog {
    pub events: Vec<Value>,
    /// Given again as `-q cursor=…` it fetches the rest.
    pub cursor: Option<String>,
}

/// What a workspace change did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Create,
    Rename,
}

/// A created or renamed workspace, or the plan of it.
#[derive(Debug, Serialize)]
pub struct WorkspaceChange {
    pub change: ChangeKind,
    /// `None` for a create that was not sent.
    pub id: Option<String>,
    pub name: String,
    pub was: Option<String>,
    #[serde(flatten)]
    pub report: ChangeReport,
}

impl WorkspaceChange {
    /// The change as a lower-case phrase; `name` writes a sent change's id.
    #[must_use]
    pub fn phrase(&self, name: impl Fn(&str) -> String) -> String {
        let id = self.id.as_deref().map(name).unwrap_or_default();
        let was = self.was.as_deref().unwrap_or_default();
        let new = &self.name;
        match (self.change, &self.report) {
            (ChangeKind::Create, ChangeReport::Sent) => format!("created {new}{id}"),
            (ChangeKind::Create, _) => format!("would create {new}"),
            (ChangeKind::Rename, ChangeReport::Sent) => {
                format!("renamed {was} to {new}{id}")
            }
            (ChangeKind::Rename, _) => format!("would rename {was} to {new}{id}"),
        }
    }
}

/// Your workspaces (or the ones you can join with `joinable`).
///
/// # Errors
/// Errors of the `current_user` or joinable request.
pub async fn list(client: &Client, args: &ListArgs) -> Result<Vec<WorkspaceRow>> {
    if args.joinable {
        api::user::joinable_workspaces(client)
            .await
            .and_then(joinable_rows)
    } else {
        let me = api::user::current(client).await?;
        account::learn(client, &me)?;
        own_rows(me)
    }
}

fn own_rows(me: Value) -> Result<Vec<WorkspaceRow>> {
    let workspaces = account_workspaces(&me);
    let user: CurrentUser = serde_json::from_value(me)?;
    let items: Vec<_> = user
        .workspaces
        .iter()
        .chain(&user.seats)
        .flatten()
        .collect();
    Ok(workspaces
        .into_iter()
        .map(|workspace| {
            let own = || {
                items
                    .iter()
                    .filter(|item| item.workspace() == Some(workspace.id.as_str()))
            };
            WorkspaceRow {
                organization: own()
                    .find_map(|item| item.organization.as_ref()?.name())
                    .map(str::to_owned),
                role: own().find_map(|item| item.role.clone()),
                id: workspace.id,
                name: workspace.name,
                members: None,
            }
        })
        .collect())
}

fn joinable_rows(reply: Value) -> Result<Vec<WorkspaceRow>> {
    let reply: JoinableReply = serde_json::from_value(reply)?;
    Ok(reply
        .into_items()
        .into_iter()
        .filter_map(|item| {
            Some(WorkspaceRow {
                id: item.id?,
                name: item.name.unwrap_or_default(),
                organization: item
                    .organization
                    .as_ref()
                    .and_then(Organization::name)
                    .map(str::to_owned),
                role: None,
                members: item.member_count,
            })
        })
        .collect())
}

/// A workspace and the parts asked for, fetched at the same time. A part that
/// fails is reported in its place; the others stay.
///
/// # Errors
/// Errors of the workspace lookup, of the `workspace get` request, and
/// [`InputError::NotArgumentValue`] for a `user` that is not one member.
pub async fn show(client: &Client, args: &ShowArgs) -> Result<WorkspaceBrief> {
    let workspace = lookup::workspace(client, args.workspace.as_deref()).await?;
    let reply: WorkspaceReply =
        serde_json::from_value(api::workspace::get(client, &workspace.id).await?)?;
    let roster = members_in(&reply);
    let who = args
        .user
        .as_deref()
        .map(|reference| find_member(&roster, reference, "workspace show"))
        .transpose()?;
    let wanted: BTreeSet<ShowPart> = args.with.iter().copied().collect();
    let fetched = join_all(
        wanted
            .iter()
            .map(|part| fetch(client, &workspace.id, &roster, who, *part)),
    )
    .await;
    Ok(WorkspaceBrief {
        id: reply.id.unwrap_or(workspace.id),
        name: reply.name.unwrap_or(workspace.name),
        member_count: reply.member_count,
        guest_count: reply.counts_by_role.and_then(|counts| counts.guest),
        parts: wanted
            .into_iter()
            .zip(fetched.into_iter().map(|read| {
                read.unwrap_or_else(|error| {
                    if refuses_licence_management(&error) {
                        PartRead::NeedsAdmin {
                            error: "needs admin rights",
                        }
                    } else {
                        PartRead::NotRead {
                            error: error.to_string(),
                        }
                    }
                })
            }))
            .collect(),
    })
}

fn refuses_licence_management(error: &crate::Error) -> bool {
    matches!(
        error,
        crate::Error::Http(HttpError::Status { status: 403, detail, .. })
            if serde_json::from_str::<Value>(detail)
                .is_ok_and(|body| body.get("errCode") == Some(&json!("can-not-manage-license")))
    )
}

/// The members a `workspace get` reply names.
///
/// # Errors
/// A reply that is not a workspace.
pub(crate) fn members_of(reply: Value) -> Result<Vec<Member>> {
    Ok(members_in(&serde_json::from_value(reply)?))
}

fn members_in(reply: &WorkspaceReply) -> Vec<Member> {
    let seats = reply.seats.as_deref().unwrap_or_default();
    let licences = reply.subscription_users.as_deref();
    reply
        .users
        .iter()
        .flatten()
        .filter_map(|user| {
            let id = user.id.clone()?;
            let seat = seats
                .iter()
                .find(|seat| seat.user_id.as_deref() == Some(id.as_str()));
            Some(Member {
                name: user.name.clone().or_else(|| user.username.clone()),
                email: user.email.clone(),
                role: seat.and_then(|seat| seat.role.clone()),
                owner: seat.is_some_and(|seat| {
                    seat.role.as_deref() == Some("admin")
                        && seat.role_reason.as_deref() == Some("by_create_workspace")
                }),
                licence: licences.map(|list| {
                    list.iter().any(|each| {
                        each.user_id.as_deref() == Some(id.as_str())
                            && each.has_license == Some(true)
                    })
                }),
                last_active: seat.and_then(|seat| seat.last_active.clone()),
                id,
            })
        })
        .collect()
}

/// The one member `reference` names: a user id (or its start), a name or an
/// email (any part, any case). A miss is worded as an argument `user` of
/// `command`.
pub(crate) fn find_member<'a>(
    members: &'a [Member],
    reference: &str,
    command: &str,
) -> Result<&'a Member> {
    let wanted = reference.trim().to_lowercase();
    let fits = |member: &&Member| {
        [
            Some(&member.id),
            member.name.as_ref(),
            member.email.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|each| {
            let each = each.to_lowercase();
            each.starts_with(&wanted)
                || (each != member.id.to_lowercase() && each.contains(&wanted))
        })
    };
    let matching: Vec<&Member> = members.iter().filter(fits).collect();
    let exact = members.iter().find(|member| member.id == reference.trim());
    match (exact, matching.as_slice()) {
        (Some(member), _) => Ok(member),
        (None, [member]) => Ok(*member),
        (None, []) => Err(not_one_member(
            command,
            format!("a member of the workspace: {}", listed(members.iter())),
        )),
        (None, many) => Err(not_one_member(
            command,
            format!(
                "one member, but \"{reference}\" fits {}",
                listed(many.iter().copied())
            ),
        )),
    }
}

fn listed<'a>(members: impl Iterator<Item = &'a Member>) -> Candidates {
    Candidates::capped(
        members.map(|member| Candidate {
            name: member.name.clone().unwrap_or_else(|| member.id.clone()),
            id: Some(member.id.clone()),
            ..Candidate::default()
        }),
        10,
    )
}

fn not_one_member(command: &str, expected: String) -> crate::Error {
    InputError::NotArgumentValue {
        command: command.to_owned(),
        name: "user",
        expected,
    }
    .into()
}

async fn fetch(
    client: &Client,
    workspace_id: &str,
    roster: &[Member],
    who: Option<&Member>,
    part: ShowPart,
) -> Result<PartRead> {
    let chosen = || {
        roster
            .iter()
            .filter(move |member| who.is_none_or(|only| only.id == member.id))
    };
    match part {
        ShowPart::Members => Ok(PartRead::Members(chosen().cloned().collect())),
        ShowPart::Requests => api::workspace::join_requests(client, workspace_id)
            .await
            .and_then(join_requests)
            .map(PartRead::Requests),
        ShowPart::Activity => api::workspace::last_active_dates(client, workspace_id)
            .await
            .map(|reply| PartRead::Found(only_of(reply, who))),
        ShowPart::Plan => api::subscription::current(client, workspace_id)
            .await
            .and_then(|reply| {
                Ok(PartRead::Plan(Plan::of(serde_json::from_value(reply)?)))
            }),
        ShowPart::Plans => api::subscription::available_plans(client)
            .await
            .map(PartRead::Found),
        ShowPart::Sso => api::workspace::sso_settings(client, workspace_id)
            .await
            .map(PartRead::Found),
        ShowPart::Domains => api::workspace::domains(client, workspace_id)
            .await
            .map(PartRead::Found),
        ShowPart::Created => {
            let targets: Vec<&Member> = chosen().collect();
            let counts = join_all(targets.iter().map(|member| {
                api::workspace::created_channels_count(client, workspace_id, &member.id)
            }))
            .await;
            targets
                .iter()
                .zip(counts)
                .map(|(member, count)| Ok((member.id.clone(), count?)))
                .collect::<Result<Map<String, Value>>>()
                .map(|counts| PartRead::Found(Value::Object(counts)))
        }
    }
}

fn join_requests(reply: Value) -> Result<Vec<JoinRequest>> {
    let reply: JoinRequestsReply = serde_json::from_value(reply)?;
    Ok(reply
        .requests
        .into_iter()
        .flatten()
        .map(|item| JoinRequest {
            id: item.id,
            name: item
                .user
                .as_ref()
                .and_then(|user| user.name.clone().or_else(|| user.username.clone())),
            email: item.user.and_then(|user| user.email),
            created: item.created,
            status: item.status,
        })
        .collect())
}

/// The activity of `who` alone, or all of it.
fn only_of(reply: Value, who: Option<&Member>) -> Value {
    match (reply, who) {
        (Value::Object(entries), Some(member)) => Value::Object(
            entries
                .into_iter()
                .filter(|(id, _)| *id == member.id)
                .collect(),
        ),
        (reply, _) => reply,
    }
}

/// Creates a workspace; with `dry_run` only the plan.
///
/// # Errors
/// [`InputError::NotArgumentValue`] for a blank name; errors of the request.
pub async fn create(client: &Client, args: &CreateArgs) -> Result<WorkspaceChange> {
    let name = visible("workspace create", "name", &args.name)?;
    let (report, reply) = settle(
        client,
        api::workspace::create_request(&api::workspace::WorkspaceName {
            name: name.clone(),
        })?,
        None,
        (args.dry_run, Confirmation::Given),
    )
    .await?;
    matches!(report, ChangeReport::Sent).then(|| client.forget_workspaces());
    let id = reply.and_then(|reply| {
        ["/id", "/workspace/id", "/result/id"]
            .iter()
            .find_map(|path| reply.pointer(path)?.as_str().map(str::to_owned))
    });
    Ok(WorkspaceChange {
        change: ChangeKind::Create,
        id,
        name,
        was: None,
        report,
    })
}

/// Renames a workspace for all its members. Without `confirm` nothing is sent
/// and the change is [`ChangeReport::Held`]; `dry_run` only plans.
///
/// # Errors
/// [`InputError::NotArgumentValue`] for a blank name; errors of the workspace
/// lookup and of the request.
pub async fn rename(client: &Client, args: &RenameArgs) -> Result<WorkspaceChange> {
    let name = visible("workspace rename", "name", &args.name)?;
    let workspace = lookup::workspace(client, Some(args.workspace.as_str())).await?;
    let unconfirmed = Unconfirmed::new(
        format!("renaming {} to {name}", workspace.name),
        ChangeRisk::Everyone,
    );
    let (report, _) = settle(
        client,
        api::workspace::rename_request(
            &workspace.id,
            &api::workspace::WorkspaceName { name: name.clone() },
        )?,
        Some(unconfirmed),
        (args.dry_run, args.confirm),
    )
    .await?;
    matches!(report, ChangeReport::Sent).then(|| client.forget_workspaces());
    Ok(WorkspaceChange {
        change: ChangeKind::Rename,
        id: Some(workspace.id),
        name,
        was: Some(workspace.name),
        report,
    })
}

/// A page of the event log of the workspace, filtered by the server from
/// `query`; a `workspaceId` of the query is left as given.
///
/// # Errors
/// Errors of the workspace lookup and of the request.
pub async fn log(client: &Client, args: &LogArgs) -> Result<EventLog> {
    let own = args.param.iter().any(|each| each.name == "workspaceId");
    let scope = OptionFuture::from(
        (!own).then(|| lookup::workspace(client, args.workspace.as_deref())),
    )
    .await
    .transpose()?;
    let query: Vec<(&str, String)> = scope
        .map(|workspace| ("workspaceId", workspace.id))
        .into_iter()
        .chain(
            args.param
                .iter()
                .map(|each| (each.name.as_str(), each.value.clone())),
        )
        .collect();
    let reply: EventsReply =
        serde_json::from_value(api::workspace::audit_events(client, &query).await?)?;
    let (events, cursor) = reply.into_page();
    Ok(EventLog { events, cursor })
}

#[cfg(test)] mod tests;
