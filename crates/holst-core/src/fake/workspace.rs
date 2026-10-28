//! The workspaces: members, roles, plan, settings pages and the event log.

use std::iter;

use serde_json::{Value, json};

use crate::fake::{
    fixture::{FixtureError, unknown},
    http::{Parts, Request, Response, found, object_body, refused},
};

const USER_ID: &str = "00000000-0000-4000-8000-000000000000";
const USER_NAME: &str = "Fake User";
const FREE_PLAN: &str = "Free";
const PLANS: [&str; 4] = [FREE_PLAN, "Team", "Business", "Enterprise"];
const PAGE_SIZE: usize = 50;

/// What a user is in a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
    Guest,
}

impl WorkspaceRole {
    /// The server has no owner role: the owner is an admin who made the
    /// workspace.
    fn wire(self) -> &'static str {
        match self {
            Self::Owner | Self::Admin => "admin",
            Self::Member => "member",
            Self::Guest => "guest",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Owner => "by_create_workspace",
            Self::Admin | Self::Member | Self::Guest => "by_invite",
        }
    }

    fn manages(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

/// One thing a fixture says about a workspace.
#[derive(Debug, Clone)]
pub(crate) enum Detail {
    Organization(String),
    Joinable,
    Member(String, WorkspaceRole),
    UserIs(WorkspaceRole),
    Domain(String),
    Sso,
    JoinRequest(String),
    Plan(String),
    Billing(String),
}

/// An entry of the event log a fixture adds.
#[derive(Debug)]
pub(crate) struct FixtureEvent {
    pub(crate) workspace: String,
    pub(crate) user: String,
    pub(crate) action: String,
    pub(crate) target: String,
}

#[derive(Debug)]
struct Person {
    id: String,
    name: String,
    email: String,
}

impl Person {
    fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            email: format!("{}@example.test", name.to_lowercase().replace(' ', ".")),
        }
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "username": self.name,
            "name": self.name,
            "avatar": null,
            "is_anonymous": false,
            "email": self.email,
        })
    }
}

#[derive(Debug)]
struct Entry {
    id: String,
    name: String,
    organization: Option<String>,
    joinable: bool,
    role: WorkspaceRole,
    members: Vec<(usize, WorkspaceRole)>,
    domains: Vec<String>,
    sso: bool,
    requests: Vec<String>,
    plan: String,
    billing: Option<String>,
}

impl Entry {
    fn new(index: usize, name: &str) -> Self {
        Self {
            id: format!("00000000-0000-4000-8000-{:012}", index + 1),
            name: name.to_owned(),
            organization: None,
            joinable: false,
            role: WorkspaceRole::Owner,
            members: Vec::new(),
            domains: Vec::new(),
            sso: false,
            requests: Vec::new(),
            plan: FREE_PLAN.to_owned(),
            billing: None,
        }
    }

    fn apply(&mut self, detail: &Detail, person: Option<usize>) {
        match detail {
            Detail::Organization(name) => self.organization = Some(name.clone()),
            Detail::Joinable => self.joinable = true,
            Detail::Member(_, role) => {
                self.members.extend(person.map(|index| (index, *role)));
            }
            Detail::UserIs(role) => self.role = *role,
            Detail::Domain(domain) => self.domains.push(domain.clone()),
            Detail::Sso => self.sso = true,
            Detail::JoinRequest(name) => self.requests.push(name.clone()),
            Detail::Plan(name) => self.plan.clone_from(name),
            Detail::Billing(company) => self.billing = Some(company.clone()),
        }
    }

    fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "organization": self.organization.as_deref().map(organization),
            "role": self.role.wire(),
        })
    }
}

fn organization(name: &str) -> Value {
    json!({ "id": format!("org-{}", name.to_lowercase().replace(' ', "-")), "name": name })
}

#[derive(Debug)]
struct Event {
    workspace: String,
    user: String,
    action: String,
    target: String,
}

impl Event {
    fn json(&self, index: usize) -> Value {
        json!({
            "id": format!("event-{index}"),
            "createdAt": moment(index),
            "workspaceId": self.workspace,
            "user": self.user,
            "action": self.action,
            "target": self.target,
        })
    }

