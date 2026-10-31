//! What the CLI shows: the error report and object listings; pure.

use holst_core::{
    Error as CoreError,
    domain::{account::Account, text::printable},
    error::input::InputError,
};

use crate::{error::Error, output::color_choice::ColorChoice};

/// The error report for stderr:
///
/// ```text
/// error: <what happened>
///   cause: <a lower-level reason>
/// hint: <what to do>
/// ```
#[must_use]
pub(crate) fn report(error: &Error, color: ColorChoice) -> String {
    let several = problems(error);
    let head = if several.is_empty() {
        error.to_string()
    } else {
        format!("{} problems:", several.len())
    };
    let first = format!(
        "{} {}",
        crate::output::DiagnosticLabel::Error.painted(color),
        spans(&printable(&head), color)
    );
    report_with(first, error, color, None)
}

#[must_use]
pub(crate) fn report_with(
    first: String,
    error: &Error,
    color: ColorChoice,
    fallback_hint: Option<String>,
) -> String {
    std::iter::once(first)
        .chain(pointer_lines(error))
        .chain(problem_lines(error))
        .chain(cause_lines(error, color, "  ", fallback_hint))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The problems of a list of several, as the core carries them.
fn problems(error: &Error) -> &[CoreError] {
    let Error::Holst(CoreError::Input(InputError::Several { errors })) = error else {
        return &[];
    };
    errors
}

/// One line for each problem of a list of several.
fn problem_lines(error: &Error) -> Vec<String> {
    problems(error)
        .iter()
        .map(|problem| format!("  - {}", printable(&problem.to_string())))
        .collect()
}

/// The offending line of a JSON argument and a `^` under the column.
fn pointer_lines(error: &Error) -> Vec<String> {
    let (Error::ItemsNotJson { spot, .. }
    | Error::Holst(CoreError::Input(InputError::NotJsonObject { spot, .. }))) = error
    else {
        return Vec::new();
    };
    let before = spot.text.chars().take(spot.caret).collect::<String>();
    vec![
        format!("  {}", printable(&spot.text)),
        format!(
            "  {}^",
            " ".repeat(console::measure_text_width(&printable(&before)))
        ),
    ]
}

#[must_use]
pub(crate) fn form_cause_lines(error: &Error, color: ColorChoice) -> Vec<String> {
    cause_lines(error, color, "", None).collect()
}

fn cause_lines<'a>(
    error: &'a Error,
    color: ColorChoice,
    indent: &'a str,
    fallback_hint: Option<String>,
) -> impl Iterator<Item = String> + 'a {
    let hint = error.hint().map(|hint| hint.to_string()).or(fallback_hint);
    holst_core::error::chain_unless(error, is_wire_detail)
        .map(move |cause| color.dim(&format!("{indent}cause: {}", printable(&cause))))
        .chain(hint.map(move |hint| {
            format!("{} {}", color.green("hint:"), spans(&hint, color))
        }))
}

/// `text` with the spans between backticks (flags, commands) in bold cyan,
/// marks dropped, and those between double quotes (values) in yellow, marks
/// kept; a mark that is never closed stays as it is.
pub(crate) fn spans(text: &str, color: ColorChoice) -> String {
    match color {
        ColorChoice::Plain => text.to_owned(),
        ColorChoice::Colored => painted(text, color),
    }
}

fn painted(text: &str, color: ColorChoice) -> String {
    let Some(at) = text.find(['`', '"']) else {
        return text.to_owned();
    };
    let (before, from) = text.split_at(at);
    let (mark, after) = from.split_at(1);
    match (mark, after.split_once(mark)) {
        ("`", Some((inner, tail))) => {
            format!("{before}{}{}", color.cyan_bold(inner), painted(tail, color))
        }
        (_, Some((inner, tail))) => format!(
            "{before}{}{}",
            color.yellow(&format!("\"{inner}\"")),
            painted(tail, color)
        ),
        (_, None) => format!("{before}{mark}{}", painted(after, color)),
    }
}

