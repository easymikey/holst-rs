//! Printing: results to stdout, notes to stderr under one set of labels.

pub(crate) mod board;
pub(crate) mod coloring;
pub(crate) mod view;

use std::{
    fmt::Display,
    io,
    io::{BufWriter, IsTerminal as _, StdoutLock, Write},
};

use holst_core::{
    domain::text::{EMPTY, is_blank, printable},
    error::hint::LOGIN,
};
use serde::Serialize;
use serde_json::Value;

use crate::{
    error::{Error, Result},
    output::coloring::Coloring,
    terminal::detect::StreamKind,
};

/// How a command prints its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum OutputFormat {
    /// Indented JSON.
    Pretty,
    /// JSON on one line — handy for `jq` and scripts.
    Json,
    /// A readable table or tree where it makes sense.
    Text,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the output module is the one door to stdout"
)]
fn to_stdout(
    write: impl FnOnce(&mut BufWriter<StdoutLock<'_>>) -> io::Result<()>,
) -> Result<()> {
    // Without the buffer the line-buffered stdout makes a syscall per line.
    let out = &mut BufWriter::new(io::stdout().lock());
    write(out).and_then(|()| out.flush()).map_err(stdout_error)
}

fn stdout_error(error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::BrokenPipe {
        Error::StdoutClosed
    } else {
        Error::WriteStdout(error)
    }
}

pub(crate) fn line(text: impl Display) -> Result<()> {
    to_stdout(|out| writeln!(out, "{text}"))
}

fn write_bytes(bytes: &[u8]) -> Result<()> {
    to_stdout(|out| out.write_all(bytes))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the output module is the one door to stderr"
)]
pub(crate) fn stderr_line(text: impl Display) {
    // A stderr that cannot be written has nowhere to report to.
    writeln!(io::stderr().lock(), "{text}").unwrap_or_default();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub(crate) enum DiagnosticLabel {
    #[display("error:")]
    Error,
    #[display("warning:")]
    Warning,
    #[display("note:")]
    Note,
}

impl DiagnosticLabel {
    #[must_use]
    fn painted(self, coloring: Coloring) -> String {
        let text = self.to_string();
        match self {
            DiagnosticLabel::Error => coloring.red(&text),
            DiagnosticLabel::Warning => coloring.yellow(&text),
            DiagnosticLabel::Note => text,
        }
    }
}

pub(crate) fn labelled(label: DiagnosticLabel, text: impl Display) {
    stderr_line(format_args!("{} {text}", label.painted(stderr_coloring())));
}

pub(crate) fn warn(text: impl Display) {
    labelled(DiagnosticLabel::Warning, text);
}

pub(crate) fn warn_with(headline: impl Display, error: &Error) {
    stderr_line(warning_text(
        &headline.to_string(),
        error,
        stderr_coloring(),
    ));
}

fn warning_text(headline: &str, error: &Error, coloring: Coloring) -> String {
    let first = format!(
        "{} {headline}: {}",
        DiagnosticLabel::Warning.painted(coloring),
        printable(&error.to_string())
    );
    let advice = format!("run `{LOGIN}` if the next command is refused");
    view::report_with(first, error, coloring, Some(advice))
}

pub(crate) fn stderr_coloring() -> Coloring {
    Coloring::colored_if(console::colors_enabled_stderr())
}

pub(crate) fn stdout_coloring() -> Coloring {
    Coloring::colored_if(console::colors_enabled())
}

pub(crate) fn value(format: OutputFormat, value: &Value) -> Result<()> {
    reply(format, ReplyKind::Read, value)
}

/// Prints a typed reply as the formats show a reply.
/// Prints `text` as text, or the reply as JSON.
pub(crate) fn print(
    format: OutputFormat,
    text: &str,
    reply: &impl Serialize,
) -> Result<()> {
    match format {
        OutputFormat::Text => line(printable(text)),
        OutputFormat::Pretty | OutputFormat::Json => print_value(format, reply),
    }
}

pub(crate) fn print_value(format: OutputFormat, reply: &impl Serialize) -> Result<()> {
    value(format, &serde_json::to_value(reply).map_err(Error::Render)?)
}

pub(crate) fn report(
    format: OutputFormat,
    result: &(impl Serialize + Display),
) -> Result<()> {
    match format {
        // Names and texts in a result come from the server, which others write.
        OutputFormat::Text => line(printable(&result.to_string())),
        OutputFormat::Pretty | OutputFormat::Json => value(
            format,
            &serde_json::to_value(result).map_err(Error::Render)?,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputTarget {
    Redirected,
    Terminal,
}

impl OutputTarget {
    pub(crate) const fn of(stdout: StreamKind) -> Self {
        match stdout {
            StreamKind::Terminal => OutputTarget::Terminal,
            StreamKind::Redirected => OutputTarget::Redirected,
        }
    }

    pub(crate) fn put(self, contents: &[u8]) -> Result<()> {
        match self {
            OutputTarget::Terminal => Err(Error::BinaryToTerminal),
            OutputTarget::Redirected => write_bytes(contents),
        }
    }

    pub(crate) const fn checked(self) -> Result<Self> {
        match self {
            OutputTarget::Terminal => Err(Error::BinaryToTerminal),
            OutputTarget::Redirected => Ok(self),
        }
    }
}

/// Tells whether stdout is a terminal or redirected.
#[expect(
    clippy::disallowed_methods,
    reason = "the output module is the one door to stdout"
)]
pub(crate) fn stdout_kind() -> StreamKind {
    StreamKind::terminal_if(io::stdout().is_terminal())
}

pub(crate) fn reply(
    format: OutputFormat,
    kind: ReplyKind,
    value: &Value,
) -> Result<()> {
    line(rendered(format, kind, value)?)
}

const DONE: &str = "done";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyKind {
    Read,
    Change,
}

fn rendered(format: OutputFormat, kind: ReplyKind, value: &Value) -> Result<String> {
    match (format, kind, is_blank(value)) {
        (OutputFormat::Text, ReplyKind::Read, true) => Ok(EMPTY.to_owned()),
        (OutputFormat::Text, ReplyKind::Change, true) => Ok(DONE.to_owned()),
        // JSON escapes C0 controls only; C1 and bidi overrides from names stay.
        (OutputFormat::Text, _, _) => serde_json::to_string_pretty(value)
            .map(|text| printable(&text).into_owned()),
        (OutputFormat::Pretty, _, _) => serde_json::to_string_pretty(value),
        (OutputFormat::Json, _, _) => serde_json::to_string(value),
    }
    .map_err(Error::Render)
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::{Value, json};

    use crate::{
        error::Error,
        output::{
            OutputFormat,
            OutputTarget,
            ReplyKind,
            coloring::Coloring,
            rendered,
            stdout_error,
            warning_text,
        },
        terminal::detect::StreamKind,
    };

    #[rstest::rstest]
    #[case::text_list(OutputFormat::Text, json!([]), "(empty)")]
    #[case::text_wrapped(OutputFormat::Text, json!({ "channels": [] }), "(empty)")]
    #[case::text_zero(OutputFormat::Text, json!({ "count": 0 }), "{\n  \"count\": 0\n}")]
    #[case::json(OutputFormat::Json, json!([]), "[]")]
    #[case::pretty(OutputFormat::Pretty, json!({ "channels": [] }), "{\n  \"channels\": []\n}")]
    fn empty_replies_are_named_in_text(
        #[case] format: OutputFormat,
        #[case] value: Value,
        #[case] expected: &str,
    ) {
        assert_eq!(
            rendered(format, ReplyKind::Read, &value).ok().as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn text_replies_are_printable_json_ones_exact() {
        let value = json!({ "name": "a\u{202e}b\u{9b}c" });
        let text = rendered(OutputFormat::Text, ReplyKind::Read, &value).ok();
        assert_eq!(
            text.as_deref(),
            Some("{\n  \"name\": \"a\u{fffd}b\u{fffd}c\"\n}")
        );
        let json = rendered(OutputFormat::Json, ReplyKind::Read, &value).ok();
        assert_eq!(json.as_deref(), Some("{\"name\":\"a\u{202e}b\u{9b}c\"}"));
    }

    #[rstest::rstest]
    #[case::null(OutputFormat::Text, Value::Null, "done")]
    #[case::object(OutputFormat::Text, json!({}), "done")]
    #[case::list(OutputFormat::Text, json!([]), "done")]
    #[case::reply(OutputFormat::Text, json!({ "id": "b1" }), "{\n  \"id\": \"b1\"\n}")]
    #[case::json(OutputFormat::Json, Value::Null, "null")]
    #[case::pretty(OutputFormat::Pretty, json!({}), "{}")]
    fn empty_replies_to_changes_say_done(
        #[case] format: OutputFormat,
        #[case] value: Value,
        #[case] expected: &str,
    ) {
        assert_eq!(
            rendered(format, ReplyKind::Change, &value).ok().as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn warning_says_what_to_do() {
        let coloring = Coloring::Plain;
        let failed = Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::FileWrite {
                path: "/h/token".into(),
                error: io::Error::from(io::ErrorKind::PermissionDenied),
            },
        ));
        assert_eq!(
            warning_text("the login was not saved", &failed, coloring),
            "warning: the login was not saved: could not write /h/token\n  cause: permission denied\nhint: run `holst \
             auth login` if the next command is refused"
        );
        let locked = Error::Holst(holst_core::Error::LocalData(
            holst_core::error::local_data::LocalDataError::KeychainLocked,
        ));
        let text = warning_text("the login was not saved", &locked, coloring);
        assert!(text.ends_with("hint: unlock the keychain and allow access, or keep the login in `HOLST_HOME`"), "{text}");
    }

    #[rstest::rstest]
    #[case::terminal(StreamKind::Terminal, OutputTarget::Terminal)]
    #[case::pipe(StreamKind::Redirected, OutputTarget::Redirected)]
    fn sink_follows_stdout(#[case] stdout: StreamKind, #[case] expected: OutputTarget) {
        assert_eq!(OutputTarget::of(stdout), expected);
    }

    #[test]
    fn sink_refuses_a_terminal() {
        assert!(matches!(
            OutputTarget::Terminal.put(b"\x89PNG"),
            Err(Error::BinaryToTerminal)
        ));
    }

    #[test]
    fn only_a_broken_pipe_means_a_closed_stdout() {
        assert!(matches!(
            stdout_error(io::Error::from(io::ErrorKind::BrokenPipe)),
            Error::StdoutClosed
        ));
        let full = stdout_error(io::Error::from(io::ErrorKind::StorageFull));
        assert!(matches!(full, Error::WriteStdout(_)));
        assert_eq!(full.to_string(), "could not write to stdout");
    }
}
