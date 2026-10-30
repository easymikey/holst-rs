//! `holst board show snap save export import`: a whole board, or a part of it.

use std::path::Path;

use holst_core::{
    command::{
        board::{
            PictureFormat,
            file::{self, Exported, Imported, STDOUT},
            picture::{self, Delivery, Saved, Snapped},
            show::{self, BoardBrief, Briefs, Missing},
        },
        change::{ChangeReport, PlannedRequest},
        table,
    },
    domain::text::{counted, printable},
    error::lookup::{LookupError, LookupKind, LookupMiss},
};
use serde_json::{Map, Value};

use crate::{
    commands,
    error::{Error, Result},
    output::{self, DiagnosticLabel, OutputFormat, OutputTarget, view::size_text},
    run::{RunSettings, run_signed_in},
};

/// Shows the boards asked for. A board that is not found is told after the
/// found ones, and the exit code is not-found.
pub(super) async fn show(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let args = table::parse(arguments)?;
    run_signed_in(run, async |client| {
        let shown = show::show(client, run.cache(), &args).await?;
        output::print(format, &shown_text(&shown), &shown)?;
        shown
            .missing
            .iter()
            .for_each(|gone| output::warn(printable(&gone.reason)));
        shown
            .missing
            .first()
            .map_or(Ok(()), |gone| Err(not_found(gone)))
    })
    .await
}

fn not_found(gone: &Missing) -> Error {
    Error::Holst(holst_core::Error::Lookup(LookupError {
        kind: LookupKind::Board,
        reference: gone.reference.clone(),
        miss: LookupMiss::not_found(),
    }))
}

/// Draws a board; a file of `-` sends the picture to stdout.
pub(super) async fn snap(
    mut arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let stdout = is_stdout(&arguments) && arguments.remove("file").is_some();
    let args: holst_core::command::board::SnapArgs = table::parse(arguments)?;
    let target = OutputTarget::of(output::stdout_kind());
    if stdout {
        // The bytes never go to a terminal: refused before any request.
        refuse_terminal(target, args.picture_format == Some(PictureFormat::Svg))?;
    }
    let downloads = run.downloads_dir().filter(|_| !stdout);
    run_signed_in(run, async |client| {
        let snapped =
            picture::snap(client, run.cache(), downloads.as_deref(), &args).await?;
        if let Some(warning) = &snapped.warning {
            output::labelled(DiagnosticLabel::Note, printable(warning));
        }
        match &snapped.delivery {
            Delivery::Inline { bytes } => {
                output::stderr_line(picture_line(&snapped));
                target.put(bytes)
            }
            Delivery::Written { .. } => {
                output::print(format, &snapped_text(&snapped), &snapped)
            }
        }
    })
    .await
}

/// Saves a board as a new folder.
pub(super) async fn save(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let args = table::parse(arguments)?;
    let downloads = run.downloads_dir();
    run_signed_in(run, async |client| {
        let saved =
            picture::save(client, run.cache(), downloads.as_deref(), &args).await?;
        output::print(format, &saved_text(&saved), &saved)
    })
    .await
}

/// Saves the whole board as a file; a file of `-` sends it to stdout.
pub(super) async fn export(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    let target = OutputTarget::of(output::stdout_kind());
    if is_stdout(&arguments) {
        // The bytes never go to a terminal: refused before any request.
        target.checked()?;
    }
    let args = table::parse(arguments)?;
    let downloads = run.downloads_dir();
    run_signed_in(run, async |client| {
        let exported =
            file::export(client, run.cache(), downloads.as_deref(), &args).await?;
        match &exported.inline {
            Some(document) => {
                target.put(document)?;
                output::stderr_line(exported_text(&exported));
                Ok(())
            }
            None => output::print(format, &exported_text(&exported), &exported),
        }
    })
    .await
}

/// Makes a board of a board file, or writes the file into a board.
pub(super) async fn import(
    arguments: Map<String, Value>,
    run: &RunSettings,
    format: OutputFormat,
) -> Result<()> {
    commands::change(
        arguments,
        run,
        format,
        async |client, args| file::import(client, run.cache(), args).await,
        |format, imported| output::print(format, &imported_text(imported), imported),
    )
    .await
}

fn is_stdout(arguments: &Map<String, Value>) -> bool {
    arguments.get("file").and_then(Value::as_str) == Some(STDOUT)
}

fn refuse_terminal(target: OutputTarget, svg: bool) -> Result<()> {
    match (target, svg) {
        (OutputTarget::Terminal, false) => Err(Error::BinaryToTerminal),
        (OutputTarget::Terminal, true) => Err(Error::SvgToTerminal),
        (OutputTarget::Redirected, _) => Ok(()),
    }
}

fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

fn shown_text(shown: &Briefs) -> String {
    shown
        .boards
        .iter()
        .map(board_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn board_text(board: &BoardBrief) -> String {
    let place = [board.workspace.as_deref(), board.folder.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" / ");
    let trash = if board.in_trash { " · in trash" } else { "" };
    let kinds = board
        .objects
        .iter()
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    let objects = if kinds.is_empty() {
        counted(board.count, "object")
    } else {
        format!("{}: {kinds}", counted(board.count, "object"))
    };
    let area = board
        .rectangle
        .map(|area| format!("\n  rectangle {area}"))
        .unwrap_or_default();
    let parts = [
        (
            "members",
            board
                .members
                .as_ref()
                .and_then(|users| serde_json::to_string(users).ok()),
        ),
        (
            "rights",
            board
                .rights
                .as_ref()
                .and_then(|users| serde_json::to_string(users).ok()),
        ),
        (
            "sharing",
            board
                .sharing
                .as_ref()
                .and_then(|sharing| serde_json::to_string(sharing).ok()),
        ),
    ]
    .into_iter()
    .filter_map(|(name, text)| text.map(|text| format!("\n  {name}: {text}")))
    .collect::<String>();
    let note = board.note.as_deref().map_or_else(String::new, |note| {
        let lines = note.lines().collect::<Vec<_>>().join("\n    ");
        if lines.is_empty() {
            "\n  note: (empty)".to_owned()
        } else {
            format!("\n  note:\n    {lines}")
        }
    });
    printable(&format!(
        "{}  {}\n  {}\n  {} · {place}{trash}{area}\n  {objects}{parts}{note}",
        board.name,
        board.id,
        board.link,
        board.edit_right.as_str()
    ))
    .into_owned()
}

fn picture_line(snapped: &Snapped) -> String {
    format!(
        "rectangle {} · {} · {}×{} px · {}",
        snapped.rectangle,
        counted(snapped.objects, "object"),
        snapped.width,
        snapped.height,
        snapped.link
    )
}

fn snapped_text(snapped: &Snapped) -> String {
    let (file, bytes) = match &snapped.delivery {
        Delivery::Written { file, bytes } => (file.as_path(), *bytes),
        Delivery::Inline { bytes } => (Path::new(STDOUT), bytes.len()),
    };
    format!(
        "drew {} · {}\n{}",
        file.display(),
        size_text(byte_count(bytes)),
        picture_line(snapped)
    )
}

fn saved_text(saved: &Saved) -> String {
    let width = saved
        .files
        .iter()
        .map(|file| file.name.chars().count())
        .chain([7])
        .max()
        .unwrap_or_default();
    let files = saved.files.iter().map(|file| {
        format!(
            "  {:<width$}  {} · {}",
            file.name,
            size_text(byte_count(file.bytes)),
            file.about
        )
    });
    let total: u64 = saved
        .images
        .iter()
        .map(|image| byte_count(image.bytes))
        .sum();
    let images = match saved.images.len() {
        1 => format!("  {:<width$}  1 file · {}", "images/", size_text(total)),
        count => format!(
            "  {:<width$}  {count} files · {}",
            "images/",
            size_text(total)
        ),
    };
    let not_saved = saved
        .not_saved
        .iter()
        .map(|reason| format!("  not saved: {reason}"));
    printable(
        &std::iter::once(format!("saved {}/", saved.dir.display()))
            .chain(files)
            .chain([images])
            .chain(not_saved)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .into_owned()
}

fn exported_text(exported: &Exported) -> String {
    let (objects, bytes) = (exported.objects, exported.bytes);
    let before = if exported.state_bytes == bytes {
        String::new()
    } else {
        format!(" ({} before gzip)", exported.state_bytes)
    };
    let place = if exported.inline.is_some() {
        " to stdout".to_owned()
    } else {
        format!(": {}", exported.file.display())
    };
    let listing = exported.object_list.iter().map(|list| {
        format!("\nlisting: {} bytes: {}", list.bytes, list.file.display())
    });
    printable(&format!(
        "exported {}, {bytes} bytes{before}{place}{}",
        counted(objects, "object"),
        listing.collect::<String>()
    ))
    .into_owned()
}

fn imported_text(imported: &Imported) -> String {
    let (verb, request, link) = match &imported.outcome {
        ChangeReport::Sent => ("imported", Vec::new(), imported.link.clone()),
        ChangeReport::DryRun { request } => (
            "would import",
            request.iter().flat_map(PlannedRequest::lines).collect(),
            None,
        ),
        ChangeReport::Held { .. } => ("would import", Vec::new(), None),
    };
    let head = format!(
        "{verb} {} into {} / {}; the board had {}, now {}",
        counted(imported.saved_objects, "object"),
        imported.workspace_name,
        imported.name,
        imported.objects_before,
        imported.objects_after
    );
    let receipt = imported.receipt.map(|receipt| receipt.to_string());
    printable(
        &std::iter::once(head)
            .chain(request)
            .chain(receipt)
            .chain(link)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .into_owned()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use holst_core::{
        command::board::{
            file::{Exported, ObjectList},
            picture::{Saved, SavedFile},
            show::BoardBrief,
        },
        domain::board::EditRight,
    };

    use crate::{
        commands::board::whole::{
            board_text,
            exported_text,
            refuse_terminal,
            saved_text,
        },
        error::Error,
        output::OutputTarget,
    };

    #[test]
    fn no_picture_is_printed_on_a_terminal() {
        assert!(matches!(
            refuse_terminal(OutputTarget::Terminal, true),
            Err(Error::SvgToTerminal)
        ));
        assert!(matches!(
            refuse_terminal(OutputTarget::Terminal, false),
            Err(Error::BinaryToTerminal)
        ));
        assert!(refuse_terminal(OutputTarget::Redirected, false).is_ok());
    }

    #[test]
    fn a_board_is_told_with_its_place_counts_and_parts() {
        let board = BoardBrief {
            id: "b1".to_owned(),
            name: "Plan".to_owned(),
            link: "https://holst/b1".to_owned(),
            workspace: Some("Team".to_owned()),
            folder: Some("Work".to_owned()),
            in_trash: true,
            edit_right: EditRight::Editable,
            rectangle: None,
            count: 3,
            objects: BTreeMap::from([
                ("frame".to_owned(), 1),
                ("sticker".to_owned(), 2),
            ]),
            members: Some(vec![holst_core::command::board::show::Holder {
                user: "ann".to_owned(),
                right: holst_core::command::board::rights::Right::Editor,
            }]),
            rights: None,
            sharing: None,
            note: None,
        };
        assert_eq!(
            board_text(&board),
            "Plan  b1\n  https://holst/b1\n  editable · Team / Work · in trash\n  \
             3 objects: frame 1, sticker 2\n  members: [{\"user\":\"ann\",\"right\":\"editor\"}]"
        );
    }

    #[test]
    fn one_object_is_not_plural() {
        let board = BoardBrief {
            id: "b1".to_owned(),
            name: "Plan".to_owned(),
            link: "L".to_owned(),
            workspace: None,
            folder: None,
            in_trash: false,
            edit_right: EditRight::Editable,
            rectangle: None,
            count: 1,
            objects: BTreeMap::new(),
            members: None,
            rights: None,
            sharing: None,
            note: None,
        };
        assert!(board_text(&board).ends_with("\n  1 object"));
        let exported = Exported {
            board: "b1".to_owned(),
            objects: 1,
            state_bytes: 30,
            bytes: 30,
            file: PathBuf::from("out.ydoc.gz"),
            object_list: None,
            inline: None,
        };
        assert_eq!(
            exported_text(&exported),
            "exported 1 object, 30 bytes: out.ydoc.gz"
        );
    }

    #[test]
    fn a_saved_folder_lists_its_files_and_images() {
        let file = |name: &str, bytes| SavedFile {
            name: name.to_owned(),
            bytes,
            about: "about".to_owned(),
        };
        let saved = Saved {
            dir: PathBuf::from("/d/Plan-b1"),
            files: vec![file("board.svg", 2048)],
            images: vec![file("images/a.png", 1024)],
            not_saved: vec!["a.gif: private".to_owned()],
        };
        assert_eq!(
            saved_text(&saved),
            "saved /d/Plan-b1/\n  board.svg  2.0 KiB · about\n  images/    1 file · 1.0 KiB\n  \
             not saved: a.gif: private"
        );
    }

    #[test]
    fn an_export_tells_its_size_and_where_it_went() {
        let exported = Exported {
            board: "b1".to_owned(),
            objects: 2,
            state_bytes: 90,
            bytes: 30,
            file: PathBuf::from("out.ydoc.gz"),
            object_list: Some(ObjectList {
                file: PathBuf::from("out.ydoc.objects.jsonl"),
                bytes: 7,
            }),
            inline: None,
        };
        assert_eq!(
            exported_text(&exported),
            "exported 2 objects, 30 bytes (90 before gzip): out.ydoc.gz\n\
             listing: 7 bytes: out.ydoc.objects.jsonl"
        );
    }
}
