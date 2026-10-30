//! Token budgets: what every session and the typical replies cost a model.

// The table shows with BUDGET_TABLE=1.

#![expect(clippy::panic_in_result_fn, reason = "asserting is what a test does")]
#![expect(
    clippy::disallowed_methods,
    reason = "tests hand the real environment to the program under test"
)]

use std::{collections::BTreeSet, io::Write as _};

use holst_core::{
    command::{
        object::list::{ListedObject, Listing, TextStyle},
        search::{Found, SearchHit, SkippedBoard as SkippedBoardText, TextMatch},
    },
    domain::{
        board::{ArrowEnds, EditRight, Object, ObjectKind},
        board_text::{
            BoardTextHits,
            BoardTextQuery,
            SkippedBoard,
            found_text,
            index::BoardIndex,
            search::ObjectFilter,
        },
        change::{
            receipt::{BoardStatus, Delivery},
            value::{ChangeUpdate, ChangedObject, ObjectChangeKind},
        },
        limit::Limit,
    },
    effect::board_write_plan::WrittenChange,
};
use rmcp::ServerHandler as _;
use serde_json::{Value, json};

use crate::{reply::change, server::McpServer};

const INSTRUCTIONS_MAX: usize = 660;

// The size BUDGET_TABLE=1 shows, rounded up to the next fifty bytes.
const TOOLS_MAX: usize = 15_000;

const PAGE_MAX: usize = 9_900;

const EDIT_MAX: usize = 200;

const TEXT_SEARCH_MAX: usize = 2_000;

fn tokens(text: &str) -> usize {
    let ascii = text.chars().filter(char::is_ascii).count();
    let other = text.chars().count().saturating_sub(ascii);
    // Whole numbers: four ASCII chars or 1.7 others make a token.
    ascii.div_ceil(4) + (other * 10).div_ceil(17)
}

fn visible_tools(holst: &McpServer) -> Result<String, serde_json::Error> {
    let tools: Vec<Value> = holst
        .listed_tools()
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })
        })
        .collect();
    serde_json::to_string(&tools)
}

const WORDS: [&str; 10] = [
    "Проект",
    "описание",
    "заказ",
    "название",
    "страница",
    "API",
    "формат",
    "YD",
    "расписание",
    "график",
];

