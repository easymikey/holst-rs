//! Board indexes and content search on synthetic boards, no network.

use divan::{AllocProfiler, Bencher, black_box};
use holst_core::domain::{
    board::{Object, ObjectKind},
    board_text::{
        index::BoardIndex,
        search::{ObjectFilter, TextCorpus},
    },
    limit::Limit,
};
use serde_json::Value;

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

const QUERIES: &[&str] = &[
    "название",
    "описание проекта",
    "формат страница",
    "zeta42",
    "провекра",
];

fn main() {
    divan::main();
}

const SYLLABLES: [&str; 16] = [
    "про", "вер", "ка", "оп", "ис", "ание", "про", "ект", "до", "ку", "мент", "ста",
    "тус", "api", "flow", "ре",
];

fn words() -> Vec<String> {
    let built = (0..400usize).map(|n| {
        (0..2 + n % 3)
            .map(|at| {
                SYLLABLES
                    .get((n * 7 + at * 5 + n / 16) % SYLLABLES.len())
                    .copied()
                    .unwrap_or_default()
            })
            .collect::<String>()
    });
    [
        "название",
        "описание",
        "проекта",
        "формат",
        "страница",
        "Ёжик",
        "заказ",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(built)
    .collect()
}

fn board(board: usize, objects: usize, words: &[String]) -> BoardIndex {
    let text = |n: usize| {
        (0..=(n * 13 + board) % 7)
            .map(|at| {
                words
                    .get((n * 31 + at * 17 + board * 7) % words.len())
                    .map_or("", String::as_str)
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let stickers: Vec<Object> = (0..objects)
        .map(|n| Object {
            id: format!("{board:08x}-0000-4000-8000-{n:012x}"),
            kind: match n % 10 {
                0 => ObjectKind::Frame,
                1 => ObjectKind::Text,
                2 | 7 => ObjectKind::Arrow,
                _ => ObjectKind::Sticker,
            },
            parent: Some(format!("{board:08x}-0000-4000-8000-{:012x}", n / 40 * 40)),
            x: f64::from(u32::try_from(n % 64).unwrap_or_default()) * 240.5,
            y: f64::from(u32::try_from(n / 64).unwrap_or_default()) * 230.25,
            width: Some(192.0),
            height: Some(192.0),
            z_index: 4_850_335.132 + f64::from(u32::try_from(n).unwrap_or_default()),
            text: match (n % 10, n % 997) {
                (2 | 7, _) => String::new(),
                (_, 0) => format!("{} zeta42", text(n)),
                _ => text(n),
            },
            color: ["yellow4", "orange6", "purple6"]
                .get(n % 3)
                .map(|&c| c.into()),
            look: None,
            ends: None,
            author: Some("a0000000-0000-4000-8000-000000000000".into()),
            locked: false,
            rotation: 0.0,
            file: None,
            fields: Value::Null,
        })
        .collect();
    BoardIndex::from_objects(&stickers, &format!("{board:08x}-board"), vec![1, 2, 3, 4])
}

fn corpus(boards: usize, objects: usize) -> Vec<BoardIndex> {
    let words = words();
    (0..boards).map(|n| board(n, objects, &words)).collect()
}

fn search(bencher: Bencher<'_, '_>, indexes: &[BoardIndex], query: &str) {
    let corpus = TextCorpus::of(indexes);
    let filter = ObjectFilter::default();
    bencher.bench(|| corpus.search(black_box(query), &filter, Limit::from(20)).1);
}

#[divan::bench(args = QUERIES)]
fn content_search_1x4400(bencher: Bencher<'_, '_>, query: &str) {
    search(bencher, &corpus(1, 4_400), query);
}

#[divan::bench(args = QUERIES)]
fn content_search_50x4000(bencher: Bencher<'_, '_>, query: &str) {
    search(bencher, &corpus(50, 4_000), query);
}

#[divan::bench]
fn corpus_build_50x4000(bencher: Bencher<'_, '_>) {
    let indexes = corpus(50, 4_000);
    bencher.bench(|| TextCorpus::of(black_box(&indexes[..])).is_empty());
}

#[divan::bench]
fn index_decode(bencher: Bencher<'_, '_>) {
    let encoded = board(0, 4_400, &words()).encode().unwrap_or_default();
    bencher
        .counter(divan::counter::BytesCount::new(encoded.len()))
        .bench(|| BoardIndex::decode(black_box(&encoded)).map(|index| index.len()));
}

#[divan::bench]
fn index_encode(bencher: Bencher<'_, '_>) {
    let index = board(0, 4_400, &words());
    bencher.bench(|| index.encode().map(|bytes| bytes.len()));
}
