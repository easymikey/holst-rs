//! `holst api`: a raw request for endpoints no command covers yet.

use holst_macros::CommandArgs;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    command::{
        change::{ChangeReport, settle},
        table::{Confirmation, EnumArgument, TextOrFile, not_value, query_object},
    },
    domain::url::ApiPath,
    effect::{
        board_write::WriteMode,
        http::{HttpBody, HttpMethod},
        request::{ApiRequest, Authorization, QueryParameter},
    },
    error::{
        Error,
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
        input::InputError,
    },
    remote::{
        catalogue::{Body, Class, Endpoint, Need, Query, Refusal, check_body, find},
        client::Client,
    },
};

const COMMAND: &str = "api";

/// The HTTP methods `holst api` sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Patch,
    Delete,
}

impl EnumArgument for Method {}

impl Method {
    /// The method as the transport names it.
    #[must_use]
    pub const fn http(self) -> HttpMethod {
        match self {
            Method::Get => HttpMethod::Get,
            Method::Post => HttpMethod::Post,
            Method::Patch => HttpMethod::Patch,
            Method::Delete => HttpMethod::Delete,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Api,
    risk = Destroy,
    about = "Send a request the Holst API catalogue knows",
    help = "Send a request the Holst API catalogue knows, for an endpoint no command covers. Only known calls are sent, never money or login calls.\n\
            GET is sent at once. Any other method may change what cannot be undone, so it needs -y; without it only the dry run is shown.",
    example = "holst api GET /gapi/current_user",
    example = "holst api PATCH /gapi/channel/<id> --body @body.json -y"
)]
pub struct ApiArgs {
    #[argument(positional, help = "The HTTP method: GET, POST, PATCH or DELETE")]
    pub method: Method,
    #[argument(
        positional,
        help = "The path from the root, for example /gapi/current_user"
    )]
    pub path: String,
    #[argument(
        letter = 'p',
        value_name = "key=value",
        help = "A query parameter, repeat for several"
    )]
    #[serde(default, deserialize_with = "query_object")]
    pub param: Vec<QueryParameter>,
    #[argument(
        value_name = "body",
        help = "The request body: a JSON string, @file to read a file, - for stdin"
    )]
    pub body: Option<TextOrFile>,
    #[argument(
        letter = 'y',
        help = "Confirm a change other than GET; a terminal asks instead; without one only the dry run is shown (exit code 77)"
    )]
    pub confirm: Confirmation,
    #[argument(letter = 'n', help = "Show the request; send nothing")]
    pub dry_run: WriteMode,
}

/// What came of a raw request.
#[derive(Debug, Serialize)]
pub struct ApiReply {
    #[serde(flatten)]
    pub report: ChangeReport,
    /// What the server answered, when the request was sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<Value>,
}

fn refused(refusal: Refusal) -> Error {
    Error::Input(match refusal {
        Refusal::Unknown { method, path } if is_login(&path) => {
            InputError::LoginCall { method, path }
        }
        other @ (Refusal::Unknown { .. }
        | Refusal::Method { .. }
        | Refusal::Segment { .. }
        | Refusal::Query { .. }
        | Refusal::NoBody { .. }
        | Refusal::Body { .. }) => InputError::Refused(other),
    })
}

/// Refuse a body that no typed entry reads as its type; an untyped entry reads
/// any.
fn check_typed(found: &[&Endpoint], text: &str) -> std::result::Result<(), Refusal> {
    use std::result::Result;

    let outcome = found
        .iter()
        .filter_map(|endpoint| match endpoint.body {
            Body::Typed(check) => Some(check(text)),
            Body::Json => Some(Ok(())),
            Body::None => None,
        })
        .reduce(Result::or)
        .and_then(Result::err);
    found
        .first()
        .zip(outcome)
        .map_or(Ok(()), |(endpoint, error)| {
            Err(Refusal::Body {
                method: endpoint.method,
                template: endpoint.template,
                error,
            })
        })
}

