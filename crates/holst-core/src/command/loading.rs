//! Pieces the commands share to load a board: socket, cache and area.

use crate::{
    command::{
        account,
        board::{AreaArgs, Caching},
        board_load::CacheUse,
        lookup::{self, Fresh},
        table::not_value,
    },
    domain::{
        area::Area,
        board::{Object, ObjectKind},
        id::{BoardId, ObjectId},
    },
    effect::{cache::CachePort, settings::CacheSettings, socket::SocketConnector},
    error::{
        Error,
        Result,
        input::InputError,
        local_data::LocalDataError,
        lookup::{LookupError, LookupKind, LookupMiss},
        picture::PictureError,
    },
    remote::{client::Client, socket::write::connector},
    storage::cache::Cache,
};

/// How a board is loaded: the socket, the cache and how the cache is used.
#[derive(Clone, Copy)]
pub(in crate::command) struct Loading<'a> {
    pub(in crate::command) connector: &'a dyn SocketConnector,
    pub(in crate::command) cache: Option<&'a dyn CachePort>,
    pub(in crate::command) cache_use: CacheUse,
}

pub(in crate::command) fn cache_use(
    caching: &Caching,
    command: &str,
) -> Result<CacheUse> {
    match (caching.refresh, caching.offline) {
        (true, true) => Err(not_value(command, "offline", "left out with --refresh")),
        (true, false) => Ok(CacheUse::Refresh),
        (false, true) => Ok(CacheUse::Offline),
        (false, false) => Ok(CacheUse::Use),
    }
}

/// The logged-in account's cache; offline, it must already be known.
///
/// # Errors
/// Offline, an account that is not known.
pub async fn account_cache(
    client: &Client,
    settings: &CacheSettings,
    cache_use: CacheUse,
) -> Result<Option<Cache>> {
    match (cache_use, &settings.root) {
        (_, None) => Ok(None),
        (CacheUse::Offline, Some(_)) => client
            .account_id()
            .map(|user| Cache::from_settings(settings, &user, client.clock()))
            .ok_or(Error::LocalData(LocalDataError::AccountUnknown)),
        (CacheUse::Use | CacheUse::Refresh, Some(_)) => {
            Ok(account::known_account_id(client)
                .await
                .and_then(|user| Cache::from_settings(settings, &user, client.clock())))
        }
    }
}

/// Opens the account's cache and runs `then` over the real socket and it.
pub(in crate::command) async fn through_cache<T>(
    client: &Client,
    settings: &CacheSettings,
    cache_use: CacheUse,
    then: impl AsyncFnOnce(Loading<'_>) -> Result<T>,
) -> Result<T> {
    let account = account_cache(client, settings, cache_use).await?;
    then(Loading {
        connector: connector(),
        cache: account.as_ref().map(Cache::as_port),
        cache_use,
    })
    .await
}

pub(in crate::command) fn explicit_area(selection: &AreaArgs) -> Result<Area> {
    Area::of(
        selection
            .rectangle
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::parse)
            .transpose()?,
        selection
            .frame
            .clone()
            .filter(|frame| !frame.trim().is_empty()),
        selection.object.clone(),
    )
}

/// A board by id, link or name; offline, only by id or link.
pub(in crate::command) async fn board_id(
    client: &Client,
    reference: &str,
    workspace: Option<&str>,
    cache_use: CacheUse,
) -> Result<String> {
    match cache_use {
        CacheUse::Offline => BoardId::find_in(reference)
            .map(|id| id.as_str().to_owned())
            .ok_or_else(|| {
                Error::Input(InputError::OfflineByName {
                    reference: reference.to_owned(),
                })
            }),
        CacheUse::Use => {
            lookup::board_id_with(client, reference, workspace, Fresh::Kept).await
        }
        CacheUse::Refresh => {
            lookup::board_id_with(client, reference, workspace, Fresh::Asked).await
        }
    }
}

/// The flags' area, else the part a link to a frame or an object names.
pub(in crate::command) fn chosen_area(
    flags: &Area,
    linked: Option<ObjectId>,
    objects: &[Object],
) -> Result<Area> {
    match (flags, linked) {
        (area, None) => Ok(area.clone()),
        (Area::Board, Some(wanted)) => objects
            .iter()
            .find(|object| object.id.eq_ignore_ascii_case(wanted.as_str()))
            .map(|object| {
                if matches!(object.kind, ObjectKind::Frame) {
                    Area::Frame(object.id.clone())
                } else {
                    Area::Objects(vec![object.id.clone()])
                }
            })
            .ok_or_else(|| {
                Error::Lookup(LookupError {
                    kind: LookupKind::Object,
                    reference: wanted.as_str().to_owned(),
                    miss: LookupMiss::not_found(),
                })
            }),
        (Area::Rectangle(_) | Area::Frame(_) | Area::Objects(_), Some(_)) => {
            Err(Error::Picture(PictureError::SeveralAreas))
        }
    }
}
