//! Making a template of a board: the part drawn, its thumbnail uploaded.

use serde_json::{Map, Value, json};

use super::{Action, Audience, Changed, CreateArgs, Plan, carry};
use crate::{
    command::{
        account::account_id,
        board_load::{self, CacheUse, RemoteBoard},
        lookup,
        node::missing_field,
        table::{Confirmation, not_value},
        workspace::{self, ListArgs},
    },
    domain::{
        account::Workspace,
        area::{Area, listed_area},
        board::Object,
        image::ImageFormat,
        lookup::choice,
        node::NodeBrief,
        picture::{self, PictureRequest},
        template::Template,
    },
    effect::{board_write::WriteMode, socket::SocketConnector},
    error::{Error, Result, http::HttpError, input::InputError, picture::PictureError},
    remote::{
        api::{
            self,
            files::{UploadBind, UploadOptions},
        },
        client::Client,
    },
};

const CREATE: &str = "template create";

/// The longest side of a template's thumbnail in pixels.
const THUMBNAIL_SIDE: f64 = 640.0;

/// What a dry run shows where the uploaded thumbnail's name would be.
const UPLOADED_LATER: &str = "(the uploaded thumbnail)";

pub(super) async fn created(
    client: &Client,
    connector: &dyn SocketConnector,
    args: &CreateArgs,
) -> Result<Changed> {
    let name = Some(args.name.trim())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| not_value(CREATE, "name", "text that is not blank"))?;
    let board =
        lookup::board_id(client, &args.board, args.workspace.as_deref()).await?;
    // imperative: the flag is named `for`, a keyword
    let audience = args.r#for.unwrap_or_default();
    let workspace = match audience {
        Audience::Me => None,
        Audience::Workspace => {
            Some(shared_in(client, args.workspace.as_deref()).await?)
        }
    };
    let (content, read) = board_load::content_with_channel(
        RemoteBoard {
            client,
            connector,
            board: &board,
        },
        None,
        CacheUse::Use,
    )
    .await?;
    let area = Area::of(
        None,
        args.frame.clone().filter(|frame| !frame.trim().is_empty()),
        args.object.clone(),
    )?;
    let listed = listed_area(&content.objects, &area, None)?;
    let frame = listed
        .target
        .filter(|_| matches!(area, Area::Frame(_)))
        .map(|frame| frame.id.as_str());
    let held = listed
        .objects
        .iter()
        .filter(|object| Some(object.id.as_str()) != frame)
        .count();
    match held {
        0 => Err(Error::Picture(nothing_in(&area, args))),
        _ => Ok(()),
    }?;
    // The server wants a workspace for every scope, a personal one too.
    let workspace_id = match &workspace {
        Some(workspace) => workspace.id.clone(),
        None => {
            board_workspace(&board_load::channel_or_get(client, &board, read).await?)?
        }
    };
    let thumbnail = match args.dry_run {
        WriteMode::DryRun => UPLOADED_LATER.to_owned(),
        WriteMode::Send => {
            let owner = workspace.as_ref().map(|workspace| workspace.id.as_str());
            thumbnail(client, &content.objects, &area, owner).await?
        }
    };
    // The objects go as the board document keeps them.
    let data = Value::Array(
        listed
            .objects
            .iter()
            .map(|object| object.fields.clone())
            .collect(),
    )
    .to_string();
    let mut fields = Map::from_iter([
        ("name".to_owned(), json!(name)),
        (
            "scope".to_owned(),
            json!(match audience {
                Audience::Me => "personal",
                Audience::Workspace => "workspace",
            }),
        ),
        ("standard".to_owned(), json!(false)),
        (
            "description".to_owned(),
            json!(args.description.as_deref().unwrap_or_default()),
        ),
        ("thumbnail".to_owned(), json!(thumbnail)),
        // Drawn here, not chosen: the cover is not fixed and has no colour.
        ("thumbnail_fixed".to_owned(), json!(false)),
        ("thumbnail_color".to_owned(), Value::Null),
        ("category_ids".to_owned(), json!([])),
        ("data".to_owned(), json!({ "data": data })),
    ]);
    fields.insert("workspace_id".to_owned(), json!(workspace_id));
    let template = Template::try_from(fields)?;
    carry(
        client,
        Plan {
            name: Some(name.to_owned()),
            objects: Some(listed.objects.len()),
            ..Plan::new(api::template::create_request(&template)?, Action::Create)
        },
        (args.dry_run, Confirmation::Missing),
    )
    .await
}