/// A status number is not for the report, only for the verbose log.
fn is_wire_detail(cause: &(dyn std::error::Error + 'static)) -> bool {
    cause
        .downcast_ref::<holst_core::Error>()
        .is_some_and(|error| error.status().is_some())
        || matches!(
            cause.downcast_ref::<holst_core::error::http::HttpError>(),
            Some(holst_core::error::http::HttpError::Status { .. })
        )
}

pub(crate) fn display_name(account: &Account) -> &str {
    account.name.as_deref().unwrap_or("(no name)")
}

#[cfg(test)]
mod tests {
    use std::io;

    use holst_core::{
        Error as CoreError,
        error::input::{InputError, JsonSpot},
    };

    use crate::{
        error::Error,
        output::{
            color_choice::ColorChoice,
            view::{form_cause_lines, report, spans},
        },
    };

    #[test]
    fn board_text_cannot_drive_the_terminal() {
        let hostile = "\u{1b}]52;c;cm0gLXJm\u{7}\u{1b}[2J\u{202e}";
        let err = Error::Holst(holst_core::Error::Lookup(
            holst_core::error::lookup::LookupError {
                kind: holst_core::error::lookup::LookupKind::Object,
                reference: "o".into(),
                miss: holst_core::error::lookup::LookupMiss::ambiguous(
                    ["o1".to_owned() + hostile, "o2".to_owned()]
                        .map(|name| holst_core::error::lookup::Candidate {
                            name,
                            ..holst_core::error::lookup::Candidate::default()
                        })
                        .into_iter()
                        .collect(),
                ),
            },
        ));
        let reported = report(&err, PLAIN);
        assert!(
            !reported.chars().any(holst_core::domain::text::is_unsafe),
            "{reported:?}"
        );
    }

    const PLAIN: ColorChoice = ColorChoice::Plain;
    const COLOUR: ColorChoice = ColorChoice::Colored;

    fn unauthorized() -> Error {
        Error::Holst(holst_core::Error::Http(holst_core::fake::http_status(
            401,
            holst_core::error::http::ReplyBody::Empty,
        )))
    }

    #[test]
    fn report_has_error_causes_and_hint() {
        let err = Error::BodyFile {
            path: "/no/such/file.json".into(),
            error: io::Error::from(io::ErrorKind::NotFound),
        };
        assert_eq!(
            report(&err, PLAIN),
            "error: could not read /no/such/file.json\n  cause: entity not found"
        );
        assert_eq!(
            report(&unauthorized(), PLAIN),
            "error: GET /gapi/current_user -> HTTP 401\n\
             hint: run `holst auth login` to log in again"
        );
    }

    #[test]
    fn a_failed_clean_shows_the_cause_on_its_own_line() {
        let err = Error::NotCleaned {
            place: holst_core::command::storage::CleanPlace::BoardCache,
            error: holst_core::error::local_data::LocalDataError::FileRemove {
                path: "/h/cache".into(),
                error: io::Error::from(io::ErrorKind::PermissionDenied),
            }
            .into(),
        };
        assert_eq!(
            report(&err, PLAIN),
            "error: could not remove a board cache\n  cause: could not remove /h/cache\n  cause: permission denied"
        );
    }

    #[test]
    fn colour_report_paints_labels_and_code() {
        let err = Error::BodyFile {
            path: "/x".into(),
            error: io::Error::from(io::ErrorKind::NotFound),
        };
        assert_eq!(
            report(&err, COLOUR),
            "\u{1b}[31merror:\u{1b}[0m could not read /x\n\u{1b}[2m  cause: entity not found\u{1b}[0m"
        );
        assert_eq!(
            report(&unauthorized(), COLOUR),
            "\u{1b}[31merror:\u{1b}[0m GET /gapi/current_user -> HTTP 401\n\
             \u{1b}[32mhint:\u{1b}[0m run \u{1b}[36m\u{1b}[1mholst auth login\u{1b}[0m to log in again"
        );
    }

    #[rstest::rstest]
    #[case::no_marks("plain words", "plain words")]
    #[case::one_flag("use `--all` here", "use \u{1b}[36m\u{1b}[1m--all\u{1b}[0m here")]
    #[case::one_value("not \"a b\" here", "not \u{1b}[33m\"a b\"\u{1b}[0m here")]
    #[case::both(
        "`x` is not \"y\"",
        "\u{1b}[36m\u{1b}[1mx\u{1b}[0m is not \u{1b}[33m\"y\"\u{1b}[0m"
    )]
    #[case::unclosed_flag("a `b \"c\"", "a `b \u{1b}[33m\"c\"\u{1b}[0m")]
    #[case::unclosed_value("a \"b `c`", "a \"b \u{1b}[36m\u{1b}[1mc\u{1b}[0m")]
    fn spans_are_painted_when_colour_is_on(#[case] text: &str, #[case] painted: &str) {
        assert_eq!(spans(text, COLOUR), painted);
    }

    #[test]
    fn spans_leave_plain_text_as_it_is() {
        assert_eq!(spans("a `b` \"c\" `d", PLAIN), "a `b` \"c\" `d");
    }

    fn broken(text: &str) -> Option<(JsonSpot, serde_json::Error)> {
        serde_json::from_str::<serde_json::Value>(text)
            .err()
            .map(|error| (JsonSpot::of(&error, text), error))
    }

    #[test]
    fn a_json_argument_error_points_at_its_column() {
        let reported = broken("[\n  {\"a\": 1},\n  {\"a\": }\n]")
            .map(|(spot, error)| report(&Error::ItemsNotJson { spot, error }, PLAIN));
        assert_eq!(
            reported.as_deref(),
            Some(
                "error: the items are not a JSON array or JSON lines\n    {\"a\": }\n          ^\n  cause: expected value at line 3 column 9"
            )
        );
    }

    #[test]
    fn a_tab_counts_as_one_column_in_the_line_and_under_it() {
        let reported = broken("[\n\t{\"a\": }\n]")
            .map(|(spot, error)| report(&Error::ItemsNotJson { spot, error }, PLAIN));
        assert_eq!(
            reported.as_deref(),
            Some(
                "error: the items are not a JSON array or JSON lines\n   {\"a\": }\n         ^\n  cause: expected value at line 2 column 8"
            )
        );
    }

    #[test]
    fn a_long_line_is_cut_around_the_column() {
        let text = format!("[{}{{\"a\": }}{}]", "1,".repeat(500), ",1".repeat(500));
        let reported = broken(&text)
            .map(|(spot, error)| report(&Error::ItemsNotJson { spot, error }, PLAIN));
        let lines: Vec<&str> =
            reported.as_deref().unwrap_or_default().lines().collect();
        assert_eq!(lines.len(), 4, "{lines:?}");
        let shown = lines.get(1).copied().unwrap_or_default();
        let pointer = lines.get(2).copied().unwrap_or_default();
        assert_eq!(shown.chars().count(), 2 + 1 + 40 + 40 + 1, "{lines:?}");
        assert!(shown.contains("1,1,{\"a\": }"), "{lines:?}");
        assert_eq!(pointer.find('^'), Some(2 + 1 + 40), "{lines:?}");
    }

    #[test]
    fn a_body_error_from_core_points_at_its_column() {
        let reported = broken("{\"a\" 1}").map(|(spot, _)| {
            report(
                &Error::Holst(CoreError::Input(InputError::NotJsonObject {
                    command: "template edit".into(),
                    name: "body",
                    spot,
                })),
                PLAIN,
            )
        });
        assert_eq!(
            reported.as_deref(),
            Some(
                "error: body of `template edit` must be a JSON object\n  {\"a\" 1}\n       ^"
            )
        );
    }

    #[test]
    fn several_problems_are_listed_one_to_a_line() {
        let err = Error::Holst(CoreError::Input(InputError::Several {
            errors: vec![
                CoreError::Input(InputError::FullUrl),
                CoreError::Input(InputError::RelativePath),
            ],
        }));
        assert_eq!(
            report(&err, PLAIN),
            "error: 2 problems:\n  - expected a path, not a full URL: the session is sent only to its own server\n  - the path must start with \"/\", for example /gapi/current_user"
        );
    }

    #[test]
    fn form_details_are_causes_then_the_hint() {
        let err = Error::EchoNotHidden(io::Error::from(io::ErrorKind::InvalidData));
        let details = form_cause_lines(&err, PLAIN);
        assert!(
            details.first().is_some_and(|d| d.starts_with("cause: ")),
            "{details:?}"
        );
        assert!(
            details
                .last()
                .is_some_and(|d| d.starts_with("hint: pipe the code")),
            "{details:?}"
        );
    }
}
