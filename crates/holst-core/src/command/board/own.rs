//! `board favorite unfavorite touch leave`: the user's own view of a board.

use serde_json::Value;

use crate::{
    command::{
        account,
        board::{
            FavoriteArgs,
            LeaveArgs,
            TouchArgs,
            UnfavoriteArgs,
            tree_change::{Action, Changed, ChangedNode, Plan, carry, node_of},
        },
        lookup,
        table::Confirmation,
    },
    domain::node::{NodeBrief, NodeKind},
    effect::{board_write::WriteMode, request::ApiRequest},
    error::{Result, input::InputError},
    remote::{api, api::recent::SetRecent, client::Client},
};

/// Adds a board to the favorites, at `--index` or at the end.
///
/// # Errors
/// A board that is not found, and errors of the requests.
pub async fn favorite(client: &Client, args: &FavoriteArgs) -> Result<Changed> {
    let request = async |board: &Found| {
        api::favorite::add_request(&api::favorite::AddFavorite {
            channel_id: board.id.clone(),
            index: args.index.unwrap_or(-1),
        })
    };
    plain(
        client,
        (&args.board, args.workspace.as_deref()),
        Action::Favorite,
        args.dry_run,
        request,
    )
    .await
}

/// Removes a board from the favorites.
///
/// # Errors
/// A board that is not found, and errors of the requests.
pub async fn unfavorite(client: &Client, args: &UnfavoriteArgs) -> Result<Changed> {
    let request = async |board: &Found| {
        api::favorite::remove_request(&api::favorite::RemoveFavorite {
            channel_id: board.id.clone(),
        })
    };
    plain(
        client,
        (&args.board, args.workspace.as_deref()),
        Action::Unfavorite,
        args.dry_run,
        request,
    )
    .await
}

/// Marks a board as recently opened.
///
/// # Errors
/// A board that is not found, and errors of the requests.
pub async fn touch(client: &Client, args: &TouchArgs) -> Result<Changed> {
    let request = async |board: &Found| {
        api::recent::set_request(&SetRecent {
            channel_id: board.id.parse()?,
        })
    };
    plain(
        client,
        (&args.board, args.workspace.as_deref()),
        Action::Touch,
        args.dry_run,
        request,
    )
    .await
}

/// Leaves a board the user was invited to. A board the user made stays: it is
/// removed, not left.
///
/// # Errors
/// A board that is not found, [`InputError::LeaveOwnBoard`], and errors of the
/// requests.
pub async fn leave(client: &Client, args: &LeaveArgs) -> Result<Changed> {
    let request = async |board: &Found| {
        let owned = match board.reply.get("author_id").and_then(Value::as_str) {
            Some(author) => author == account::account_id(client).await?,
            None => false,
        };
        owned
            .then(|| InputError::LeaveOwnBoard {
                name: board.node.name.clone().unwrap_or_else(|| board.id.clone()),
            })
            .map_or_else(
                || {
                    // A dry run keeps the tree; only a sent leave changes it.
                    matches!(args.dry_run, WriteMode::Send)
                        .then(|| lookup::forget_trees_holding(client, &board.id));
                    api::channel::leave_request(&board.id)
                },
                |refusal| Err(refusal.into()),
            )
    };
    plain(
        client,
        (&args.board, args.workspace.as_deref()),
        Action::Leave,
        args.dry_run,
        request,
    )
    .await
}

/// A board as the server describes it, and the node a change of it reports.
struct Found {
    id: String,
    node: ChangedNode,
    reply: Value,
}

/// Finds the board, builds the one changing request, sends it unconfirmed.
async fn plain(
    client: &Client,
    (reference, workspace): (&str, Option<&str>),
    action: Action,
    dry_run: WriteMode,
    request: impl AsyncFnOnce(&Found) -> Result<ApiRequest>,
) -> Result<Changed> {
    let board = board(client, reference, workspace, action).await?;
    let request = request(&board).await?;
    carry(
        client,
        Plan::plain(request, board.node),
        (dry_run, Confirmation::Missing),
    )
    .await
}

/// A board by id, link or name.
async fn board(
    client: &Client,
    reference: &str,
    workspace: Option<&str>,
    action: Action,
) -> Result<Found> {
    let id = lookup::board_id(client, reference, workspace).await?;
    let kept =
        lookup::cached_node(client, &id).filter(|_| !matches!(action, Action::Leave));
    let reply = match &kept {
        Some(_) => Value::Null,
        None => api::channel::get(client, &id).await?,
    };
    let name = match kept {
        Some(kept) => kept.name,
        None => NodeBrief::of(&reply)?.name().to_owned(),
    };
    let node = ChangedNode {
        name: Some(name).filter(|name| !name.is_empty()),
        ..node_of(action, Some(id.clone()), Some(NodeKind::Board))
    };
    Ok(Found { id, node, reply })
}
