//! Tool replies; failures are tool errors: clients hide protocol errors.

pub(crate) mod change;
pub(crate) mod compact;
pub(crate) mod objects;
pub(crate) mod render;

use rmcp::model::{CallToolResult, ContentBlock};

use crate::error::Error;

pub(crate) fn text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

pub(crate) fn error(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text.into())])
}

#[must_use]
pub(crate) fn explain(err: &Error) -> String {
    let text = holst_core::error::chain(err)
        .fold(message(err), |text, cause| format!("{text}: {cause}"));
    err.advice()
        .map_or_else(|| text.clone(), |hint| format!("{text}\nhint: {hint}"))
}

fn message(err: &Error) -> String {
    match err {
        Error::Holst(core) => core.on(holst_core::error::hint::Surface::Mcp),
        Error::Alias(_)
        | Error::EmptyQuery
        | Error::NoArgument { .. }
        | Error::TooManyItems { .. }
        | Error::NoStdout
        | Error::CliOnly { .. }
        | Error::Unconfirmed(_)
        | Error::RawWriteDisabled { .. }
        | Error::Unreadable { .. }
        | Error::NotAFile
        | Error::Secret { .. }
        | Error::SecretPlace { .. }
        | Error::JsonTooLong { .. }
        | Error::StoreAborted => err.to_string(),
    }
}

