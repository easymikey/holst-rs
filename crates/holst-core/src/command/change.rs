//! What is told of a change: sent, dry run, or held; shared by all changes.

use std::any::TypeId;

use serde::Serialize;
use serde_json::Value;

use crate::{
    command::table::Confirmation,
    effect::{
        board_write::WriteMode,
        confirmation::unless_confirmed,
        interpreter::{CarriedRequest, RequestInterpreter as _},
        request::{ApiRequest, QueryParameter},
    },
    error::{Result, confirmation::Unconfirmed},
    remote::client::{
        Client,
        interpreter::{RequestReporter, RequestSender},
    },
};

/// What is told of a change; `P` is the plan of what would go out.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChangeReport<P: 'static = PlannedRequest> {
    /// The request went out.
    Sent,
    /// A dry run: this plan would go out; a `()` plan is left out.
    DryRun {
        #[serde(skip_serializing_if = "unplanned")]
        request: P,
    },
    /// No confirmation: unsent; `needs` is the risk, the shell adds the flag.
    Held {
        #[serde(skip_serializing_if = "unplanned")]
        request: P,
        needs: String,
        /// The same risk as a refusal, for the shell to word.
        #[serde(skip)]
        refusal: Unconfirmed,
    },
}

fn unplanned<P: 'static>(_: &P) -> bool {
    TypeId::of::<P>() == TypeId::of::<()>()
}

impl<P: 'static> ChangeReport<P> {
    /// The refusal of a change held for its confirmation.
    #[must_use]
    pub const fn refusal(&self) -> Option<&Unconfirmed> {
        match self {
            Self::Held { refusal, .. } => Some(refusal),
            Self::Sent | Self::DryRun { .. } => None,
        }
    }
}

/// A request not sent, as data, in the shape of the old CLI's JSON.
#[derive(Debug, Serialize)]
pub struct PlannedRequest {
    pub method: &'static str,
    pub path: String,
    pub query: Vec<QueryParameter>,
    pub body: Option<Value>,
}

impl PlannedRequest {
    /// The request as lines for a shell: `METHOD path?query`, then the body.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let query = if self.query.is_empty() {
            String::new()
        } else {
            format!(
                "?{}",
                self.query
                    .iter()
                    .map(|QueryParameter { name, value }| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join("&")
            )
        };
        [format!("{} {}{query}", self.method, self.path)]
            .into_iter()
            .chain(self.body.as_ref().map(ToString::to_string))
            .collect()
    }
}

impl From<&ApiRequest> for PlannedRequest {
    /// The body of a login stays out, as in the request's `Display`.
    fn from(request: &ApiRequest) -> Self {
        let body = request.body.as_ref().map(|body| {
            let hidden = || {
                Value::from(format!(
                    "({} bytes of {})",
                    body.bytes.len(),
                    body.content_type
                ))
            };
            if request.hides_body() {
                hidden()
            } else {
                serde_json::from_slice(&body.bytes).unwrap_or_else(|_| hidden())
            }
        });
        Self {
            method: request.method.as_str(),
            path: request.path.clone(),
            query: request.query.clone(),
            body,
        }
    }
}

/// Sends the request, reports it for a dry run, or holds it for want of its
/// confirmation (a dry run needs none). The reply is there when it was sent.
pub(in crate::command) async fn settle(
    client: &Client,
    request: ApiRequest,
    unconfirmed: Option<Unconfirmed>,
    (dry_run, confirm): (WriteMode, Confirmation),
) -> Result<(ChangeReport, Option<Value>)> {
    let mode = dry_run;
    if let Err(needs) =
        unless_confirmed(unconfirmed, confirm == Confirmation::Given, mode)
    {
        let held = ChangeReport::Held {
            request: PlannedRequest::from(&request),
            needs: format!("{} {}", needs.change, needs.risk),
            refusal: needs,
        };
        Ok((held, None))
    } else {
        let carried = match dry_run {
            WriteMode::DryRun => RequestReporter.carry(client, request).await?,
            WriteMode::Send => RequestSender.carry(client, request).await?,
        };
        Ok(match carried {
            CarriedRequest::Reported(reported) => (
                ChangeReport::DryRun {
                    request: PlannedRequest::from(&reported),
                },
                None,
            ),
            CarriedRequest::Sent(reply) => (ChangeReport::Sent, Some(reply)),
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "asserting is the point of a test"
    )]

    use serde_json::json;

    use crate::{
        command::change::PlannedRequest,
        effect::{
            http::{HttpBody, HttpMethod},
            request::{ApiRequest, Authorization, QueryParameter},
        },
        error::Result,
    };

    fn request(path: &str, authorization: Authorization) -> Result<ApiRequest> {
        Ok(ApiRequest {
            method: HttpMethod::Post,
            path: path.into(),
            query: vec![QueryParameter {
                name: "k".into(),
                value: "v".into(),
            }],
            body: Some(HttpBody::json(
                &json!({ "email": "me@example.com", "password": "s3cr3t" }),
            )?),
            authorization,
        })
    }

    #[test]
    fn a_planned_login_request_holds_no_password() -> Result<()> {
        let login = request("/gapi/login/with_email", Authorization::Anonymous)?;
        let shown = serde_json::to_string(&PlannedRequest::from(&login))?;
        assert!(!shown.contains("s3cr3t"), "{shown}");
        assert!(!shown.contains("me@example.com"), "{shown}");
        Ok(())
    }

    #[test]
    fn a_planned_request_keeps_query_and_json_body() -> Result<()> {
        let plain = request("/gapi/x", Authorization::Session)?;
        assert_eq!(
            serde_json::to_value(PlannedRequest::from(&plain))?,
            json!({
                "method": "POST",
                "path": "/gapi/x",
                "query": [["k", "v"]],
                "body": { "email": "me@example.com", "password": "s3cr3t" },
            })
        );
        Ok(())
    }
}