fn is_login(path: &str) -> bool {
    matches!(
        path.strip_prefix("/gapi/")
            .and_then(|rest| rest.split(['/', '?']).next()),
        Some("login" | "logout" | "send_temporary_password")
    )
}

/// The catalogue entries the request names; nothing else is sent.
fn catalogued(args: &ApiArgs, path: &ApiPath) -> Result<Vec<&'static Endpoint>> {
    let method = args.method.http();
    let (route, inline) = path.as_str().split_once('?').unwrap_or((path.as_str(), ""));
    let items = args.param.iter().map(|pair| pair.name.as_str());
    let wire = format!("{route}?{inline}&{}", items.clone().join("&"));
    let found = find(method, &wire).map_err(refused)?;
    check_body(&found, args.body.is_some()).map_err(refused)?;
    args.body
        .as_ref()
        .map_or(Ok(()), |text| check_typed(&found, &text.0))
        .map_err(refused)?;
    let given: Vec<&str> = inline
        .split('&')
        .map(|pair| pair.split_once('=').map_or(pair, |(key, _)| key))
        .chain(items)
        .collect();
    let missing = found.first().and_then(|endpoint| match endpoint.query {
        Query::Names(names) => names
            .iter()
            .find(|(name, need)| *need == Need::Required && !given.contains(name))
            .map(|(name, _)| (endpoint.template, *name)),
        Query::Any => None,
    });
    match missing {
        Some((template, key)) => Err(Error::Input(InputError::MissingQuery {
            method,
            template,
            key,
        })),
        None => Ok(found),
    }
}

/// What a held call risks: sharing or the whole workspace for an admin call.
fn risk(found: &[&Endpoint]) -> ChangeRisk {
    match found.first() {
        Some(Endpoint {
            class: Class::Admin,
            template,
            ..
        }) if template.starts_with("/gapi/workspace") => ChangeRisk::Everyone,
        Some(Endpoint {
            class: Class::Admin,
            ..
        }) => ChangeRisk::Sharing,
        Some(_) | None => ChangeRisk::Irreversible,
    }
}

/// Refuses a change of access or workspace settings: MCP sends none through
/// `api`.
///
/// # Errors
/// The refusals of [`request`], and [`InputError::AdminWrite`].
pub fn refuse_admin_write(args: &ApiArgs) -> Result<()> {
    let path = args.path.parse::<ApiPath>()?;
    let found = catalogued(args, &path)?;
    match found.first() {
        Some(Endpoint {
            class: Class::Admin,
            template,
            ..
        }) if !args.method.http().is_read() => {
            Err(Error::Input(InputError::AdminWrite {
                method: args.method.http(),
                template,
            }))
        }
        Some(_) | None => Ok(()),
    }
}

/// Sends the request; a method other than GET without `confirm` is held.
///
/// # Errors
/// A path that is not rooted or is a full address, a call outside the
/// catalogue (its method, an id, a query key or the body), a body that is not
/// JSON, and errors of the request.
pub async fn request(client: &Client, args: &ApiArgs) -> Result<ApiReply> {
    let path = args.path.parse::<ApiPath>()?;
    let found = catalogued(args, &path)?;
    let body = args
        .body
        .as_ref()
        .map(|text| {
            serde_json::from_str::<Value>(&text.0)
                .map_err(|_| not_value(COMMAND, "body", "JSON text"))
        })
        .transpose()?
        .map(|body| HttpBody::json(&body))
        .transpose()?;
    let method = args.method.http();
    let unconfirmed = (!method.is_read()).then(|| {
        Unconfirmed::new(format!("sending {method} {}", path.as_str()), risk(&found))
    });
    let request = ApiRequest {
        method,
        path: path.as_str().to_owned(),
        query: args.param.clone(),
        body,
        authorization: Authorization::Session,
    };
    let (report, reply) =
        settle(client, request, unconfirmed, (args.dry_run, args.confirm)).await?;
    Ok(ApiReply { report, reply })
}

#[cfg(test)] mod tests;