#[must_use]
pub(crate) fn chained(err: &(dyn std::error::Error + 'static)) -> String {
    holst_core::error::chain(err)
        .fold(err.to_string(), |text, cause| format!("{text}: {cause}"))
}

#[cfg(test)]
mod tests {
    use holst_core::{
        effect::http::HttpMethod,
        error::{
            hint::{Hint, Surface},
            lookup::{
                LookupError,
                LookupKind,
                LookupMiss,
                SkipReason,
                Skipped,
                SkippedWorkspaces,
            },
        },
        fake::candidates,
    };
    use strum::IntoEnumIterator as _;

    use crate::{error::Error, reply::explain};

    fn status_error(status: u16) -> Error {
        Error::Holst(holst_core::Error::Http(
            holst_core::error::http::HttpError::Status {
                method: "GET",
                path: "/gapi/current_user".into(),
                status,
                detail: String::new(),
            },
        ))
    }

    #[rstest::rstest]
    #[case::revoked(401)]
    #[case::forbidden(403)]
    #[case::missing(404)]
    #[case::down(503)]
    fn hint_follows_the_core(#[case] status: u16) {
        let err = status_error(status);
        let hint = err.advice().map(|hint| hint.to_string());
        assert!(hint.is_some(), "no hint for {status}");
        assert_eq!(
            explain(&err).lines().last(),
            hint.map(|hint| format!("hint: {hint}")).as_deref()
        );
    }

    #[test]
    fn an_unconfirmed_board_write_asks_to_read_before_a_retry() {
        let err = Error::Holst(holst_core::Error::BoardChange(
            holst_core::error::board_change::BoardChangeError::NotAccepted {
                update: 42,
                objects: vec!["s1".into(), "a1".into()],
                cause: Box::new(holst_core::Error::BoardChange(
                    holst_core::error::board_change::BoardChangeError::ClosedBeforeAccept,
                )),
            },
        ));
        let text = explain(&err);
        assert!(
            text.starts_with(
                "update 42 (objects s1, a1) was sent but the server did not accept it"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "may or may not be on the board: the board connection closed"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "\nhint: call the object tool to see whether the change is on the board"
            ),
            "{text}"
        );
    }

    #[test]
    fn own_errors_have_no_hint_line() {
        assert_eq!(
            explain(&Error::NoStdout),
            "the original goes into a new file: there is no standard output here"
        );
    }

    #[test]
    fn unreadable_file_names_its_cause() {
        let source = std::io::Error::from(std::io::ErrorKind::NotFound);
        let cause = source.to_string();
        let err = Error::Unreadable { source };
        assert_eq!(explain(&err), format!("the file cannot be read: {cause}"));
    }

    fn lookups() -> impl Iterator<Item = (LookupKind, Error)> {
        let outcomes = || {
            [
                LookupMiss::not_found(),
                LookupMiss::ambiguous(candidates(&["@1 a", "@2 b"])),
                LookupMiss::Uncertain {
                    candidates: candidates(&["@1 a"]),
                    unread: SkippedWorkspaces(vec![Skipped {
                        workspace: "team".into(),
                        reason: SkipReason::NotRead,
                        status: Some(500),
                    }]),
                },
                LookupMiss::Elsewhere {
                    workspace: "easy".into(),
                },
            ]
        };
        LookupKind::iter().flat_map(move |what| {
            outcomes().into_iter().map(move |outcome| {
                let lookup = LookupError {
                    kind: what,
                    reference: "Plans".into(),
                    miss: outcome,
                };
                (what, Error::Holst(holst_core::Error::Lookup(lookup)))
            })
        })
    }

    #[test]
    fn mcp_hints_never_name_cli_commands_or_flags() {
        let errors = lookups()
            .map(|(_, err)| err)
            .chain([401, 403, 404, 429, 500, 503].map(status_error))
            .chain([Error::Unconfirmed(
                holst_core::error::confirmation::Unconfirmed::new(
                    "deleting a template",
                    holst_core::error::confirmation::ChangeRisk::Irreversible,
                ),
            )]);
        let hints: Vec<String> = errors
            .filter_map(|err| err.advice().map(|hint| hint.to_string()))
            .collect();
        let cli: Vec<&String> = hints
            .iter()
            .filter(|hint| {
                // The one command the model may name is the server's own.
                let hint = hint.replace("`holst-mcp setup`", "");
                hint.contains("holst ") || hint.contains("--") || hint.contains('`')
            })
            .collect();
        assert!(hints.len() > 20, "{hints:?}");
        assert_eq!(cli, Vec::<&String>::new());
        let all: Vec<String> = Hint::iter()
            .map(|hint| hint.on(Surface::Mcp).to_string())
            .filter(|text| text.contains("holst ") || text.contains("--"))
            .collect();
        assert_eq!(all, Vec::<String>::new());
    }

    #[test]
    fn lookup_errors_name_their_kind() {
        let unnamed: Vec<String> = lookups()
            .map(|(what, err)| (what, explain(&err)))
            .filter(|(what, text)| {
                !text.contains(&what.to_string()) && !text.contains(what.plural())
            })
            .map(|(_, text)| text)
            .collect();
        assert_eq!(unnamed, Vec::<String>::new());
        let folder = lookups()
            .find(|(what, _)| *what == LookupKind::Folder)
            .map(|(_, err)| explain(&err));
        assert_eq!(
            folder.as_deref(),
            Some(
                "folder \"Plans\" not found\nhint: call board action=ls to see the boards and folders"
            )
        );
    }

    #[test]
    fn mcp_messages_name_parameters_not_flags() {
        let plain_gz = Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::PlainGz {
                path: "board.ydoc.gz".into(),
            },
        ));
        let text = explain(&plain_gz);
        assert!(text.contains("uncompressed: true"), "{text}");
        let outside = Error::Holst(holst_core::Error::BoardChange(
            holst_core::error::board_change::BoardChangeError::OutsideFrame {
                reference: "Plan".into(),
                frame: holst_core::error::board_change::GivenRectangle {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                at: holst_core::error::board_change::GivenPoint { x: 20.0, y: 20.0 },
            },
        ));
        let own = [
            Error::EmptyQuery,
            Error::NoArgument { what: "board" },
            Error::CliOnly {
                command: "storage ls".into(),
            },
            Error::RawWriteDisabled {
                method: HttpMethod::Post,
            },
            Error::NotAFile,
            Error::StoreAborted,
        ];
        let flagged: Vec<String> = [plain_gz, outside]
            .into_iter()
            .chain(own)
            .map(|err| explain(&err))
            .filter(|message| message.contains("--"))
            .collect();
        assert_eq!(flagged, Vec::<String>::new());
    }
}
