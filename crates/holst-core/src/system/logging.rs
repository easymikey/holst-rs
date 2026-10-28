//! Logs to stderr filtered by `HOLST_LOG` (`level,target=level`).

use std::io::IsTerminal as _;

pub use tracing_subscriber::filter::{LevelFilter, Targets};

use crate::effect::settings::{ColorPreference, LogSettings, TerminalType};

/// Install the global subscriber; a bad `HOLST_LOG` falls back to `default`.
pub fn init(log: &LogSettings, default: Targets) {
    use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

    let raw = log.filter.clone();
    let parsed = raw.as_deref().map(parse);
    let filter = cap_wire_crates(parsed.clone().flatten().unwrap_or(default));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(
                    std::io::stderr().is_terminal()
                        && log.color == ColorPreference::Allowed
                        && log.terminal == TerminalType::Capable,
                )
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
                .without_time(),
        )
        .init();
    raw.zip(parsed)
        .filter(|(_, parsed)| parsed.is_none())
        .into_iter()
        .for_each(|(raw, _)| {
            tracing::warn!(
                HOLST_LOG = %raw,
                "not understood, using the default filter; the format is `level,target=level`"
            );
        });
}

const WIRE_CRATES: [&str; 5] =
    ["tungstenite", "tokio_tungstenite", "hyper", "h2", "reqwest"];

/// Cap the wire crates at `info` whatever `HOLST_LOG` asks for.
#[must_use]
pub fn cap_wire_crates(filter: Targets) -> Targets {
    WIRE_CRATES.iter().fold(filter, |filter, target| {
        let level = effective(&filter, target).min(LevelFilter::INFO);
        filter.with_target(*target, level)
    })
}

fn effective(filter: &Targets, target: &str) -> LevelFilter {
    use tracing::Level;
    [
        Level::TRACE,
        Level::DEBUG,
        Level::INFO,
        Level::WARN,
        Level::ERROR,
    ]
    .into_iter()
    .find(|level| filter.would_enable(target, level))
    .map_or(LevelFilter::OFF, LevelFilter::from_level)
}

/// Parse `HOLST_LOG`: blanks are ignored, empty turns logging off, errors and
/// span filters give `None`.
///
/// ```
/// use holst_core::system::logging::{LevelFilter, parse};
///
/// let filter = parse("warn,holst_core=trace");
/// assert_eq!(
///     filter.and_then(|f| f.default_level()),
///     Some(LevelFilter::WARN)
/// );
/// assert!(parse("holst_core[send]=trace").is_none());
/// ```
pub fn parse(raw: &str) -> Option<Targets> {
    let directives = raw
        .split(',')
        .map(str::trim)
        .filter(|directive| !directive.is_empty());
    match crate::domain::text::joined(directives, ",").to_string() {
        directives if directives.is_empty() => Some(Targets::new()),
        // `Targets` would take `target[span]` as a module and log nothing.
        directives if directives.contains(['[', ']', '{', '}']) => None,
        directives => directives.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use crate::system::logging::{LevelFilter, cap_wire_crates, parse};

    #[test]
    fn wire_crates_never_log_frames() {
        use tracing::Level;
        let traced = parse("trace,tungstenite=trace").map(cap_wire_crates);
        let quiet = parse("warn").map(cap_wire_crates);
        assert!(traced.as_ref().is_some_and(|f| {
            f.would_enable("holst_core::remote::socket", &Level::TRACE)
        }));
        assert!(
            traced.as_ref().is_some_and(
                |f| !f.would_enable("tungstenite::protocol", &Level::DEBUG)
            )
        );
        assert!(
            traced
                .as_ref()
                .is_some_and(|f| !f.would_enable("tokio_tungstenite", &Level::TRACE))
        );
        assert!(
            traced
                .as_ref()
                .is_some_and(|f| !f.would_enable("hyper::proto", &Level::TRACE))
        );
        assert!(
            traced
                .as_ref()
                .is_some_and(|f| f.would_enable("tungstenite", &Level::INFO))
        );
        assert!(
            quiet
                .as_ref()
                .is_some_and(|f| !f.would_enable("tungstenite", &Level::INFO))
        );
    }

    type Parsed = Option<(Option<LevelFilter>, Vec<(String, LevelFilter)>)>;

    #[rstest::rstest]
    #[case::level("debug", Some((Some(LevelFilter::DEBUG), vec![])))]
    #[case::level_and_target(
        " warn , holst_core=trace ,",
        Some((Some(LevelFilter::WARN), vec![("holst_core".into(), LevelFilter::TRACE)]))
    )]
    #[case::empty_disables(" , ", Some((None, vec![])))]
    #[case::bad_level("holst_core=bogus", None)]
    #[case::span_filter("holst_core[send]=trace", None)]
    fn parses_holst_log(#[case] raw: &str, #[case] expected: Parsed) {
        let parsed = parse(raw).map(|filter| {
            let targets = filter.iter().map(|(t, l)| (t.to_string(), l)).collect();
            (filter.default_level(), targets)
        });
        assert_eq!(parsed, expected);
    }
}
