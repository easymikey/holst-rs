//! `board grant revoke share transfer`: who may use a board.

use futures_util::{TryStreamExt as _, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    command::{
        account,
        board::{GrantArgs, RevokeArgs, ShareArgs, TransferArgs},
        change::{ChangeReport, PlannedRequest, settle},
        lookup,
        table::{ArgumentValue, EnumArgument, ValueKind},
        workspace::{Member, find_member, members_of},
    },
    domain::{node::NodeBrief, sharing::SubtreeTransfer, text::printable},
    effect::request::ApiRequest,
    error::{
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
    },
    protocol::sharing::{self, HeldRole, SharingPatch, WireRole},
    remote::{api, client::Client},
};

/// A right on a board, on the one scale every command uses.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Right {
    None,
    Viewer,
    Commentator,
    Editor,
    Full,
}

// A choice on the command line, text in the MCP schema.
impl ArgumentValue for Right {
    const KIND: ValueKind = ValueKind::Scale(<Self as strum::VariantNames>::VARIANTS);
}

impl Right {
    /// The server's value: `null` for no right.
    #[must_use]
    pub const fn wire(self) -> Option<&'static str> {
        match self {
            Right::None => None,
            Right::Viewer => Some("viewer"),
            Right::Commentator => Some("commentator"),
            Right::Editor => Some("editor"),
            Right::Full => Some("co_owner"),
        }
    }

    /// `self` when `allowed` has it; else the argument `name` of `command`
    /// refused, with the rights it takes.
    fn among(
        self,
        allowed: &[Right],
        command: &'static str,
        name: &'static str,
    ) -> Result<Self> {
        if allowed.contains(&self) {
            Ok(self)
        } else {
            let expected = one_of(allowed);
            Err(InputError::NotArgumentValue {
                command: command.to_owned(),
                name,
                expected,
            }
            .into())
        }
    }
}

fn one_of(rights: &[Right]) -> String {
    let words: Vec<String> = rights.iter().map(ToString::to_string).collect();
    format!("one of {}", words.join(", "))
}

/// A switch with words, for `--template`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OnOff {
    On,
    Off,
}

impl EnumArgument for OnOff {}

const GRANTABLE: &[Right] = &[
    Right::Viewer,
    Right::Commentator,
    Right::Editor,
    Right::Full,
];
const BY_LINK: &[Right] = &[
    Right::None,
    Right::Viewer,
    Right::Commentator,
    Right::Editor,
];
const BY_MEMBERS: &[Right] = &[
    Right::None,
    Right::Viewer,
    Right::Commentator,
    Right::Editor,
    Right::Full,
];
const TO_COPY: &[Right] = &[Right::Viewer, Right::Editor, Right::Full];
const TO_MANAGE: &[Right] = &[Right::Editor, Right::Full];

/// What a rights command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsAction {
    Grant,
    Revoke,
    Share,
    Transfer,
}

/// A user, as the workspace names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardMember {
    pub id: String,
    pub name: Option<String>,
}

impl BoardMember {
    fn of(member: &Member) -> Self {
        Self {
            id: member.id.clone(),
            name: member.name.clone(),
        }
    }

    fn shown(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// A change of who may use a board.
#[derive(Debug, Serialize)]
pub struct Shared {
    pub action: RightsAction,
    pub id: String,
    pub name: Option<String>,
    /// Where the board opens.
    pub link: String,
    pub user: Option<BoardMember>,
    /// The right a grant gave.
    pub right: Option<Right>,
    /// The sharing settings a share set.
    pub settings: Option<Settings>,
    /// What each setting a share names was, and becomes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<SettingChange>,
    /// The first request's report, or why nothing was sent.
    pub outcome: Settled,
    /// The reports of the requests after the first, in the order they go out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub then: Vec<ChangeReport>,
}

/// The sharing settings a share changes, in the words of its flags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Right>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Right>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<Right>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<Right>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Right>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<Right>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<OnOff>,
}

/// One sharing setting: what it is now, and what the share makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingChange {
    pub setting: &'static str,
    pub was: String,
    pub becomes: String,
}

impl SettingChange {
    fn line(&self) -> String {
        let Self {
            setting,
            was,
            becomes,
        } = self;
        if was == becomes {
            format!("{setting}: {was} (unchanged)")
        } else {
            format!("{setting}: {was} \u{2192} {becomes}")
        }
    }
}

