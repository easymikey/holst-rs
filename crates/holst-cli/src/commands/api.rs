//! `holst api`: a raw request for endpoints no command covers yet.

use holst_core::{
    command::{
        api::{self, ApiReply, Method},
        change::{ChangeReport, PlannedRequest},
        table,
    },
    domain::text::printable,
    effect::request::DRY_RUN,
};
use serde_json::{Map, Value};

use crate::{
    commands::refused,
    error::Result,
    output::{self, OutputFormat, ReplyKind},
    run::{RunSettings, run_signed_in},
};

/// Runs `api` with the arguments the command line gave it. A request held for
/// its confirmation is shown first; the refusal is the exit code.
pub(crate) async fn run(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let args: api::ApiArgs = table::parse(arguments)?;
    let kind = match args.method {
        Method::Get => ReplyKind::Read,
        Method::Post | Method::Put | Method::Patch | Method::Delete => {
            ReplyKind::Change
        }
    };
    run_signed_in(run, async |client| {
        let ApiReply { report, reply } = api::request(client, &args).await?;
        match &report {
            ChangeReport::Sent => {
                output::reply(format, kind, &reply.unwrap_or(Value::Null))?;
            }
            ChangeReport::DryRun { request } => {
                shown(format, &report, Some(DRY_RUN), request)?;
            }
            ChangeReport::Held { request, .. } => {
                shown(format, &report, None, request)?;
            }
        }
        refused(report.refusal())
    })
    .await
}

/// Prints the request that was not sent.
fn shown(
    format: OutputFormat,
    report: &ChangeReport,
    head: Option<&str>,
    request: &PlannedRequest,
) -> Result<()> {
    match format {
        OutputFormat::Text => output::line(printable(
            &head
                .into_iter()
                .map(str::to_owned)
                .chain(request.lines())
                .collect::<Vec<_>>()
                .join("\n"),
        )),
        OutputFormat::Pretty | OutputFormat::Json => {
            output::print_value(format, report)
        }
    }
}
