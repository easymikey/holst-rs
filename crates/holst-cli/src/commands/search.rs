//! `holst search`: the core's search, and what it prints.

use holst_core::{
    command::{
        board::show::account_cache,
        board_load::CacheUse,
        search::{self, Found, NameMatch, SearchHit, TextMatch},
        table,
    },
    domain::{node::NodeKind, text::printable},
    storage::cache::Cache,
};
use serde_json::{Map, Value};

use crate::{
    error::Result,
    output::{self, DiagnosticLabel, OutputFormat, coloring::Coloring},
    run::{RunSettings, run_signed_in},
};

pub(crate) async fn run(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let args = table::parse(arguments)?;
    run_signed_in(run, async |client| {
        let cache = account_cache(client, run.cache(), CacheUse::Use)
            .await?
            .map(Cache::into_shared);
        let found = search::search(client, &args, cache).await?;
        match format {
            OutputFormat::Text => {
                output::line(text(&found, output::stdout_coloring()))?;
            }
            OutputFormat::Pretty | OutputFormat::Json => {
                output::print_value(format, &found)?;
            }
        }
        notes(&found);
        Ok(())
    })
    .await
}

/// Returns one line per match, best first; `no matches` when there is none.
fn text(found: &Found, coloring: Coloring) -> String {
    let lines: Vec<String> = found
        .hits
        .iter()
        .map(|hit| match hit {
            SearchHit::Name(name) => name_line(name),
            SearchHit::Text(text) => text_line(text, coloring),
        })
        .collect();
    if lines.is_empty() {
        String::from("no matches")
    } else {
        lines.join("\n")
    }
}

fn name_line(found: &NameMatch) -> String {
    let place = [found.workspace.as_str(), found.path.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let slash = if found.kind == NodeKind::Folder.as_str() {
        "/"
    } else {
        ""
    };
    let trash = if found.in_trash { " (trash)" } else { "" };
    let link = found
        .link
        .as_deref()
        .map(|link| format!("  {link}"))
        .unwrap_or_default();
    printable(&format!(
        "{:<7}{}{slash} · {place}{trash}{link}",
        found.kind, found.name
    ))
    .into_owned()
}

fn text_line(found: &TextMatch, coloring: Coloring) -> String {
    let [from, to] = found.matched;
    let chars: Vec<char> = found.excerpt.chars().collect();
    let part = |start: usize, end: usize| -> String {
        chars.get(start..end).unwrap_or_default().iter().collect()
    };
    let short = found.object.get(..8).unwrap_or(&found.object);
    format!(
        "{} / {} @{} : {}{}{}  {}",
        printable(&found.board_name),
        printable(&found.kind),
        printable(short),
        printable(&part(0, from)),
        coloring.yellow(&printable(&part(from, to))),
        printable(&part(to, chars.len())),
        found.link,
    )
}

/// The warning for a workspace whose tree could not be read.
fn skipped_workspace(workspace: &str) -> String {
    printable(&format!("not searched: {workspace}")).into_owned()
}

/// Says on stderr what the result left out.
fn notes(found: &Found) {
    found
        .skipped_workspaces
        .iter()
        .for_each(|workspace| output::warn(skipped_workspace(workspace)));
    found.skipped_boards.iter().for_each(|board| {
        output::warn(printable(&format!(
            "board {} not searched: {}",
            board.name, board.reason
        )));
    });
    if found.hits.len() < found.total {
        output::labelled(
            DiagnosticLabel::Note,
            format_args!(
                "{} of {} matches shown; -l shows more",
                found.hits.len(),
                found.total
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use holst_core::command::search::{Found, NameMatch, SearchHit, TextMatch};

    use crate::{
        commands::search::{name_line, skipped_workspace, text, text_line},
        output::coloring::Coloring,
    };

    fn text_match(excerpt: &str, matched: [usize; 2]) -> TextMatch {
        TextMatch {
            board: "b1".to_owned(),
            board_name: "Roadmap\u{9b}".to_owned(),
            object: "0123456789ab".to_owned(),
            kind: "sticker".to_owned(),
            excerpt: excerpt.to_owned(),
            matched,
            fuzzy: false,
            link: "https://x".to_owned(),
            score: 650,
        }
    }

    fn found(hits: Vec<SearchHit>) -> Found {
        Found {
            query: "plan".to_owned(),
            total: hits.len(),
            hits,
            boards_read: 0,
            skipped_workspaces: Vec::new(),
            skipped_boards: Vec::new(),
        }
    }

    #[test]
    fn a_text_line_marks_the_match_and_disarms_escapes() {
        let hit = text_match("План \u{1b}[2J релиза", [10, 16]);
        assert_eq!(
            text_line(&hit, Coloring::Plain),
            "Roadmap\u{fffd} / sticker @01234567 : План \u{fffd}[2J релиза  https://x"
        );
        let coloured = text_line(&hit, Coloring::Colored);
        assert!(
            coloured.contains("\u{1b}[33mрелиза\u{1b}[0m"),
            "{coloured:?}"
        );
        assert!(!coloured.contains("\u{1b}[1m"), "bold: {coloured:?}");
    }

    #[test]
    fn a_name_line_names_the_place_and_the_link() {
        let board = NameMatch {
            id: "b1".to_owned(),
            name: "Road map".to_owned(),
            kind: "board",
            workspace: "Team".to_owned(),
            path: "Q3".to_owned(),
            in_trash: true,
            link: Some("https://x/b1".to_owned()),
            score: 900,
        };
        let folder = NameMatch {
            kind: "folder",
            path: String::new(),
            in_trash: false,
            link: None,
            ..board.clone()
        };
        assert_eq!(
            [name_line(&board), name_line(&folder)],
            [
                "board  Road map · Team/Q3 (trash)  https://x/b1",
                "folder Road map/ · Team"
            ]
        );
    }

    #[test]
    fn a_skipped_workspace_name_disarms_escapes() {
        let note = skipped_workspace("\u{1b}]52;c;x\u{7}");
        assert!(
            !note.contains('\u{1b}') && !note.contains('\u{7}'),
            "{note:?}"
        );
    }

    #[test]
    fn no_hits_say_so() {
        assert_eq!(text(&found(Vec::new()), Coloring::Plain), "no matches");
    }

    #[test]
    fn hits_print_in_the_order_given() {
        let hits = vec![
            SearchHit::Text(text_match("plan", [0, 4])),
            SearchHit::Text(text_match("a plan", [2, 6])),
        ];
        assert_eq!(text(&found(hits), Coloring::Plain).lines().count(), 2);
    }
}
