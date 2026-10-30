//! `object edit --extract`: pages of a file as file objects.

use std::collections::BTreeMap;

use futures_util::{TryStreamExt as _, stream};
use itertools::Itertools as _;
use serde_json::{Value, json};

use crate::{
    command::{
        board::show::{Loading, board_id},
        board_load::CacheUse,
        board_write::{BoardWrite, change_board_with},
        object::{
            EditArgs,
            EditFields,
            change::{Removed, load_board_read, named, settled},
        },
        table::{ArgumentGroup as _, not_value},
    },
    domain::{
        area::scene::{Scene, drawn_scale},
        board::{EditRight, Object, ObjectKind, objects},
        change::{
            media::{page_count, uncounted},
            receipt::ChangeReport,
            value::{BoardChange, Pasted},
        },
        geometry::Point,
        lookup::rules::find_id,
        response::PagesInfo,
    },
    effect::{board_write::WriteMode, confirmation::ALWAYS},
    error::{Error, Result, board_change::BoardChangeError},
    protocol::document::{FILE_NAME, POSITION},
    remote::{
        api::files::{convert_pages, page_count as page_count_of},
        client::Client,
    },
};

const EDIT: &str = "object edit";
/// BUNDLE: at most 50 pages (`extractPages`, 209875).
const MOST_PAGES: usize = 50;
/// BUNDLE: a run converts at most 20 pages (209893).
const RUN_PAGES: u32 = 20;
/// BUNDLE: the copies stand 8 to a row (209912).
const ROW_PAGES: u32 = 8;
/// BUNDLE: the gap is 64 at the file's scale (209881).
const GAP: f64 = 64.0;

/// One file object per page, below the file; uncounted pages counted first.
///
/// # Errors
/// More than one object or any other field beside `--extract`, an object that
/// is not a page file or has no valid pages, pages that are not a list or are
/// more than 50, a board that cannot be changed, a run the page service
/// refuses, and errors of the requests and of the write.
pub(super) async fn extracted(
    client: &Client,
    loading: Loading<'_>,
    args: &EditArgs,
    pages: &str,
) -> Result<Removed> {
    refuse_beside(args)?;
    let board = board_id(
        client,
        &args.board,
        args.workspace.as_deref(),
        CacheUse::Use,
    )
    .await?;
    let (loaded, read) = load_board_read(client, &loading, &board).await?;
    // Before any conversion: a board that cannot be changed takes no page.
    match loaded.right {
        EditRight::Editable => Ok(()),
        EditRight::ReadOnly => Err(Error::BoardChange(BoardChangeError::ReadOnly)),
    }?;
    let listed = objects(&loaded.document);
    let scene = Scene::new(&listed);
    let (file, name, stored) = source(&scene, &one_object(args)?)?;
    let asked = if stored.is_none() {
        Some(counted_by_service(client, &name).await?)
    } else {
        None
    };
    let count = stored
        .or_else(|| asked.as_ref().map(|info| info.count))
        .unwrap_or_default();
    let wanted = parse_pages(pages, count)?;
    let pasted = Pasted {
        objects: copies(&scene, file, &wanted, asked)?,
        tags: BTreeMap::new(),
        shift: Point::ORIGIN,
    };
    let write = async |mode| {
        change_board_with(
            client,
            loading,
            BoardWrite {
                board: &board,
                change: BoardChange::Paste(pasted.clone()),
                release: ALWAYS,
                guard: None,
                mode,
            },
            read.clone(),
        )
        .await
    };
    // Before any conversion: the copies are built, to see they can be written.
    let built = write(WriteMode::DryRun).await?;
    let runs = runs_of(&wanted);
    let report = match args.dry_run {
        WriteMode::DryRun => ChangeReport {
            warnings: built
                .warnings
                .into_iter()
                .chain(runs.iter().map(|(first, last)| {
                    format!(
                        "would ask the server to convert pages {first} to {last} of {name}"
                    )
                }))
                .collect(),
            ..built
        },
        WriteMode::Send => {
            stream::iter(runs.into_iter().map(Ok::<_, Error>))
                .try_for_each(|(first, last)| convert_pages(client, &name, first, last))
                .await?;
            write(WriteMode::Send).await?
        }
    };
    Ok(settled(report, args))
}

