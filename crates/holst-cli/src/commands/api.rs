//! `holst api`: a raw request for endpoints no command covers yet.

use holst_core::{
    command::{
        api::{self, ApiArgs, ApiReply, Method},
        change::{ChangeReport, PlannedRequest},
        table,
    },
    domain::text::printable,
    effect::request::DRY_RUN,
};
use serde_json::{Map, Value};

use crate::{
    commands,
    error::Result,
    output::{self, OutputFormat, ReplyKind},
    run::RunSettings,
};

/// Runs `api` with the arguments the command line gave it. A request held for
/// its confirmation is shown first; the refusal is the exit code.
pub(crate) async fn run(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let kind = match table::parse::<ApiArgs>(arguments.clone())?.method {
        Method::Get => ReplyKind::Read,
        Method::Post | Method::Patch | Method::Delete => ReplyKind::Change,
    };
    commands::change(arguments, run, format, api::request, |format, done| {
        render(format, kind, done)
    })
    .await
}

fn render(format: OutputFormat, kind: ReplyKind, done: &ApiReply) -> Result<()> {
    match &done.report {
        ChangeReport::Sent => {
            output::reply(format, kind, done.reply.as_ref().unwrap_or(&Value::Null))
        }
        ChangeReport::DryRun { request } => {
            shown(format, &done.report, Some(DRY_RUN), request)
        }
        ChangeReport::Held { request, .. } => {
            shown(format, &done.report, None, request)
        }
    }
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
