//! The raw API request: `api`, and the switch that lets it write.

use holst_core::{
    command::{
        api::{self as operation_api, ApiArgs, ApiReply, refuse_admin_write},
        change::ChangeReport,
        table::parse,
    },
    effect::board_write::WriteMode,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value};

use crate::{
    error::{Error, Result},
    login::HandedClient,
    memory::alias::{expand_json, expand_path},
    server::McpServer,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawWrite {
    Allowed,
    Denied,
}

impl RawWrite {
    pub(crate) fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("1") => RawWrite::Allowed,
            Some(_) | None => RawWrite::Denied,
        }
    }
}

impl McpServer {
    /// Sends one `api` request. `@N` is expanded in the path, in the value of
    /// each query item and in the strings of the body.
    pub(super) async fn api_command(
        &self,
        arguments: Map<String, Value>,
    ) -> CallToolResult {
        match self
            .expanded_api(arguments)
            .and_then(|each| Ok(parse::<ApiArgs>(each)?))
        {
            Ok(args) => self.send_api(&args).await,
            Err(err) => self.fail(&err),
        }
    }

    async fn send_api(&self, args: &ApiArgs) -> CallToolResult {
        let method = args.method.http();
        match (
            refuse_admin_write(args),
            method.is_read(),
            args.dry_run,
            self.raw_write,
        ) {
            (Err(refusal), ..) => self.fail(&Error::from(refusal)),
            (Ok(()), false, WriteMode::Send, RawWrite::Denied) => {
                self.fail(&Error::RawWriteDisabled { method })
            }
            // A read keeps what the server remembers.
            (Ok(()), true, ..) => {
                self.run(async |client: HandedClient| {
                    let sent = operation_api::request(&client, args).await?;
                    Ok(self.api_text(&sent))
                })
                .await
            }
            (Ok(()), false, ..) => {
                self.changed(args, operation_api::request, |this, sent| {
                    this.api_text(sent)
                })
                .await
            }
        }
    }

    fn api_text(&self, sent: &ApiReply) -> String {
        match &sent.report {
            ChangeReport::Sent => {
                self.state.render(sent.reply.clone().unwrap_or(Value::Null))
            }
            ChangeReport::DryRun { request } => self
                .state
                .alias_text(&format!("would send\n{}\n", request.lines().join("\n"))),
            // A held request is answered by its refusal before the text is asked.
            ChangeReport::Held { .. } => String::new(),
        }
    }

    fn expanded_api(
        &self,
        mut arguments: Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        let expand = |reference: &str| self.expand(reference);
        if let Some(Value::String(path)) = arguments.get_mut("path") {
            *path = expand_path(path, expand)?;
        }
        if let Some(param @ Value::Object(_)) = arguments.get_mut("param") {
            expand_json(param, &expand)?;
        }
        // A string that is a whole `@N` is expanded; a body not JSON is left.
        if let Some(Value::String(text)) = arguments.get_mut("body")
            && let Ok(mut body) = serde_json::from_str::<Value>(text)
        {
            expand_json(&mut body, &expand)?;
            *text = body.to_string();
        }
        Ok(arguments)
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::admin::RawWrite;

    #[rstest::rstest]
    #[case::enabled(Some("1"), RawWrite::Allowed)]
    #[case::unset(None, RawWrite::Denied)]
    #[case::other(Some("true"), RawWrite::Denied)]
    #[case::empty(Some(""), RawWrite::Denied)]
    fn raw_write_needs_exactly_one(
        #[case] value: Option<&str>,
        #[case] expected: RawWrite,
    ) {
        assert_eq!(RawWrite::from_setting(value), expected);
    }
}