/// The right the board's reply holds under `key`, in the words of the flags.
fn right_was(channel: &Value, key: &str) -> String {
    channel.get(key).map_or_else(
        || "unknown".to_owned(),
        |value| {
            let wire = value.as_str();
            BY_MEMBERS
                .iter()
                .find(|right| right.wire() == wire)
                .map_or_else(|| value.to_string(), ToString::to_string)
        },
    )
}

/// Who opens the board by its link: none when it is not public.
fn link_was(channel: &Value) -> String {
    if channel.get("public").and_then(Value::as_bool) == Some(false) {
        Right::None.to_string()
    } else {
        right_was(channel, "public_access")
    }
}

fn template_was(channel: &Value) -> String {
    channel
        .get("share_as_template")
        .and_then(Value::as_bool)
        .map_or_else(
            || "unknown".to_owned(),
            |on| if on { OnOff::On } else { OnOff::Off }.to_string(),
        )
}

impl Settings {
    /// Each named setting with its value in `channel` (the board's reply) and
    /// the one these settings give it.
    fn changes(&self, channel: &Value) -> Vec<SettingChange> {
        let rights = [
            ("link", self.link, link_was(channel)),
            (
                "members",
                self.members,
                right_was(channel, "workspace_member_access"),
            ),
            (
                "copy",
                self.copy,
                right_was(channel, "who_can_copy_content"),
            ),
            (
                "delete",
                self.delete,
                right_was(channel, "who_can_delete_content"),
            ),
            (
                "tools",
                self.tools,
                right_was(channel, "who_can_manage_collaboration_tools"),
            ),
            (
                "lock",
                self.lock,
                right_was(channel, "who_can_manage_lock_content"),
            ),
        ];
        rights
            .into_iter()
            .filter_map(|(setting, becomes, was)| {
                becomes.map(|right| SettingChange {
                    setting,
                    was,
                    becomes: right.to_string(),
                })
            })
            .chain(self.template.map(|template| SettingChange {
                setting: "template",
                was: template_was(channel),
                becomes: template.to_string(),
            }))
            .collect()
    }

    /// The server's patches for these settings: the keys of the channel patch
    /// (who sees the board), and the `who_can_*` keys of `sharing_settings`.
    /// A patch with no key set is left out.
    fn patches(&self) -> (Option<SharingPatch>, Option<SharingPatch>) {
        let role = |right: Option<Right>| right.map(|r| WireRole::from(r.wire()));
        let access = SharingPatch {
            public: self.link.map(|link| link != Right::None),
            public_access: role(self.link),
            workspace_member_access: role(self.members),
            share_as_template: self.template.map(|t| t == OnOff::On),
            ..SharingPatch::default()
        };
        let sharing = SharingPatch {
            who_can_copy_content: role(self.copy),
            who_can_delete_content: role(self.delete),
            who_can_manage_collaboration_tools: role(self.tools),
            who_can_manage_lock_content: role(self.lock),
            who_can_manage_frame_hide_content: role(self.lock),
            ..SharingPatch::default()
        };
        let set =
            |patch: SharingPatch| (patch != SharingPatch::default()).then_some(patch);
        (set(access), set(sharing))
    }
}

/// What came of it.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Settled {
    Carried(ChangeReport),
    /// A revoke of a right the user does not hold: nothing was sent.
    NotHeld {
        status: &'static str,
    },
}

impl Shared {
    /// What the change did, or would do, in a sentence; names are made safe to
    /// print.
    #[must_use]
    pub fn phrase(&self) -> String {
        let board = printable(self.name.as_deref().unwrap_or(&self.id));
        let user = self
            .user
            .as_ref()
            .map_or_else(Default::default, |user| printable(user.shown()));
        let right = self
            .right
            .map(|right| right.to_string())
            .unwrap_or_default();
        let would = matches!(
            self.outcome,
            Settled::Carried(ChangeReport::DryRun { .. } | ChangeReport::Held { .. })
        );
        match (&self.outcome, self.action, would) {
            (Settled::NotHeld { .. }, ..) => {
                format!("{user} holds no right on {board}; nothing was sent")
            }
            (_, RightsAction::Grant, false) => {
                format!("gave {user} the right {right} on {board}")
            }
            (_, RightsAction::Grant, true) => {
                format!("would give {user} the right {right} on {board}")
            }
            (_, RightsAction::Revoke, false) => {
                format!("took away the right of {user} on {board}")
            }
            (_, RightsAction::Revoke, true) => {
                format!("would take away the right of {user} on {board}")
            }
            (_, RightsAction::Transfer, false) => {
                format!("made {user} the owner of {board}")
            }
            (_, RightsAction::Transfer, true) => {
                format!("would make {user} the owner of {board}")
            }
            (_, RightsAction::Share, false) => {
                format!("changed the sharing of {board}")
            }
            (_, RightsAction::Share, true) => {
                format!("would change the sharing of {board}")
            }
        }
    }