    fn holds(&self, key: &str, value: &str) -> bool {
        match key {
            "workspaceId" => self.workspace == value,
            "user" => self.user == value,
            "action" => self.action == value,
            "target" => self.target == value,
            _ => false,
        }
    }
}

/// Counts every number a page of the workspace shows.
struct Counts {
    full: usize,
    admins: usize,
    members: usize,
    guests: usize,
}

impl Counts {
    fn of(people: &[(&Person, WorkspaceRole)]) -> Self {
        let count = |wanted: fn(WorkspaceRole) -> bool| {
            people.iter().filter(|(_, role)| wanted(*role)).count()
        };
        Self {
            full: count(|role| role != WorkspaceRole::Guest),
            admins: count(WorkspaceRole::manages),
            members: count(|role| role == WorkspaceRole::Member),
            guests: count(|role| role == WorkspaceRole::Guest),
        }
    }
}

/// The time of the `minutes`th minute of the fake's calendar.
fn moment(minutes: usize) -> String {
    format!(
        "2026-03-01T{:02}:{:02}:00Z",
        minutes / 60 % 24,
        minutes % 60
    )
}

fn plan_id(name: &str) -> String {
    format!("plan-{}", name.to_lowercase())
}

fn plan_json(name: &str) -> Value {
    json!({ "id": plan_id(name), "name": name })
}

/// The signed-in user, the other people and the workspaces they share.
#[derive(Debug)]
pub(crate) struct Directory {
    entries: Vec<Entry>,
    people: Vec<Person>,
    events: Vec<Event>,
}

impl Directory {
    pub(crate) fn fixture(
        names: &[String],
        details: &[(String, Detail)],
        events: &[FixtureEvent],
    ) -> Result<Self, FixtureError> {
        let start = Self {
            entries: names
                .iter()
                .enumerate()
                .map(|(index, name)| Entry::new(index, name))
                .collect(),
            people: vec![Person::new(USER_ID, USER_NAME)],
            events: Vec::new(),
        };
        let described = details
            .iter()
            .try_fold(start, |directory, (workspace, detail)| {
                directory.with(workspace, detail)
            })?;
        events.iter().try_fold(described, Self::with_event)
    }

