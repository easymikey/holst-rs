//! The `workspace` group: list, show, create, rename and the event log.

use std::collections::BTreeSet;

use futures_util::future::join_all;
use holst_macros::CommandArgs;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    command::{
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
        input::InputError,
        lookup::{Candidate, Candidates},
    },
    protocol::workspace::{
        CurrentUser,
        EventsReply,
        JoinableReply,
        Organization,
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
        help = "Also show these parts, comma separated: members, activity, plan, plans, sso, domains, created; none by default"
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
        help = "Confirm a risky change; without it only the dry run is shown (exit code 77)"
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
            The server filters it by the query parameters you give with -q.",
    example = "holst ws log",
    example = "holst ws log easy -q limit=50"
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
        letter = 'q',
        value_name = "key=value",
        help = "A query parameter of the log request, repeatable, e.g. -q limit=50"
    )]
    #[serde(default, deserialize_with = "query_object")]
    pub query: Vec<QueryParameter>,
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
    pub last_active: Option<String>,
}

/// What came of one part: its reply, or why it failed; a part fails alone.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PartRead {
    Members(Vec<Member>),
    Found(Value),
    NotRead { error: String },
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
        api::user::current(client).await.and_then(own_rows)
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
                read.unwrap_or_else(|error| PartRead::NotRead {
                    error: error.to_string(),
                })
            }))
            .collect(),
    })
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
        ShowPart::Activity => api::workspace::last_active_dates(client, workspace_id)
            .await
            .map(|reply| PartRead::Found(only_of(reply, who))),
        ShowPart::Plan => api::subscription::current(client, workspace_id)
            .await
            .map(PartRead::Found),
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
        api::workspace::create_request(&name)?,
        None,
        (args.dry_run, Confirmation::Given),
    )
    .await?;
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
        api::workspace::rename_request(&workspace.id, &name)?,
        Some(unconfirmed),
        (args.dry_run, args.confirm),
    )
    .await?;
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
    let workspace = lookup::workspace(client, args.workspace.as_deref()).await?;
    let own = args.query.iter().any(|each| each.name == "workspaceId");
    let query: Vec<(&str, String)> = (!own)
        .then(|| ("workspaceId", workspace.id.clone()))
        .into_iter()
        .chain(
            args.query
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