    /// What a change not sent would do, as lines: a share's settings with what
    /// they were, else the requests as they would go out.
    #[must_use]
    pub fn plan_lines(&self) -> Vec<String> {
        if self.changes.is_empty() {
            self.plans().flat_map(PlannedRequest::lines).collect()
        } else {
            self.changes.iter().map(SettingChange::line).collect()
        }
    }

    /// The requests that did not go out (a dry run, or a change held), in the
    /// order they would.
    pub fn plans(&self) -> impl Iterator<Item = &PlannedRequest> + Clone {
        let first = match &self.outcome {
            Settled::Carried(report) => Some(report),
            Settled::NotHeld { .. } => None,
        };
        first
            .into_iter()
            .chain(&self.then)
            .filter_map(|report| match report {
                ChangeReport::DryRun { request }
                | ChangeReport::Held { request, .. } => Some(request),
                ChangeReport::Sent => None,
            })
    }

    /// The refusal of a change held for its confirmation.
    #[must_use]
    pub fn refusal(&self) -> Option<&Unconfirmed> {
        match &self.outcome {
            Settled::Carried(report) => report.refusal(),
            Settled::NotHeld { .. } => None,
        }
    }
}

/// Gives a user a right on a board; a right held is replaced.
///
/// # Errors
/// A right that cannot be given (`none`), a board or user that is not found,
/// and errors of the requests.
pub async fn grant(client: &Client, args: &GrantArgs) -> Result<Shared> {
    let right = args.right.among(GRANTABLE, "board grant", "right")?;
    let board = board(client, &args.board, args.workspace.as_deref()).await?;
    let user = user(client, &board, &args.user, "board grant").await?;
    let request =
        api::access::set_member_right_request(&board.id, &user.id, right.wire())?;
    let unconfirmed = unconfirmed(format!(
        "giving {} the right {right} on {}",
        user.shown(),
        board.shown()
    ));
    let (outcome, _) = settle(
        client,
        request,
        Some(unconfirmed),
        (args.dry_run, args.confirm),
    )
    .await?;
    Ok(Shared {
        user: Some(user),
        right: Some(right),
        ..board.shared(client, RightsAction::Grant, Settled::Carried(outcome))
    })
}

/// Takes a user's right on a board away. A right the user does not hold is
/// told, and nothing is sent.
///
/// # Errors
/// A board or user that is not found, and errors of the requests.
pub async fn revoke(client: &Client, args: &RevokeArgs) -> Result<Shared> {
    let board = board(client, &args.board, args.workspace.as_deref()).await?;
    let user = user(client, &board, &args.user, "board revoke").await?;
    let holders = api::channel::users(client, &board.id).await?;
    if holds_right(holders, &user.id)? {
        let request = api::access::set_member_right_request(&board.id, &user.id, None)?;
        let unconfirmed = unconfirmed(format!(
            "taking away the right of {} on {}",
            user.shown(),
            board.shown()
        ));
        let (outcome, _) = settle(
            client,
            request,
            Some(unconfirmed),
            (args.dry_run, args.confirm),
        )
        .await?;
        Ok(Shared {
            user: Some(user),
            ..board.shared(client, RightsAction::Revoke, Settled::Carried(outcome))
        })
    } else {
        let settled = Settled::NotHeld { status: "not_held" };
        Ok(Shared {
            user: Some(user),
            ..board.shared(client, RightsAction::Revoke, settled)
        })
    }
}