    pub(crate) fn ids(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|entry| !entry.joinable)
            .map(|entry| entry.id.as_str())
            .collect()
    }

    /// Returns everyone as a pair of name and id.
    pub(crate) fn people(&self) -> Vec<(String, String)> {
        self.people
            .iter()
            .map(|person| (person.name.clone(), person.id.clone()))
            .collect()
    }

    /// Returns every workspace as a pair of its name and id.
    pub(crate) fn named(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.id.clone()))
            .collect()
    }

    pub(crate) fn id_of(&self, name: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.id.clone())
    }

    pub(crate) fn current_user(&self) -> Value {
        json!({
            "id": USER_ID,
            "name": USER_NAME,
            "email": self.people.first().map(|person| person.email.as_str()),
            "workspaces": self
                .entries
                .iter()
                .filter(|entry| !entry.joinable)
                .map(Entry::summary)
                .collect::<Vec<_>>(),
        })
    }

    pub(crate) fn respond(&mut self, request: &Request) -> Option<Response> {
        let parts = Parts::of(request);
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "current_user", "joinable_workspaces"]) => {
                Some(found(&self.joinable()))
            }
            ("POST", ["", "gapi", "workspace", "create"]) => Some(self.create(request)),
            ("PATCH", ["", "gapi", "workspace", id]) => Some(self.rename(id, request)),
            ("GET", ["", "gapi", "audit_events"]) => Some(found(&self.log(&parts))),
            _ => self.page(&parts).or_else(|| self.subscription(&parts)),
        }
    }

    fn page(&self, parts: &Parts<'_>) -> Option<Response> {
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "workspace", id]) => {
                Some(self.shown(id, |entry| self.detail(entry)))
            }
            ("GET", ["", "gapi", "workspace_user", id, "count_info"]) => {
                Some(self.shown(id, |entry| self.count_info(entry)))
            }
            ("GET", ["", "gapi", "workspace_user", id, "list_last_active_dates"]) => {
                Some(self.shown(id, |entry| self.last_active(entry)))
            }
            ("GET", ["", "gapi", "workspace", id, "sso_settings"]) => {
                Some(self.shown(id, |entry| json!({ "enabled": entry.sso })))
            }
            ("GET", ["", "gapi", "workspace_domains", id, "list"]) => {
                Some(self.shown(id, |entry| {
                    json!({ "domains": entry.domains.iter().map(
                        |domain| json!({ "domain": domain, "status": "verified" })
                    ).collect::<Vec<_>>() })
                }))
            }
            ("GET", ["", "gapi", "workspace", id, "join_request"]) => {
                Some(self.shown(id, |entry| {
                    json!({ "joinRequests": entry.requests.iter().enumerate().map(
                        |(index, name)| json!({ "id": format!("request-{index}"), "name": name })
                    ).collect::<Vec<_>>() })
                }))
            }
            _ => None,
        }
    }

    fn subscription(&self, parts: &Parts<'_>) -> Option<Response> {
        match (parts.method, parts.segments.as_slice()) {
            ("GET", ["", "gapi", "subscription", "get_available_plans"]) => {
                Some(found(&json!({ "plans": PLANS.map(plan_json) })))
            }
            ("GET", ["", "gapi", "subscription", "get_plan", wanted]) => {
                Some(PLANS.iter().find(|name| plan_id(name) == *wanted).map_or_else(
                    || refused(404, "not-found"),
                    |name| found(&plan_json(name)),
                ))
            }
            ("GET", ["", "gapi", "subscription", "get_billing_details", id]) => {
                Some(self.shown(id, |entry| json!({ "billingDetails": billing(entry) })))
            }
            (
                "GET",
                ["", "gapi", "subscription", "get_subscription_with_billing_details", id],
            ) => Some(self.shown(id, |entry| {
                json!({
                    "billingDetails": billing(entry),
                    "brotherPlan": null,
                    "subscription": { "workspace_id": entry.id, "plan": plan_json(&entry.plan) },
                })
            })),
            _ => None,
        }
    }

    /// Answers with what `show` makes of the workspace `id`, when the user
    /// belongs to it.
    fn shown(&self, id: &str, show: impl FnOnce(&Entry) -> Value) -> Response {
        self.entries
            .iter()
            .find(|entry| entry.id == id && !entry.joinable)
            .map_or_else(|| refused(404, "not-found"), |entry| found(&show(entry)))
    }

    fn joinable(&self) -> Value {
        json!({
            "workspaces": self
                .entries
                .iter()
                .filter(|entry| entry.joinable)
                .map(Entry::summary)
                .collect::<Vec<_>>(),
        })
    }

    fn create(&mut self, request: &Request) -> Response {
        match name_in(request) {
            Some(name) => {
                let entry = Entry::new(self.entries.len(), &name);
                let reply = json!({ "id": entry.id, "name": entry.name });
                self.entries.push(entry);
                found(&reply)
            }
            None => refused(400, "bad-request"),
        }
    }

    /// Renames the workspace `id`; only the owner and the admins may.
    fn rename(&mut self, id: &str, request: &Request) -> Response {
        match (
            self.entries
                .iter_mut()
                .find(|entry| entry.id == id && !entry.joinable),
            name_in(request),
        ) {
            (None, _) => refused(404, "not-found"),
            (Some(entry), _) if !entry.role.manages() => refused(403, "forbidden"),
            (Some(entry), Some(name)) => {
                entry.name = name;
                found(&json!({ "id": entry.id, "name": entry.name }))
            }
            (Some(_), None) => refused(400, "bad-request"),
        }
    }

    /// The user first, then the members in the order they were filled.
    fn people_of(&self, entry: &Entry) -> Vec<(&Person, WorkspaceRole)> {
        iter::once((0, entry.role))
            .chain(entry.members.iter().copied())
            .filter_map(|(index, role)| {
                self.people.get(index).map(|person| (person, role))
            })
            .collect()
    }

    fn detail(&self, entry: &Entry) -> Value {
        let people = self.people_of(entry);
        let counts = Counts::of(&people);
        json!({
            "id": entry.id,
            "name": entry.name,
            "owner_user_id": people
                .iter()
                .find(|(_, role)| *role == WorkspaceRole::Owner)
                .map(|(person, _)| person.id.as_str()),
            "logo": null,
            "invites_enabled": true,
            "members_count": counts.full,
            "users_count_by_role": {
                "admin": counts.admins,
                "member": counts.members,
                "guest": counts.guests,
            },
            "users": people.iter().map(|(person, _)| person.json()).collect::<Vec<_>>(),
            "workspace_users": people
                .iter()
                .enumerate()
                .map(|(position, (person, role))| json!({
                    "workspace_id": entry.id,
                    "user_id": person.id,
                    "role": role.wire(),
                    "role_reason": role.reason(),
                    "updated_at": moment(position),
                    "last_active_at": moment(position),
                }))
                .collect::<Vec<_>>(),
            "subscription_users": [],
        })
    }

    fn count_info(&self, entry: &Entry) -> Value {
        let counts = Counts::of(&self.people_of(entry));
        json!({
            "fullMembers": counts.full,
            "admins": counts.admins,
            "guests": counts.guests,
        })
    }

    fn last_active(&self, entry: &Entry) -> Value {
        Value::Object(
            self.people_of(entry)
                .iter()
                .enumerate()
                .map(|(position, (person, _))| {
                    (person.id.clone(), Value::String(moment(position)))
                })
                .collect(),
        )
    }

    /// The events newest first, the ones that hold every `KEY=VALUE` filter
    /// of the query, a page of `pageSize` from the `cursor` on.
    fn log(&self, parts: &Parts<'_>) -> Value {
        let number = |name: &str| parts.query(name).and_then(|text| text.parse().ok());
        let size = number("pageSize").unwrap_or(PAGE_SIZE).max(1);
        let start = number("cursor").unwrap_or(0);
        let filters: Vec<&(String, String)> = parts
            .query
            .iter()
            .filter(|(key, _)| !["pageSize", "cursor"].contains(&key.as_str()))
            .collect();
        let matching: Vec<(usize, &Event)> = self
            .events
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, event)| {
                filters.iter().all(|(key, value)| event.holds(key, value))
            })
            .skip(start)
            .collect();
        json!({
            "events": matching
                .iter()
                .take(size)
                .map(|(index, event)| event.json(*index))
                .collect::<Vec<_>>(),
            "cursor": (matching.len() > size).then(|| (start + size).to_string()),
        })
    }

    fn with(mut self, workspace: &str, detail: &Detail) -> Result<Self, FixtureError> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.name == workspace)
            .ok_or_else(|| unknown("workspace", workspace))?;
        let person = match detail {
            Detail::Member(name, _) => Some(self.person_index(name)),
            Detail::Organization(_)
            | Detail::Joinable
            | Detail::UserIs(_)
            | Detail::Domain(_)
            | Detail::Sso
            | Detail::JoinRequest(_)
            | Detail::Plan(_)
            | Detail::Billing(_) => None,
        };
        self.entries
            .get_mut(index)
            .into_iter()
            .for_each(|entry| entry.apply(detail, person));
        Ok(self)
    }

    fn with_event(mut self, event: &FixtureEvent) -> Result<Self, FixtureError> {
        let workspace = self
            .id_of(&event.workspace)
            .ok_or_else(|| unknown("workspace", &event.workspace))?;
        self.events.push(Event {
            workspace,
            user: event.user.clone(),
            action: event.action.clone(),
            target: event.target.clone(),
        });
        Ok(self)
    }

    /// Returns the place of the person called `name`, adding them when new.
    fn person_index(&mut self, name: &str) -> usize {
        self.people
            .iter()
            .position(|person| person.name == name)
            .unwrap_or_else(|| {
                let index = self.people.len();
                let id = format!("00000000-0000-4000-9000-{index:012}");
                self.people.push(Person::new(&id, name));
                index
            })
    }
}

fn billing(entry: &Entry) -> Value {
    entry
        .billing
        .as_ref()
        .map_or(Value::Null, |company| json!({ "company": company }))
}

/// The non-empty `name` a request carries.
fn name_in(request: &Request) -> Option<String> {
    object_body(request)
        .and_then(|body| body.get("name").and_then(Value::as_str).map(str::to_owned))
        .filter(|name| !name.is_empty())
}
