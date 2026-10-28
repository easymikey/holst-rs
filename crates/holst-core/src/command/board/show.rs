//! `board show`: boards, or a part of one, with details.

use std::collections::BTreeMap;

use futures_util::future::{OptionFuture, join, join_all, try_join};
use itertools::Itertools as _;
use serde::Serialize;
use serde_json::Value;

use crate::{
    command::{
        account,
        board::{AreaArgs, Caching, ShowArgs, ShowPart, rights::Right},
        board_load::{self, CacheUse, RemoteBoard},
        lookup,
        table::not_value,
    },
    domain::{
        area::{Area, listed_area, scene::Scene},
        board::{EditRight, Object, ObjectKind, note_text, objects},
        geometry::Rectangle,
        id::{self, BoardId, ObjectId},
        node::{NodeBrief, Parent},
        picture::BoardLength,
        sharing::Sharing,
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
    protocol::sharing::{self, HeldRole},
    remote::{api, client::Client, socket::write::connector},
    storage::cache::Cache,
};

const COMMAND: &str = "board show";

/// The boards that were found, and the references that were not.
#[derive(Debug, Serialize)]
pub struct Briefs {
    pub boards: Vec<BoardBrief>,
    /// References not found: the command ends as not found, after the rest.
    pub missing: Vec<Missing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Missing {
    pub reference: String,
    pub reason: String,
}

/// One board, or the part of it a selection or a link names.
#[derive(Debug, Serialize)]
pub struct BoardBrief {
    pub id: String,
    pub name: String,
    /// A link to the board, or to the selected frame or object.
    pub link: String,
    pub workspace: Option<String>,
    /// The folder it sits in; `None` at the workspace root.
    pub folder: Option<String>,
    pub in_trash: bool,
    pub edit_right: EditRight,
    /// The part shown with its padding; the extent of the objects when none.
    pub rectangle: Option<Rectangle>,
    /// How many objects the part holds, not counting the frame it is.
    pub count: usize,
    /// The count by object kind.
    pub objects: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<Holder>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The rights the board gets through its folder.
    pub rights: Option<Vec<Holder>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharing: Option<Sharing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A user and the right they hold on a board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Holder {
    pub user: String,
    pub right: Right,
}

/// How a board is loaded: the socket, the cache and how the cache is used.
#[derive(Clone, Copy)]
pub(in crate::command) struct Loading<'a> {
    pub(in crate::command) connector: &'a dyn SocketConnector,
    pub(in crate::command) cache: Option<&'a dyn CachePort>,
    pub(in crate::command) cache_use: CacheUse,
}

/// Shows the boards asked for. A reference that is not found is listed in
/// [`Briefs::missing`] while another is shown; when none is found, its error is
/// the result.
///
/// # Errors
/// An area flag with several boards, `--refresh` with `--offline`, no board,
/// an area that is not valid, a board that is not found, and errors of the
/// requests.
pub async fn show(
    client: &Client,
    cache: &CacheSettings,
    args: &ShowArgs,
) -> Result<Briefs> {
    let cache_use = cache_use(&args.caching, COMMAND)?;
    let account = account_cache(client, cache, cache_use).await?;
    shown(
        client,
        Loading {
            connector: connector(),
            cache: account.as_ref().map(Cache::as_port),
            cache_use,
        },
        args,
    )
    .await
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

/// The cache of the logged-in account; offline, the account must be known
/// without asking.
pub(in crate::command) async fn account_cache(
    client: &Client,
    settings: &CacheSettings,
    cache_use: CacheUse,
) -> Result<Option<Cache>> {
    match (cache_use, &settings.root) {
        (_, None) => Ok(None),
        (CacheUse::Offline, Some(_)) => client
            .account_id()
            .map(|user| board_load::account_cache(settings, &user, client.clock()))
            .ok_or(Error::LocalData(LocalDataError::AccountUnknown)),
        (CacheUse::Use | CacheUse::Refresh, Some(_)) => {
            Ok(account::known_account_id(client).await.and_then(|user| {
                board_load::account_cache(settings, &user, client.clock())
            }))
        }
    }
}

async fn shown(
    client: &Client,
    loading: Loading<'_>,
    args: &ShowArgs,
) -> Result<Briefs> {
    let (area, padding) = asked(args)?;
    let area = &area;
    let (boards, failed): (Vec<_>, Vec<_>) =
        join_all(args.board.0.iter().map(|reference| async move {
            board(client, loading, reference, (area, padding), args)
                .await
                .map_err(|error| (reference, error))
        }))
        .await
        .into_iter()
        .partition_result();
    let (gone, fatal): (Vec<_>, Vec<_>) =
        failed.into_iter().partition(|(_, error)| is_missing(error));
    let missing = gone
        .iter()
        .map(|(reference, error)| Missing {
            reference: (*reference).clone(),
            reason: error.to_string(),
        })
        .collect();
    match (
        fatal.into_iter().next(),
        boards.is_empty(),
        gone.into_iter().next(),
    ) {
        (Some((_, error)), _, _) | (None, true, Some((_, error))) => Err(error),
        (None, _, _) => Ok(Briefs { boards, missing }),
    }
}

/// The area and padding asked for, when the flags fit the number of boards.
fn asked(args: &ShowArgs) -> Result<(Area, Option<BoardLength>)> {
    let selection = &args.selection;
    match (args.board.0.len(), area_flag(selection)) {
        (0, _) => Err(InputError::NoArgument {
            command: COMMAND.to_owned(),
            name: "board",
        }
        .into()),
        (2.., Some(flag)) => {
            Err(not_value(COMMAND, flag, "left out with several boards"))
        }
        (_, _) => Ok((
            explicit_area(selection)?,
            BoardLength::optional(selection.padding)?,
        )),
    }
}

fn is_missing(error: &Error) -> bool {
    matches!(error, Error::Lookup(_)) || error.is_board_closed()
}

/// The first area flag given, if any.
fn area_flag(selection: &AreaArgs) -> Option<&'static str> {
    [
        ("rectangle", selection.rectangle.is_some()),
        ("frame", selection.frame.is_some()),
        ("object", !selection.object.is_empty()),
        ("padding", selection.padding.is_some()),
    ]
    .into_iter()
    .find_map(|(name, given)| given.then_some(name))
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

async fn board(
    client: &Client,
    loading: Loading<'_>,
    reference: &str,
    (area, padding): (&Area, Option<BoardLength>),
    args: &ShowArgs,
) -> Result<BoardBrief> {
    let id = board_id(
        client,
        reference,
        args.workspace.as_deref(),
        loading.cache_use,
    )
    .await?;
    let (channel, (right, objects, held_note)) = try_join(
        api::channel::get(client, &id),
        read_board(client, loading, &id, args),
    )
    .await?;
    let area = chosen_area(area, id::object_in(reference), &objects)?;
    let listed = listed_area(&objects, &area, padding)?;
    let target = listed.target.map(|object| object.id.as_str());
    let frame = target.filter(|_| matches!(area, Area::Frame(_)));
    let by_kind = listed
        .objects
        .iter()
        .filter(|object| Some(object.id.as_str()) != frame)
        .fold(BTreeMap::new(), |mut counts, object| {
            *counts
                .entry(object.kind.short_name().to_owned())
                .or_insert(0) += 1;
            counts
        });
    let node = NodeBrief::of(&channel)?;
    let (members, rights) = join(
        part(
            args,
            ShowPart::Members,
            || api::channel::users(client, &id),
            sharing::members,
        ),
        part(
            args,
            ShowPart::Rights,
            || api::channel::parent_total_permits(client, &id),
            sharing::folder_rights,
        ),
    )
    .await;
    Ok(BoardBrief {
        link: client.base_url().link(&id, target),
        name: node.name().to_owned(),
        workspace: node.workspace_id().map(str::to_owned),
        folder: match node.parent() {
            Some(Parent::Folder(folder)) => Some(folder),
            Some(Parent::Workspace(_)) | None => None,
        },
        in_trash: node.in_trash(),
        edit_right: right,
        rectangle: listed
            .rectangle
            .or_else(|| Scene::new(&objects).area_rectangle(&Area::Board, 0.0).ok()),
        count: by_kind.values().sum(),
        objects: by_kind,
        members: members?,
        rights: rights?,
        note: held_note,
        sharing: args
            .with
            .contains(&ShowPart::Sharing)
            .then(|| channel.as_object().cloned().map(Sharing::from))
            .flatten(),
        id,
    })
}

/// The right, the objects and, when asked for, the note of a board: the note
/// is in the document, so the whole board loads for it.
async fn read_board(
    client: &Client,
    loading: Loading<'_>,
    id: &str,
    args: &ShowArgs,
) -> Result<(EditRight, Vec<Object>, Option<String>)> {
    let link = RemoteBoard {
        client,
        connector: loading.connector,
        board: id,
    };
    if args.with.contains(&ShowPart::Note) {
        let (loaded, _) =
            board_load::load(link, loading.cache, loading.cache_use).await?;
        let note = note_text(&loaded.document, args.note_format.unwrap_or_default());
        Ok((loaded.right, objects(&loaded.document), Some(note)))
    } else {
        let (right, index) =
            board_load::index(link, loading.cache, loading.cache_use).await?;
        Ok((right, index.objects(), None))
    }
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
        CacheUse::Use | CacheUse::Refresh => {
            lookup::board_id(client, reference, workspace).await
        }
    }
}

/// The reply to a part asked for with `--with`, read by `read` into users
/// with a right on the scale; nothing when not asked.
async fn part<F: Future<Output = Result<Value>>>(
    args: &ShowArgs,
    wanted: ShowPart,
    request: impl FnOnce() -> F,
    read: fn(Value) -> serde_json::Result<Vec<sharing::Holder>>,
) -> Result<Option<Vec<Holder>>> {
    let reply = OptionFuture::from(args.with.contains(&wanted).then(request))
        .await
        .transpose()?;
    Ok(reply.map(read).transpose()?.map(|holders| {
        holders
            .into_iter()
            .map(|holder| Holder {
                right: match holder.role() {
                    None => Right::None,
                    Some(HeldRole::Viewer) => Right::Viewer,
                    Some(HeldRole::Commentator) => Right::Commentator,
                    Some(HeldRole::Editor) => Right::Editor,
                    Some(HeldRole::CoOwner) => Right::Full,
                },
                user: holder.user_id,
            })
            .collect()
    }))
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

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use futures_util::future::join_all;
    use serde_json::{Value, json};

    use crate::{
        Result,
        command::{
            board::{
                ShowArgs,
                show::{Briefs, Loading, cache_use, shown},
            },
            board_load::CacheUse,
            fixture::{arguments, client},
            table::CommandArgs,
        },
        domain::board::EditRight,
        protocol::hud::{DocumentResponse, UpdatePayload},
        remote::{
            client::{Client, fake::reply},
            socket::fake::{BOARD, Scripted, answer, script, sticker_by, update},
        },
    };

    fn channel() -> String {
        json!({
            "id": BOARD,
            "name": "Plan",
            "workspace_id": "00000001-0000-4000-8000-000000000001",
            "parent_id": "00000001-0000-4000-8000-000000000002",
            "parent_type": "channel",
            "link_access": "viewer",
            "schema_version": 2,
            "server_provider_type": "hud",
        })
        .to_string()
    }

    fn json_reply(body: &str) -> crate::effect::http::HttpResponse {
        reply(200, &[("content-type", "application/json")], body)
    }

    fn missing_reply() -> crate::effect::http::HttpResponse {
        reply(404, &[("content-type", "application/json")], "{}")
    }

    /// A board holding one sticker, `a`, at (1, 2).
    fn board() -> Scripted {
        let (state, _) = sticker_by(4, "a");
        script([Some(vec![
            answer(DocumentResponse::Subscribed {
                right: EditRight::Editable,
            }),
            update(0, UpdatePayload::Inline(state)),
        ])])
    }

    async fn run(
        client: &Client,
        socket: &Scripted,
        given: &Value,
        cache_use: CacheUse,
    ) -> Result<Briefs> {
        shown(
            client,
            Loading {
                connector: socket,
                cache: None,
                cache_use,
            },
            &arguments(given.clone())?,
        )
        .await
    }

    #[test]
    fn the_selection_joins_the_arguments_in_field_order() {
        let names: Vec<_> = ShowArgs::COMMAND
            .arguments
            .iter()
            .map(|argument| argument.name)
            .collect();
        assert_eq!(
            names,
            [
                "board",
                "workspace",
                "with",
                "note_format",
                "rectangle",
                "frame",
                "object",
                "padding",
                "refresh",
                "offline"
            ]
        );
    }

    #[test]
    fn one_board_or_several_are_read_and_the_padding_is_a_number() -> Result<()> {
        let one = arguments::<ShowArgs>(json!({ "board": "A", "padding": 12.5 }))?;
        let many = arguments::<ShowArgs>(json!({ "board": ["A", "B"] }))?;
        assert_eq!(
            (one.board.0, one.selection.padding, many.board.0),
            (
                vec!["A".to_owned()],
                Some(12.5),
                vec!["A".to_owned(), "B".to_owned()]
            )
        );
        let wrong =
            arguments::<ShowArgs>(json!({ "board": 7, "padding": "wide" })).err();
        assert_eq!(
            wrong.map(|error| error.to_string()),
            Some(
                "2 problems:\n  - board of `board show` must be text or a list of them\n  - padding of `board show` must be a number"
                    .to_owned()
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn rights_are_shown_on_the_scale_with_full_for_co_owner() -> Result<()> {
        let (_, client) = client([
            json_reply(&channel()),
            json_reply(&channel()),
            json_reply(
                r#"{"permits":[{"userId":"a","permits":["read","write","set_permits"]},{"userId":"b","permits":["read","read_comments"]},{"userId":"c","permits":null}]}"#,
            ),
        ])?;
        let shown = run(
            &client,
            &board(),
            &json!({ "board": BOARD, "with": ["rights"] }),
            CacheUse::Use,
        )
        .await?;
        let rights = shown.boards.first().and_then(|one| one.rights.as_ref());
        assert_eq!(
            rights.and_then(|rights| serde_json::to_value(rights).ok()),
            Some(json!([
                { "user": "a", "right": "full" },
                { "user": "b", "right": "viewer" },
                { "user": "c", "right": "none" },
            ]))
        );
        Ok(())
    }

    #[tokio::test]
    async fn one_unknown_board_is_an_error() -> Result<()> {
        let (_, client) = client([missing_reply()])?;
        let failed =
            run(&client, &board(), &json!({ "board": BOARD }), CacheUse::Use).await;
        let closed = failed
            .as_ref()
            .err()
            .is_some_and(crate::error::Error::is_board_closed);
        assert!(closed, "{failed:?}");
        Ok(())
    }

    #[tokio::test]
    async fn mistakes_in_the_flags_are_refused_in_our_words() -> Result<()> {
        let (fake, client) = client([])?;
        let socket = board();
        let cases = [
            json!({ "board": ["A", "B"], "frame": "Q3" }),
            json!({ "board": ["A", "B"], "padding": 4 }),
            json!({ "board": "A", "frame": "Q3", "rectangle": "0,0,5,5" }),
            json!({ "board": [] }),
        ];
        let refused: Vec<_> = join_all(
            cases
                .iter()
                .map(|given| run(&client, &socket, given, CacheUse::Use)),
        )
        .await
        .into_iter()
        .map(|result| result.err().map(|error| error.to_string()))
        .collect();
        assert_eq!(
            (refused, fake.requests().len()),
            (
                vec![
                    Some(
                        "frame of `board show` must be left out with several boards"
                            .to_owned()
                    ),
                    Some(
                        "padding of `board show` must be left out with several boards"
                            .to_owned()
                    ),
                    Some(crate::error::picture::PictureError::SeveralAreas.to_string()),
                    Some("board is required by `board show`".to_owned()),
                ],
                0
            )
        );
        Ok(())
    }

    #[test]
    fn refresh_and_offline_do_not_go_together() -> Result<()> {
        let both = arguments::<ShowArgs>(
            json!({ "board": "A", "refresh": true, "offline": true }),
        )?;
        let refused = cache_use(&both.caching, "board show")
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            refused.as_deref(),
            Some("offline of `board show` must be left out with --refresh")
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_offline_read_is_by_id_only() -> Result<()> {
        let (_, client) = client([])?;
        let failed = run(
            &client,
            &board(),
            &json!({ "board": "Plan", "offline": true }),
            CacheUse::Offline,
        )
        .await;
        assert!(
            matches!(
                &failed,
                Err(crate::Error::Input(
                    crate::error::input::InputError::OfflineByName { .. }
                ))
            ),
            "{failed:?}"
        );
        Ok(())
    }
}