fn words_of(index: usize) -> String {
    (0..3 + index % 6)
        .map(|word| {
            WORDS
                .get((index + word * 7) % WORDS.len())
                .copied()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn object_id(prefix: &str, index: usize) -> String {
    format!("{prefix}{index:05}-0000-4000-8000-000000000000")
}

fn grid_position(index: usize) -> (f64, f64) {
    (
        coordinate(index % 40) * 240.5 - 30_000.0,
        coordinate(index / 40) * 230.25 - 28_000.0,
    )
}

fn plain_object(
    id: String,
    kind: ObjectKind,
    index: usize,
    parent: Option<String>,
) -> Object {
    let (x, y) = grid_position(index);
    Object {
        id,
        kind,
        parent,
        x,
        y,
        width: Some(192.0),
        height: Some(192.0),
        z_index: 4_850_335.132_012_044 + coordinate(index),
        text: String::new(),
        color: ["yellow4", "orange6", "purple6"]
            .get(index % 3)
            .map(|&color| color.into()),
        look: None,
        media: None,
        ends: None,
        author: Some(id_of_author()),
        locked: false,
        rotation: 0.0,
        link_to: None,
        file: None,
        fields: Value::Null,
    }
}

fn frame_objects(frames: usize) -> impl Iterator<Item = Object> {
    let top = (0..frames).map(|n| Object {
        width: Some(4_000.0 + coordinate(n)),
        height: Some(3_000.5),
        text: format!("Roadmap {}", words_of(n)),
        parent: None,
        ..plain_object(object_id("1", n), ObjectKind::Frame, n * 40, None)
    });
    let nested = (0..frames * 3).map(move |n| Object {
        width: Some(1_700.0),
        height: Some(2_300.0),
        text: format!("Раздел {n}"),
        ..plain_object(
            object_id("2", n),
            ObjectKind::Frame,
            n,
            Some(object_id("1", n % frames)),
        )
    });
    top.chain(nested)
}

fn sticker_objects(frames: usize) -> impl Iterator<Item = Object> {
    let stickers = (0..3_000).map(move |n| Object {
        text: words_of(n),
        ..plain_object(
            object_id("3", n),
            ObjectKind::Sticker,
            n,
            Some(object_id("2", n % (frames * 3))),
        )
    });
    let arrows = (0..470).map(move |n| Object {
        width: None,
        height: None,
        color: None,
        ends: Some(ArrowEnds {
            start: Some(object_id("3", n)),
            end: Some(object_id("3", n + 1)),
            drawn: None,
        }),
        ..plain_object(
            object_id("4", n),
            ObjectKind::Arrow,
            n,
            Some(object_id("2", n % (frames * 3))),
        )
    });
    stickers.chain(arrows)
}

fn other_objects(frames: usize) -> impl Iterator<Item = Object> {
    let cells = (0..500).map(|n| Object {
        width: Some(120.0),
        height: Some(40.0),
        text: format!("{n}"),
        ..plain_object(object_id("5", n), ObjectKind::TableCell, n, None)
    });
    let images = (0..100).map(move |n| Object {
        width: Some(972.0),
        height: Some(1_246.0),
        color: None,
        ..plain_object(
            object_id("6", n),
            ObjectKind::Image,
            n,
            Some(object_id("1", n % frames)),
        )
    });
    cells.chain(images)
}

fn synthetic_board(frames: usize) -> Vec<Object> {
    frame_objects(frames)
        .chain(sticker_objects(frames))
        .chain(other_objects(frames))
        .collect()
}

const BOARD: &str = "11111111-2222-4333-8444-555555555555";

fn coordinate(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn id_of_author() -> String {
    "a0000000-0000-4000-8000-000000000000".to_owned()
}

/// What a budget test can fail on.
#[derive(Debug, thiserror::Error)]
enum BudgetError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Listing(#[from] crate::error::Error),
    #[error(transparent)]
    Write(#[from] std::io::Error),
}

/// The first page of `object` over the whole board, as the model reads it.
fn listed(holst: &McpServer, objects: &[Object]) -> String {
    let page = objects
        .iter()
        .take(100)
        .map(|object| ListedObject {
            link: format!("https://app.holst.so/board/{BOARD}?objectId={}", object.id),
            object: object.clone(),
            drawn: None,
            file: None,
            card: None,
            tags: Vec::new(),
            reactions: std::collections::BTreeMap::new(),
            style: TextStyle::default(),
            detail: Vec::new(),
        })
        .collect();
    holst.object_listing_text(&Listing {
        board: BOARD.to_owned(),
        link: format!("https://app.holst.so/board/{BOARD}"),
        edit_right: EditRight::Editable,
        rectangle: None,
        matched: objects.len(),
        from: 0,
        objects: page,
        next: Some(100),
    })
}

/// What the core's search reports of a text search that found `text`.
fn searched(query: &str, text: &BoardTextHits) -> Found {
    let hits = text
        .hits
        .iter()
        .map(|hit| {
            let (before, matched, _) = hit.excerpt.parts();
            let start = before.chars().count();
            SearchHit::Text(TextMatch {
                board: hit.board.clone(),
                board_name: text.board_name(&hit.board),
                object: hit.object.clone(),
                kind: hit.kind.as_str().to_owned(),
                excerpt: hit.excerpt.text.clone(),
                matched: [start, start + matched.chars().count()],
                fuzzy: hit.fuzzy,
                link: String::new(),
                score: hit.score,
            })
        })
        .collect();
    Found {
        query: query.to_owned(),
        hits,
        total: text.total,
        boards_read: text.boards_read,
        skipped_workspaces: Vec::new(),
        skipped_boards: text
            .unread
            .iter()
            .map(|board| SkippedBoardText {
                id: board.board.clone(),
                name: text.board_name(&board.board),
                reason: board.reason.clone(),
            })
            .collect(),
    }
}

fn edit_reply(holst: &McpServer) -> String {
    let written = WrittenChange {
        change: ChangeUpdate {
            update: vec![0; 527],
            objects: vec![ChangedObject {
                id: "66666666-7777-4888-8999-aaaaaaaaaaaa".into(),
                kind: ObjectKind::Sticker,
                change: ObjectChangeKind::Created,
                fields: Vec::new(),
                link: None,
            }],
            kept: BTreeSet::new(),
        },
        update_id: 42,
        delivery: Delivery::Accepted,
        placement: BoardStatus::Live,
    };
    let board = "11111111-2222-4333-8444-555555555555";
    let link =
        |object: &str| format!("https://app.holst.so/board/{board}?objectId={object}");
    change::written(&written.report(board, link), &|id| holst.state.alias(id))
}

fn measured_replies(
    holst: &McpServer,
    board: &[Object],
) -> Result<Vec<(&'static str, String, usize)>, BudgetError> {
    let indexes = [std::sync::Arc::new(BoardIndex::from_objects(
        board,
        BOARD,
        Vec::new(),
    ))];
    let names = [(BOARD.to_owned(), "Roadmap of documents".to_owned())];
    // Its own server: aliases handed out here leave other replies as they were.
    let search = BoardTextQuery {
        query: "заказ",
        filter: ObjectFilter::default(),
        limit: Limit::from(10),
    };
    let found = found_text(&indexes, names, Vec::new(), &search);
    let text_search = McpServer::offline().found_text(&searched("заказ", &found));
    Ok(vec![
        (
            "instructions",
            holst.get_info().instructions.unwrap_or_default(),
            INSTRUCTIONS_MAX,
        ),
        (
            "tools/list (model-visible)",
            visible_tools(holst)?,
            TOOLS_MAX,
        ),
        (
            "object, 100 of 4400 objects",
            listed(holst, board),
            PAGE_MAX,
        ),
        ("object_write add", edit_reply(holst), EDIT_MAX),
        (
            "search mode=board_text, 10 hits",
            text_search,
            TEXT_SEARCH_MAX,
        ),
    ])
}

#[test]
fn session_and_replies_stay_in_budget() -> Result<(), BudgetError> {
    let holst = McpServer::offline();
    let replies = measured_replies(&holst, &synthetic_board(34))?;
    let table = replies.iter().fold(
        "| what | bytes | budget | tokens (est.) |\n|---|---:|---:|---:|\n".to_owned(),
        |table, (what, text, budget)| {
            table
                + &format!(
                    "| {what} | {} | {budget} | {} |\n",
                    text.len(),
                    tokens(text)
                )
        },
    );
    // Shown on request: `BUDGET_TABLE=1 cargo test -p holst-mcp budget`.
    std::env::var_os("BUDGET_TABLE")
        .map(|_| writeln!(std::io::stdout().lock(), "{table}"))
        .transpose()?;
    let over: Vec<_> = replies
        .iter()
        .filter(|(_, text, budget)| text.len() > *budget)
        .map(|(what, text, budget)| format!("{what}: {} > {budget} bytes", text.len()))
        .collect();
    assert!(over.is_empty(), "over budget: {over:?}\n{table}");
    Ok(())
}

#[test]
fn text_search_lists_one_line_per_hit() {
    let holst = McpServer::offline();
    let board = synthetic_board(34);
    let indexes = [std::sync::Arc::new(BoardIndex::from_objects(
        &board,
        BOARD,
        Vec::new(),
    ))];
    let names = [(BOARD.to_owned(), "Roadmap".to_owned())];
    let failed = vec![SkippedBoard {
        board: "b2".to_owned(),
        reason: "the board server refused".to_owned(),
    }];
    let search = BoardTextQuery {
        query: "ЗАКАЗ название",
        filter: ObjectFilter::default(),
        limit: Limit::from(3),
    };
    let found = found_text(&indexes, names, failed, &search);
    let reply = holst.found_text(&searched("ЗАКАЗ название", &found));
    insta::assert_snapshot!(reply, @r#"
    "ЗАКАЗ название": 3 of 708 (boards read: 1)
    @1 sticker in @2 Roadmap: заказ график формат название Проект
    @3 sticker in @2 Roadmap: заказ график формат название Проект
    @4 sticker in @2 Roadmap: заказ график формат название Проект
    boards not read: b2: the board server refused
    "#);
}
