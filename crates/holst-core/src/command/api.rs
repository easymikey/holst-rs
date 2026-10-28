//! `holst api`: a raw request for endpoints no command covers yet.

use holst_macros::CommandArgs;
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
        Result,
        confirmation::{ChangeRisk, Unconfirmed},
    },
    remote::client::Client,
};

const COMMAND: &str = "api";

/// The HTTP methods `holst api` sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
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
            Method::Put => HttpMethod::Put,
            Method::Patch => HttpMethod::Patch,
            Method::Delete => HttpMethod::Delete,
        }
    }
}

#[derive(Debug, Deserialize, CommandArgs)]
#[holst_command(
    group = Api,
    risk = Destroy,
    about = "Send any request to the Holst API",
    help = "Send any request to the Holst API, for an endpoint no command covers.\n\
            GET is sent at once. Any other method may change what cannot be undone, so it needs -y; without it only the dry run is shown.",
    example = "holst api GET /gapi/current_user",
    example = "holst api PATCH /gapi/channel/<id> --body @body.json -y"
)]
pub struct ApiArgs {
    #[argument(positional, help = "The HTTP method: GET, POST, PUT, PATCH or DELETE")]
    pub method: Method,
    #[argument(
        positional,
        help = "The path from the root, for example /gapi/current_user"
    )]
    pub path: String,
    #[argument(
        letter = 'q',
        value_name = "key=value",
        help = "A query parameter, repeat for several"
    )]
    #[serde(default, deserialize_with = "query_object")]
    pub query: Vec<QueryParameter>,
    #[argument(
        value_name = "body",
        help = "The request body: a JSON string, @file to read a file, - for stdin"
    )]
    pub body: Option<TextOrFile>,
    #[argument(
        letter = 'y',
        help = "Confirm a change other than GET; without it only the dry run is shown (exit code 77)"
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

/// Sends the request; a method other than GET without `confirm` is held.
///
/// # Errors
/// A path that is not rooted or is a full address, a body that is not JSON,
/// and errors of the request.
pub async fn request(client: &Client, args: &ApiArgs) -> Result<ApiReply> {
    let path = args.path.parse::<ApiPath>()?;
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
        Unconfirmed::new(
            format!("sending {method} {}", path.as_str()),
            ChangeRisk::Irreversible,
        )
    });
    let request = ApiRequest {
        method,
        path: path.as_str().to_owned(),
        query: args.query.clone(),
        body,
        authorization: Authorization::Session,
    };
    let (report, reply) =
        settle(client, request, unconfirmed, (args.dry_run, args.confirm)).await?;
    Ok(ApiReply { report, reply })
}

#[cfg(test)] mod tests;
