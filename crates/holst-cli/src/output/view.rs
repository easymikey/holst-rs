//! What the CLI shows: the error report and object listings; pure.

use holst_core::domain::{account::Account, text::printable};

use crate::{error::Error, output::coloring::Coloring};

/// The error report for stderr:
///
/// ```text
/// error: <what happened>
///   cause: <a lower-level reason>
/// hint: <what to do>
/// ```
#[must_use]
pub(crate) fn report(error: &Error, coloring: Coloring) -> String {
    let first = format!(
        "{} {}",
        crate::output::DiagnosticLabel::Error.painted(coloring),
        printable(&error.to_string())
    );
    report_with(first, error, coloring, None)
}

#[must_use]
pub(crate) fn report_with(
    first: String,
    error: &Error,
    coloring: Coloring,
    fallback_hint: Option<String>,
) -> String {
    std::iter::once(first)
        .chain(cause_lines(error, coloring, "  ", fallback_hint))
        .collect::<Vec<_>>()
        .join("\n")
}

#[must_use]
pub(crate) fn form_cause_lines(error: &Error, coloring: Coloring) -> Vec<String> {
    cause_lines(error, coloring, "", None).collect()
}

fn cause_lines<'a>(
    error: &'a Error,
    coloring: Coloring,
    indent: &'a str,
    fallback_hint: Option<String>,
) -> impl Iterator<Item = String> + 'a {
    let hint = error.hint().map(|hint| hint.to_string()).or(fallback_hint);
    holst_core::error::chain_unless(error, is_wire_detail)
        .map(move |cause| {
            coloring.dim(&format!("{indent}cause: {}", printable(&cause)))
        })
        .chain(hint.map(move |hint| {
            format!("{} {}", coloring.green("hint:"), coloring.code(&hint))
        }))
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

pub(crate) fn size_text(bytes: u64) -> String {
    let units = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
    units
        .into_iter()
        .find(|&(unit, _)| bytes >= unit)
        .map_or_else(
            || format!("{bytes} B"),
            |(unit, name)| {
                let tenths = bytes.saturating_mul(10) / unit;
                format!("{}.{} {name}", tenths / 10, tenths % 10)
            },
        )
}

pub(crate) fn display_name(account: &Account) -> &str {
    account.name.as_deref().unwrap_or("(no name)")
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::{
        error::Error,
        output::{
            coloring::Coloring,
            view::{form_cause_lines, report},
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

    const PLAIN: Coloring = Coloring::Plain;
    const COLOUR: Coloring = Coloring::Colored;

    fn unauthorized() -> Error {
        Error::Holst(holst_core::Error::Http(
            holst_core::error::http::HttpError::Status {
                method: "GET",
                path: "/gapi/current_user".into(),
                status: 401,
                detail: String::new(),
            },
        ))
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
             \u{1b}[32mhint:\u{1b}[0m run \u{1b}[36mholst auth login\u{1b}[0m to log in again"
        );
        assert!(
            !report(&unauthorized(), COLOUR).contains("\u{1b}[1m"),
            "nothing bold"
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