/// The id of the workspace the board lives in.
fn board_workspace(reply: &Value) -> Result<String> {
    NodeBrief::of(reply)?
        .workspace_id()
        .map(str::to_owned)
        .ok_or_else(|| missing_field("workspace_id"))
}

/// The workspace a template is made for. The role is read before anything is
/// drawn or uploaded: the server has no delete for an upload, and a guest
/// cannot share templates.
async fn shared_in(client: &Client, reference: Option<&str>) -> Result<Workspace> {
    let rows = workspace::list(client, &ListArgs { joinable: false }).await?;
    let all: Vec<Workspace> = rows
        .iter()
        .map(|row| Workspace {
            id: row.id.clone(),
            name: row.name.clone(),
        })
        .collect();
    let chosen = choice::workspace(&all, reference)?;
    // A role the server does not word is left to the server.
    let guest = rows
        .iter()
        .any(|row| row.id == chosen.id && row.role.as_deref() == Some("guest"));
    if guest {
        Err(InputError::GuestSharesNoTemplate {
            workspace: chosen.name,
        }
        .into())
    } else {
        Ok(chosen)
    }
}

/// Why the part of the board holds nothing to make a template of.
fn nothing_in(area: &Area, args: &CreateArgs) -> PictureError {
    match area {
        Area::Board => PictureError::EmptyBoard,
        Area::Rectangle(_) | Area::Frame(_) | Area::Objects(_) => {
            PictureError::EmptyArea {
                reference: args.frame.clone().unwrap_or_else(|| args.object.join(", ")),
            }
        }
    }
}

/// Draws the part of the board and uploads the picture, bound to the workspace
/// the template is shared with or else to the user; the name it is stored
/// under.
async fn thumbnail(
    client: &Client,
    objects: &[Object],
    area: &Area,
    workspace: Option<&str>,
) -> Result<String> {
    let request = PictureRequest::new(area.clone(), None, 1.0, THUMBNAIL_SIDE)?;
    let drawn = picture::draw(objects, &request, &client.base_url().storage())?;
    let (file, bytes, format) = thumbnail_file(client, &drawn.picture).await?;
    let user;
    let bind = if let Some(id) = workspace {
        UploadBind::Workspace(id)
    } else {
        user = account_id(client).await?;
        UploadBind::User(&user)
    };
    let options = UploadOptions {
        key: None,
        freshness: None,
        content_type: format.mime(),
    };
    api::files::upload(client, bind, options, file, bytes)
        .await?
        .name
        .ok_or(Error::Http(HttpError::MissingField {
            response: "upload",
            field: "name",
        }))
}

// Renders off the runtime thread.
#[cfg(feature = "png")]
async fn thumbnail_file(
    client: &Client,
    drawn: &picture::Picture,
) -> Result<(&'static str, Vec<u8>, ImageFormat)> {
    let images = crate::command::stored_file::images(
        client,
        &drawn.image_addresses,
        None,
        CacheUse::Use,
    )
    .await?;
    let svg = drawn.svg.clone();
    tokio::task::spawn_blocking(move || crate::system::raster::png_with(&svg, &images))
        .await
        .map_err(|_| Error::Picture(PictureError::PngEncode))?
        .map(|bytes| ("thumbnail.png", bytes, ImageFormat::Png))
}

/// A build without PNG uploads the SVG.
#[cfg(not(feature = "png"))]
fn thumbnail_file(
    _: &Client,
    drawn: &picture::Picture,
) -> impl Future<Output = Result<(&'static str, Vec<u8>, ImageFormat)>> {
    std::future::ready(Ok((
        "thumbnail.svg",
        drawn.svg.clone().into_bytes(),
        ImageFormat::Svg,
    )))
}
