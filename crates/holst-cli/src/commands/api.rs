//! `holst api`: a raw request for endpoints no command covers yet.

use holst_core::{
    command::{
        api::{self, ApiArgs, ApiReply, Method},
        change::{ChangeReport, PlannedRequest},
        table,
    },
    domain::text::{hide_tokens, printable},
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
        ChangeReport::Sent => output::reply(format, kind, &sent_reply(done)),
        ChangeReport::DryRun { request } => {
            shown(format, &done.report, Some(DRY_RUN), request)
        }
        ChangeReport::Held { request, .. } => {
            shown(format, &done.report, None, request)
        }
    }
}

/// What the server answered; token values stay hidden, as in MCP `api`.
fn sent_reply(done: &ApiReply) -> Value {
    hide_tokens(done.reply.clone().unwrap_or(Value::Null))
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

#[cfg(test)]
mod tests {
    use holst_core::command::{api::ApiReply, change::ChangeReport};
    use serde_json::json;

    use crate::commands::api::sent_reply;

    #[test]
    fn a_reply_with_a_token_value_is_printed_hidden() {
        let done = ApiReply {
            report: ChangeReport::Sent,
            reply: Some(json!([{ "name": "ci", "token": "holst_pat_planted_fake" }])),
        };
        assert_eq!(sent_reply(&done), json!([{ "name": "ci", "token": "***" }]));
    }
}
