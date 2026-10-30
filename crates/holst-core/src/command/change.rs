//! What is told of a change: sent, dry run, or held; shared by all changes.

use std::any::TypeId;

use serde::Serialize;
use serde_json::Value;

use crate::{
    command::{
        board::{
            rights::{Settled, Shared},
            settings::Edited,
            tree_change,
        },
        comment::CommentReport,
        object::change::Removed,
        table::Confirmation,
        template,
    },
    effect::{
        board_write::WriteMode,
        confirmation::unless_confirmed,
        request::{ApiRequest, QueryParameter},
    },
    error::{Result, confirmation::Unconfirmed},
    remote::client::Client,
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
    #[expect(
        clippy::same_name_method,
        reason = "callers use this one until the runners take `Outcome` over"
    )]
    pub const fn refusal(&self) -> Option<&Unconfirmed> {
        match self {
            Self::Held { refusal, .. } => Some(refusal),
            Self::Sent | Self::DryRun { .. } => None,
        }
    }
}

/// What a shell needs of every change result: the refusal it is held for.
pub trait Outcome {
    fn refusal(&self) -> Option<&Unconfirmed>;
}

impl<P: 'static> Outcome for ChangeReport<P> {
    fn refusal(&self) -> Option<&Unconfirmed> {
        Self::refusal(self)
    }
}

impl Outcome for crate::domain::change::receipt::ChangeReport {
    fn refusal(&self) -> Option<&Unconfirmed> {
        None
    }
}
impl Outcome for Edited {
    fn refusal(&self) -> Option<&Unconfirmed> {
        self.outcome.refusal()
    }
}

impl Outcome for tree_change::Changed {
    fn refusal(&self) -> Option<&Unconfirmed> {
        self.outcome.refusal()
    }
}

impl Outcome for template::Changed {
    fn refusal(&self) -> Option<&Unconfirmed> {
        self.outcome.refusal()
    }
}

impl Outcome for CommentReport {
    fn refusal(&self) -> Option<&Unconfirmed> {
        self.outcome.refusal()
    }
}

impl Outcome for Shared {
    fn refusal(&self) -> Option<&Unconfirmed> {
        match &self.outcome {
            Settled::Carried(report) => report.refusal(),
            Settled::NotHeld { .. } => None,
        }
    }
}

impl Outcome for Removed {
    fn refusal(&self) -> Option<&Unconfirmed> {
        self.refusal.as_ref()
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

/// A change under the confirmation rule: `Err` is the report of one that stays
/// (held for its confirmation, or a dry run; a dry run needs none), with
/// `plan`; `Ok` means carry it now.
pub(in crate::command) fn gate<P: 'static>(
    unconfirmed: Option<Unconfirmed>,
    (mode, confirm): (WriteMode, Confirmation),
    plan: impl FnOnce() -> P,
) -> std::result::Result<(), ChangeReport<P>> {
    match (
        unless_confirmed(unconfirmed, confirm == Confirmation::Given, mode),
        mode,
    ) {
        (Err(refusal), _) => Err(ChangeReport::Held {
            request: plan(),
            needs: refusal.needs(),
            refusal,
        }),
        (Ok(()), WriteMode::DryRun) => Err(ChangeReport::DryRun { request: plan() }),
        (Ok(()), WriteMode::Send) => Ok(()),
    }
}

/// Sends the request, reports it for a dry run, or holds it for want of its
/// confirmation. The reply is there when it was sent.
pub(in crate::command) async fn settle(
    client: &Client,
    request: ApiRequest,
    unconfirmed: Option<Unconfirmed>,
    confirmation: (WriteMode, Confirmation),
) -> Result<(ChangeReport, Option<Value>)> {
    match gate(unconfirmed, confirmation, || PlannedRequest::from(&request)) {
        Err(stays) => Ok((stays, None)),
        Ok(()) => Ok((ChangeReport::Sent, Some(client.send_json(request).await?))),
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
        command::{
            change::{ChangeReport, Outcome, PlannedRequest, gate},
            table::Confirmation,
        },
        effect::{
            board_write::WriteMode,
            http::{HttpBody, HttpMethod},
            request::{ApiRequest, Authorization, QueryParameter},
        },
        error::{
            Result,
            confirmation::{ChangeRisk, Unconfirmed},
        },
    };

    fn risky() -> Unconfirmed {
        Unconfirmed::new("deleting it", ChangeRisk::Irreversible)
    }

    #[test]
    fn an_unconfirmed_change_is_held_with_what_it_needs() -> Result<()> {
        let stays = gate(
            Some(risky()),
            (WriteMode::Send, Confirmation::Missing),
            || (),
        );
        assert_eq!(
            serde_json::to_value(stays.err())?,
            json!({ "status": "held", "needs": "deleting it cannot be undone" })
        );
        Ok(())
    }

    #[test]
    fn a_dry_run_needs_no_confirmation_and_stays_unsent() {
        let stays = gate(
            Some(risky()),
            (WriteMode::DryRun, Confirmation::Missing),
            || (),
        );
        assert!(
            matches!(stays, Err(ChangeReport::DryRun { .. })),
            "{stays:?}"
        );
    }

    #[test]
    fn a_confirmed_or_harmless_change_is_carried() {
        let confirmed =
            gate(Some(risky()), (WriteMode::Send, Confirmation::Given), || ());
        let harmless = gate(None, (WriteMode::Send, Confirmation::Missing), || ());
        assert!(confirmed.is_ok() && harmless.is_ok());
    }

    #[test]
    fn a_held_change_gives_its_refusal_and_a_dry_run_none() {
        let held = ChangeReport::<()>::Held {
            request: (),
            needs: "deleting it cannot be undone".into(),
            refusal: risky(),
        };
        let dry = ChangeReport::<()>::DryRun { request: () };
        assert_eq!(
            (Outcome::refusal(&held), Outcome::refusal(&dry)),
            (Some(&risky()), None)
        );
    }

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