/// What `--extract` cannot stand beside: `--items`, a picture, any other field.
fn refuse_beside(args: &EditArgs) -> Result<()> {
    let others = EditFields {
        extract: None,
        ..args.edit.clone()
    };
    match (
        args.items.0.is_empty(),
        args.path.is_some(),
        others.first_given(),
    ) {
        (false, _, _) => Err(not_value(EDIT, "items", "left out with --extract")),
        (_, true, _) => Err(not_value(EDIT, "path", "left out with --extract")),
        (_, _, Some(name)) => Err(not_value(EDIT, name, "left out with --extract")),
        (true, false, None) => Ok(()),
    }
}

fn one_object(args: &EditArgs) -> Result<String> {
    match named(&args.board, &args.object, EDIT)?.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err(not_value(EDIT, "object", "one file with --extract")),
    }
}

/// File `reference` with its stored name and page count, if it has one.
fn source<'a>(
    scene: &Scene<'a>,
    reference: &str,
) -> Result<(&'a Object, String, Option<u32>)> {
    let id = find_id(
        scene.objects().iter().map(|object| object.id.as_str()),
        reference,
    )?;
    let file = scene.get(&id).ok_or_else(|| {
        Error::BoardChange(BoardChangeError::NoObject {
            reference: id.clone(),
        })
    })?;
    let none_counted = || {
        not_value(
            EDIT,
            "extract",
            &format!("a file with pages: {id} has none counted"),
        )
    };
    match (
        &file.kind,
        file.fields.get(FILE_NAME).and_then(Value::as_str),
    ) {
        (ObjectKind::File, Some(name)) => match page_count(&file.fields) {
            Some(count) if count > 0 => Ok((file, name.to_owned(), Some(count))),
            _ if uncounted(&file.fields).is_some() => Ok((file, name.to_owned(), None)),
            _ => Err(none_counted()),
        },
        (ObjectKind::File, None) => Err(none_counted()),
        (kind, _) => Err(not_value(
            EDIT,
            "extract",
            &format!("a file with pages: {id} is {}", kind.short_name()),
        )),
    }
}

/// Counts pages of `name` as the web does (main.a33651a4.js 209861).
///
/// # Errors
/// The request, or a file the service finds no pages in.
async fn counted_by_service(client: &Client, name: &str) -> Result<PagesInfo> {
    let info = page_count_of(client, name).await?;
    if info.valid && info.count > 0 {
        Ok(info)
    } else {
        Err(not_value(
            EDIT,
            "extract",
            &format!("a file with pages: {name} has no valid pages"),
        ))
    }
}

fn unreadable() -> Error {
    not_value(
        EDIT,
        "extract",
        "all, or pages and ranges such as 1-3,5, from page 1",
    )
}

/// Pages `list` names: `all`, or commas of pages and ranges, each held to
/// `1..=count`; reversed ranges turned, order kept (main.a33651a4.js 116826).
///
/// # Errors
/// An item that is not a page or a range, no page, more than 50 pages.
pub(super) fn parse_pages(list: &str, count: u32) -> Result<Vec<u32>> {
    let held = |text: &str| -> Result<u32> {
        text.chars()
            .all(|digit| digit.is_ascii_digit())
            .then(|| text.parse::<u32>().ok())
            .flatten()
            .map(|page| page.clamp(1, count.max(1)))
            .ok_or_else(unreadable)
    };
    let pages: Vec<u32> = match list.trim() {
        "all" => (1..=count).collect(),
        listed => listed
            .split(',')
            .map(|item| listed_item(item.trim(), &held))
            .collect::<Result<Vec<Vec<u32>>>>()?
            .concat(),
    };
    match pages.len() {
        0 => Err(unreadable()),
        taken if taken > MOST_PAGES => {
            Err(not_value(EDIT, "extract", "at most 50 pages"))
        }
        _ => Ok(pages),
    }
}

/// The pages of one item of the list: a page, or a range turned if reversed.
fn listed_item(item: &str, held: &impl Fn(&str) -> Result<u32>) -> Result<Vec<u32>> {
    match item.split('-').collect::<Vec<_>>().as_slice() {
        [page] => held(page.trim()).map(|page| vec![page]),
        [from, to] => held(from.trim()).and_then(|start| {
            held(to.trim()).map(|end| (start.min(end)..=start.max(end)).collect())
        }),
        _ => Err(unreadable()),
    }
}

