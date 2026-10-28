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
    error::{Result, input::InputError},
    remote::{api, client::Client},
};

/// Adds a board to the favorites, at `--index` or at the end.
///
/// # Errors
/// A board that is not found, and errors of the requests.
pub async fn favorite(client: &Client, args: &FavoriteArgs) -> Result<Changed> {
    let (id, node, _) = board(
        client,
        &args.board,
        args.workspace.as_deref(),
        Action::Favorite,
    )
    .await?;
    let request = api::favorite::add_request(&id, args.index.unwrap_or(-1))?;
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// Removes a board from the favorites.
///
/// # Errors
/// A board that is not found, and errors of the requests.
pub async fn unfavorite(client: &Client, args: &UnfavoriteArgs) -> Result<Changed> {
    let (id, node, _) = board(
        client,
        &args.board,
        args.workspace.as_deref(),
        Action::Unfavorite,
    )
    .await?;
    let request = api::favorite::remove_request(&id)?;
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// Marks a board as recently opened.
///
/// # Errors
/// A board that is not found, and errors of the requests.
pub async fn touch(client: &Client, args: &TouchArgs) -> Result<Changed> {
    let (id, node, _) = board(
        client,
        &args.board,
        args.workspace.as_deref(),
        Action::Touch,
    )
    .await?;
    let request = api::recent::set_request(&id)?;
    carry(
        client,
        Plan::plain(request, node),
        (args.dry_run, Confirmation::Missing),
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
    let (id, node, reply) = board(
        client,
        &args.board,
        args.workspace.as_deref(),
        Action::Leave,
    )
    .await?;
    let owned = match reply.get("author_id").and_then(Value::as_str) {
        Some(author) => author == account::account_id(client).await?,
        None => false,
    };
    if owned {
        Err(InputError::LeaveOwnBoard {
            name: node.name.unwrap_or(id),
        }
        .into())
    } else {
        let request = api::channel::leave_request(&id)?;
        carry(
            client,
            Plan::plain(request, node),
            (args.dry_run, Confirmation::Missing),
        )
        .await
    }
}

/// A board by id, link or name: its id, the node a change of it reports, and
/// the reply the server describes it by.
async fn board(
    client: &Client,
    reference: &str,
    workspace: Option<&str>,
    action: Action,
) -> Result<(String, ChangedNode, Value)> {
    let id = lookup::board_id(client, reference, workspace).await?;
    let reply = api::channel::get(client, &id).await?;
    let name = NodeBrief::of(&reply)?.name().to_owned();
    let node = ChangedNode {
        name: Some(name).filter(|name| !name.is_empty()),
        ..node_of(action, Some(id.clone()), Some(NodeKind::Board))
    };
    Ok((id, node, reply))
}