/// Makes a member the owner of a board, or of a folder and, with
/// `with_boards`, the boards inside. Always a risky change: it cannot be
/// undone.
///
/// # Errors
/// A board or user that is not found, and errors of the requests.
pub async fn transfer(client: &Client, args: &TransferArgs) -> Result<Shared> {
    let board = board(client, &args.board, args.workspace.as_deref()).await?;
    let user = user(client, &board, &args.user, "board transfer").await?;
    let subtree = if args.with_boards {
        SubtreeTransfer::Transfer
    } else {
        SubtreeTransfer::Keep
    };
    let request =
        api::access::transfer_ownership_request(&board.id, &user.id, subtree)?;
    let unconfirmed = Unconfirmed {
        change: format!(
            "transferring ownership of {} to {}",
            board.shown(),
            user.shown()
        )
        .into(),
        risk: ChangeRisk::Irreversible,
    };
    let (outcome, _) = settle(
        client,
        request,
        Some(unconfirmed),
        (args.dry_run, args.confirm),
    )
    .await?;
    Ok(Shared {
        user: Some(user),
        ..board.shared(client, RightsAction::Transfer, Settled::Carried(outcome))
    })
}

/// Changes the sharing settings the flags name; the others stay.
///
/// # Errors
/// No setting given, a right the flag does not take, a board that is not
/// found, and errors of the requests.
pub async fn share(client: &Client, args: &ShareArgs) -> Result<Shared> {
    let settings = settings(args)?;
    let board = board(client, &args.board, args.workspace.as_deref()).await?;
    let (access, sharing) = settings.patches();
    let mut requests = Vec::new();
    if let Some(members) = settings.members {
        requests.extend(keep_caller(client, &board, members).await?);
    }
    if let Some(patch) = access {
        requests.push(api::access::set_channel_access_request(&board.id, patch)?);
    }
    if let Some(patch) = sharing {
        requests.push(api::access::set_sharing_settings_request(&board.id, patch)?);
    }
    let unconfirmed = unconfirmed(format!(
        "changing the sharing settings of {}",
        board.shown()
    ));
    let mode = (args.dry_run, args.confirm);
    let mut reports = stream::iter(requests.into_iter().map(Ok))
        .and_then(|request| settle(client, request, Some(unconfirmed.clone()), mode))
        .map_ok(|(report, _)| report)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter();
    let first = reports.next();
    first.map_or_else(
        || Err(no_setting()),
        |first| {
            Ok(Shared {
                changes: settings.changes(&board.channel),
                settings: Some(settings),
                then: reports.collect(),
                ..board.shared(client, RightsAction::Share, Settled::Carried(first))
            })
        },
    )
}

/// The request that makes the caller co-owner, when setting the members'
/// access to `members` would take their access with it.
async fn keep_caller(
    client: &Client,
    board: &Board,
    members: Right,
) -> Result<Option<ApiRequest>> {
    let by_members = board
        .channel
        .get("workspace_member_access")
        .and_then(Value::as_str);
    if by_members == Right::Full.wire() && members != Right::Full {
        let me = account::account_id(client).await?;
        let drops = drops_access(client, board, &me).await?;
        drops
            .then(|| {
                api::access::set_member_right_request(
                    &board.id,
                    &me,
                    Right::Full.wire(),
                )
            })
            .transpose()
    } else {
        Ok(None)
    }
}

/// Whether `user` holds the board only through all workspace members: not as
/// a co-owner of their own, not as an admin who owns every board, not through
/// the folder, as the web client tells.
async fn drops_access(client: &Client, board: &Board, user: &str) -> Result<bool> {
    let holders = sharing::members(api::channel::users(client, &board.id).await?)?;
    Ok(role_of(&holders, user) != Some(HeldRole::CoOwner)
        && is_member(client, board, user).await?
        && !folder_owns(client, board, user).await?)
}

/// Whether `user` is a member of the board's workspace who does not own the
/// board as an admin.
async fn is_member(client: &Client, board: &Board, user: &str) -> Result<bool> {
    let workspace = roster(client, board).await?;
    let admins_own = workspace
        .get("workspace_admin_as_channels_owner")
        .and_then(Value::as_bool)
        == Some(true);
    let seat = members_of(workspace)?
        .into_iter()
        .find(|member| member.id == user)
        .and_then(|member| member.role);
    Ok(match seat.as_deref() {
        Some("admin" | "owner") => !admins_own,
        Some("member") => true,
        _ => false,
    })
}

