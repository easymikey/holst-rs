//! The `holst-mcp` binary reached through a real MCP client.

use std::process::Command;

use rmcp::{
    ServiceExt as _,
    model::{CallToolRequestParams, CallToolResult},
    transport::TokioChildProcess,
};
use serde_json::{Map, Value};

use crate::{
    error::ScenarioError,
    observation::{ErrorExit, Observation},
};

const HINT_SEPARATOR: &str = "\nhint: ";

/// What a tool answered: its text and the types of its images.
pub(crate) struct Reply {
    pub(crate) text: String,
    pub(crate) image_types: Vec<String>,
}

/// Calls `tool` with `arguments`; a reply is read by `observe`, a tool error
/// is read here, the same for every tool.
pub(crate) async fn call(
    command: Command,
    tool: &'static str,
    arguments: Map<String, Value>,
    observe: impl FnOnce(&Reply) -> Observation,
) -> Result<Observation, ScenarioError> {
    call_then(command, tool, arguments, |_| None, observe).await
}

/// Calls `tool`, then the call `next` builds from its reply, in one session:
/// an `@N` alias is good only in the session that handed it out. The reply of
/// the last call is read by `observe`.
pub(crate) async fn call_then(
    command: Command,
    tool: &'static str,
    arguments: Map<String, Value>,
    next: impl FnOnce(&Reply) -> Option<(&'static str, Map<String, Value>)>,
    observe: impl FnOnce(&Reply) -> Observation,
) -> Result<Observation, ScenarioError> {
    let _running = crate::cli::RUNNING.acquire().await;
    let client = ()
        .serve(TokioChildProcess::new(tokio::process::Command::from(
            command,
        ))?)
        .await?;
    let first = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await;
    let follow = first
        .as_ref()
        .ok()
        .filter(|reply| failure(reply).is_none())
        .and_then(|reply| next(&read(reply)));
    let second = match follow {
        Some((second_tool, second_arguments)) => Some(
            client
                .call_tool(
                    CallToolRequestParams::new(second_tool)
                        .with_arguments(second_arguments),
                )
                .await,
        ),
        None => None,
    };
    client.cancel().await?;
    let reply = second.unwrap_or(first)?;
    Ok(failure(&reply).unwrap_or_else(|| observe(&read(&reply))))
}

fn read(reply: &CallToolResult) -> Reply {
    Reply {
        text: text_of(reply),
        image_types: reply
            .content
            .iter()
            .filter_map(|block| block.as_image())
            .map(|image| image.mime_type.clone())
            .collect(),
    }
}

pub(crate) fn text_of(reply: &CallToolResult) -> String {
    reply
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn failure(reply: &CallToolResult) -> Option<Observation> {
    (reply.is_error == Some(true))
        .then(|| text_of(reply))
        .map(|text| {
            let (message, hint) =
                text.split_once(HINT_SEPARATOR).unwrap_or((&text, ""));
            Observation::Failed {
                message: message.to_owned(),
                hint: hint.to_owned(),
                signal: ErrorExit::ToolError,
            }
        })
}