/// The runs of consecutive pages, each of at most 20 pages, as first and last.
fn runs_of(pages: &[u32]) -> Vec<(u32, u32)> {
    pages
        .iter()
        .map(|&page| (page, page))
        .coalesce(|(first, last), (page, _)| {
            (last + 1 == page && last - first < RUN_PAGES - 1)
                .then_some((first, page))
                .ok_or(((first, last), (page, page)))
        })
        .collect()
}

/// A copy of `file` per page, 8 to a row below; `counted` page info is copied.
/// A copy stands for its source under the page's name; `Pasted` gives own ids.
fn copies(
    scene: &Scene<'_>,
    file: &Object,
    pages: &[u32],
    counted: Option<PagesInfo>,
) -> Result<Vec<Object>> {
    let own = scene
        .rectangle_of(file)
        .ok_or_else(|| not_value(EDIT, "extract", "a file with a box on the board"))?;
    let gap = GAP * drawn_scale(file);
    let fields = file.fields.as_object().cloned().unwrap_or_default();
    Ok(pages
        .iter()
        .zip(0_u32..)
        .map(|(page, slot)| {
            let x = file.x + f64::from(slot % ROW_PAGES) * (own.width + gap);
            let y = file.y
                + own.height
                + gap
                + f64::from(slot / ROW_PAGES) * (own.height + gap);
            let made = [
                ("pinnedPage", json!(page)),
                ("extracted", json!(true)),
                (POSITION, json!({ "x": x, "y": y })),
            ]
            .into_iter()
            .chain(counted.map(|info| ("pagesInfo", json!(info))))
            .map(|(name, value)| (name.to_owned(), value));
            Object {
                id: format!("{}#{slot}", file.id),
                x,
                y,
                fields: Value::Object(fields.clone().into_iter().chain(made).collect()),
                ..file.clone()
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use rstest::rstest;

    use super::{parse_pages, runs_of};
    use crate::error::Result;

    #[rstest]
    #[case::all("all", 4, vec![1, 2, 3, 4])]
    #[case::a_list_and_a_range("1-3,5", 6, vec![1, 2, 3, 5])]
    #[case::spaces_are_dropped(" 2 , 4-5 ", 6, vec![2, 4, 5])]
    #[case::the_order_given_is_kept("5,1,3-2", 6, vec![5, 1, 2, 3])]
    #[case::a_page_below_one_is_the_first("0", 3, vec![1])]
    #[case::a_reversed_range_is_turned("5-2", 6, vec![2, 3, 4, 5])]
    #[case::a_page_past_the_end_is_the_last("9", 3, vec![3])]
    #[case::a_range_is_cut_at_the_end("2-9", 3, vec![2, 3])]
    fn pages_are_read_as_the_menu_reads_them(
        #[case] list: &str,
        #[case] count: u32,
        #[case] expected: Vec<u32>,
    ) -> Result<()> {
        assert_eq!(parse_pages(list, count)?, expected);
        Ok(())
    }

    #[rstest]
    #[case::nothing("")]
    #[case::a_word("one")]
    #[case::an_empty_item("1,,2")]
    #[case::three_ends("1-2-3")]
    #[case::a_sign("-3")]
    fn a_list_that_is_not_pages_is_refused(#[case] list: &str) {
        assert!(parse_pages(list, 6).is_err());
    }

    #[test]
    fn more_than_fifty_pages_are_refused_saying_so() {
        let refused = parse_pages("all", 51).err().map(|error| error.to_string());
        assert_eq!(
            refused
                .as_deref()
                .map(|text| text.ends_with("must be at most 50 pages")),
            Some(true),
            "{refused:?}"
        );
    }

    #[rstest]
    #[case::one_run(vec![1, 2, 3], vec![(1, 3)])]
    #[case::a_gap_ends_a_run(vec![1, 2, 4], vec![(1, 2), (4, 4)])]
    #[case::an_order_turned_ends_a_run(vec![5, 1], vec![(5, 5), (1, 1)])]
    #[case::twenty_pages_fill_a_run(
        (1..=25).collect(),
        vec![(1, 20), (21, 25)]
    )]
    fn consecutive_pages_are_converted_in_runs_of_twenty(
        #[case] pages: Vec<u32>,
        #[case] expected: Vec<(u32, u32)>,
    ) {
        assert_eq!(runs_of(&pages), expected);
    }

    #[test]
    fn fifty_pages_are_taken() -> Result<()> {
        assert_eq!(parse_pages("1-50", 80)?.len(), 50);
        Ok(())
    }
}