/// Whether the board's folder makes `user` a co-owner of it.
async fn folder_owns(client: &Client, board: &Board, user: &str) -> Result<bool> {
    let by = |key: &str| board.channel.get(key).and_then(Value::as_str);
    if by("parent_type") == Some("channel") {
        let folder = sharing::folder_rights(
            api::channel::parent_total_permits(client, &board.id).await?,
        )?;
        let held = role_of(&folder, user);
        Ok(match by("parent_channel_access") {
            Some("inherit") => held == Some(HeldRole::CoOwner),
            Some("co_owner") => held.is_some(),
            _ => false,
        })
    } else {
        Ok(false)
    }
}

fn role_of(holders: &[sharing::Holder], user: &str) -> Option<HeldRole> {
    holders
        .iter()
        .find(|holder| holder.user_id == user)
        .and_then(sharing::Holder::role)
}

/// The sharing settings the flags set.
fn settings(args: &ShareArgs) -> Result<Settings> {
    let command = "board share";
    let settings = Settings {
        link: args
            .link
            .map(|link| link.among(BY_LINK, command, "link"))
            .transpose()?,
        members: args
            .members
            .map(|members| members.among(BY_MEMBERS, command, "members"))
            .transpose()?,
        copy: args
            .copy
            .map(|copy| copy.among(TO_COPY, command, "copy"))
            .transpose()?,
        delete: args
            .delete
            .map(|delete| delete.among(TO_MANAGE, command, "delete"))
            .transpose()?,
        tools: args
            .tools
            .map(|tools| tools.among(TO_MANAGE, command, "tools"))
            .transpose()?,
        lock: args
            .lock
            .map(|lock| lock.among(TO_MANAGE, command, "lock"))
            .transpose()?,
        template: args.template,
    };
    if settings == Settings::default() {
        Err(no_setting())
    } else {
        Ok(settings)
    }
}

fn no_setting() -> crate::error::Error {
    InputError::NoArgument {
        command: "board share".to_owned(),
        name: "a setting to change (--link, --members, --copy, --delete, --tools, --lock or --template)",
    }
    .into()
}

/// Whether `holders` (the board's `channel_user/users` reply) lists `user`
/// with a right.
fn holds_right(holders: Value, user: &str) -> Result<bool> {
    Ok(sharing::members(holders)?
        .iter()
        .any(|holder| holder.user_id == user && holder.role().is_some()))
}

/// A board found, with what the commands report of it.
struct Board {
    id: String,
    /// The board as the server replies.
    channel: Value,
    name: Option<String>,
    workspace: Option<String>,
    /// The workspace of the command, for a reply that names none.
    workspace_reference: Option<String>,
}

impl Board {
    fn shown(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    fn shared(self, client: &Client, action: RightsAction, outcome: Settled) -> Shared {
        Shared {
            action,
            link: client.board_link(&self.id),
            id: self.id,
            name: self.name,
            user: None,
            right: None,
            settings: None,
            changes: Vec::new(),
            outcome,
            then: Vec::new(),
        }
    }
}

/// A board by id, link or name.
async fn board(
    client: &Client,
    reference: &str,
    workspace: Option<&str>,
) -> Result<Board> {
    let id = lookup::board_id(client, reference, workspace).await?;
    let channel = api::channel::get(client, &id).await?;
    let summary = NodeBrief::of(&channel)?;
    Ok(Board {
        name: Some(summary.name().to_owned()).filter(|name| !name.is_empty()),
        workspace: summary.workspace_id().map(str::to_owned),
        workspace_reference: workspace.map(str::to_owned),
        id,
        channel,
    })
}

/// The reply of the board's workspace.
async fn roster(client: &Client, board: &Board) -> Result<Value> {
    let workspace = match &board.workspace {
        Some(id) => id.clone(),
        None => {
            lookup::workspace(client, board.workspace_reference.as_deref())
                .await?
                .id
        }
    };
    api::workspace::get(client, &workspace).await
}

/// The one member of the board's workspace `reference` names.
async fn user(
    client: &Client,
    board: &Board,
    reference: &str,
    command: &str,
) -> Result<BoardMember> {
    let roster = members_of(roster(client, board).await?)?;
    find_member(&roster, reference, command).map(BoardMember::of)
}

/// A change that needs its confirmation; the shell words how to give it.
fn unconfirmed(change: String) -> Unconfirmed {
    Unconfirmed {
        change: change.into(),
        risk: ChangeRisk::Sharing,
    }
}

#[cfg(test)] mod tests;
