//! Finding a template by id or name.

use futures_util::future::join_all;
use itertools::{Either, Itertools as _};

use super::{ListedTemplate, listed_under, own, standard};
use crate::{
    command::lookup,
    domain::{id, template::Template},
    error::{
        Error,
        Result,
        http::HttpError,
        lookup::{
            Candidate,
            Candidates,
            LookupError,
            LookupKind,
            LookupMiss,
            SkipReason,
            Skipped,
            SkippedWorkspaces,
        },
    },
    remote::{api, client::Client},
};

/// A template's id and content, given its id or its name.
pub(super) async fn found(
    client: &Client,
    reference: &str,
) -> Result<(String, Template)> {
    let reference = reference.trim();
    let id = if id::is_uuid(reference) {
        reference.to_owned()
    } else {
        by_name(client, reference).await?
    };
    api::template::get(client, &id)
        .await
        .map(|template| (id, template))
        .map_err(|error| match error.status() {
            Some(404) => not_found(
                reference,
                SkippedWorkspaces::default(),
                Candidates::default(),
            ),
            Some(_) | None => error,
        })
}

/// The id of the one template whose name is, else contains, `reference`, among
/// your own, your workspaces' and the ready-made ones.
async fn by_name(client: &Client, reference: &str) -> Result<String> {
    let workspaces = lookup::workspaces(client).await?;
    let mut everything = own(client).await?;
    let listings = join_all(
        workspaces
            .iter()
            .map(|workspace| api::template::workspace(client, &workspace.id)),
    )
    .await;
    let (read, unread): (Vec<_>, Vec<_>) = workspaces
        .into_iter()
        .zip(listings)
        .partition_map(|(workspace, listing)| match listing {
            Ok(templates) => {
                Either::Left(listed_under(Some(workspace.name), templates))
            }
            Err(error) => Either::Right(Skipped {
                workspace: workspace.name,
                reason: SkipReason::from(&error),
                status: error.status(),
            }),
        });
    everything.extend(read.into_iter().flatten());
    everything.extend(standard(client, None).await?);
    let unread = SkippedWorkspaces(unread);
    let wanted = reference.to_lowercase();
    let named = |exact: bool| -> Vec<&ListedTemplate> {
        everything
            .iter()
            .filter(|listed| {
                listed
                    .template
                    .name
                    .as_deref()
                    .map(str::to_lowercase)
                    .is_some_and(|name| {
                        if exact {
                            name == wanted
                        } else {
                            name.contains(&wanted)
                        }
                    })
            })
            .unique_by(|listed| listed.template.id.clone())
            .collect()
    };
    let exact = named(true);
    let matching = if exact.is_empty() {
        named(false)
    } else {
        exact
    };
    match matching.as_slice() {
        [one] => one.template.id.clone().ok_or_else(|| {
            Error::Http(HttpError::MissingField {
                response: "template list",
                field: "id",
            })
        }),
        [] => Err(not_found(
            reference,
            unread,
            candidates(everything.iter().unique_by(|listed| &listed.template.id)),
        )),
        several => Err(Error::Lookup(LookupError {
            kind: LookupKind::Template,
            reference: reference.to_owned(),
            miss: LookupMiss::Ambiguous {
                candidates: candidates(several.iter().copied()),
                unread,
            },
        })),
    }
}

/// The listed templates, named so the reader can tell them apart.
fn candidates<'a>(listed: impl IntoIterator<Item = &'a ListedTemplate>) -> Candidates {
    Candidates::capped(
        listed.into_iter().map(|listed| Candidate {
            name: listed.template.name.clone().unwrap_or_default(),
            id: listed.template.id.clone(),
            workspace: listed.under.clone(),
            ..Candidate::default()
        }),
        CANDIDATES_SHOWN,
    )
}

/// How many templates a lookup that failed names.
const CANDIDATES_SHOWN: usize = 10;

fn not_found(
    reference: &str,
    unread: SkippedWorkspaces,
    available: Candidates,
) -> Error {
    Error::Lookup(LookupError {
        kind: LookupKind::Template,
        reference: reference.to_owned(),
        miss: LookupMiss::NotFound { unread, available },
    })
}
