use itertools::Itertools;
use proptest::{
    collection::vec,
    prelude::{Strategy, prop_assert, prop_oneof, proptest},
    sample::select,
};
use rstest::rstest;

use crate::{
    domain::{
        board::{Object, ObjectKind, tests::plain_object},
        board_text::{
            index::BoardIndex,
            search::{EXCERPT_CHARS, Excerpt, ObjectFilter, TextCorpus, within_one},
        },
        limit::Limit,
        tree::name_index::build::normalized,
    },
    laws::config,
};

fn stickers(board: &str, texts: &[&str]) -> BoardIndex {
    let objects: Vec<Object> = texts
        .iter()
        .enumerate()
        .map(|(n, text)| plain_object(&format!("s{n}"), ObjectKind::Sticker, text))
        .collect();
    BoardIndex::from_objects(&objects, board, Vec::new())
}

fn boards() -> Vec<BoardIndex> {
    let planning = stickers(
        "planning",
        &[
            "План",
            "План релиза Q4",
            "Новый план",
            "Планирование спринта",
            "Сплан команды",
            "Релиз: итоговый план",
            "Ёжик в тумане",
            "Е\u{308}жик сказка",
            "ПЛАН В КАПСЕ",
            "релиз\nи план\tпосле",
        ],
    );
    let mixed = BoardIndex::from_objects(
        &[
            plain_object("f0", ObjectKind::Frame, "Планы"),
            plain_object("t0", ObjectKind::Text, "Проверка клиента"),
            plain_object("t1", ObjectKind::Text, "Провекра документов"),
            plain_object("t2", ObjectKind::Text, "Пробверка статуса"),
            plain_object("a0", ObjectKind::Arrow, ""),
            plain_object("c0", ObjectKind::TableCell, "проверить"),
        ],
        "mixed",
        Vec::new(),
    );
    vec![planning, mixed]
}

fn render(
    corpus: &TextCorpus<'_>,
    query: &str,
    filter: &ObjectFilter,
    limit: usize,
) -> String {
    let (hits, total) = corpus.search(query, filter, Limit::from(limit));
    let lines = hits
        .iter()
        .map(|hit| {
            let (before, matched, after) = hit.excerpt.parts();
            let typo = "~".repeat(usize::from(hit.fuzzy));
            format!(
                "{:>4}{typo:1} {}/{} {}: {before}[{matched}]{after}\n",
                hit.score,
                hit.board,
                hit.object,
                hit.kind.as_str()
            )
        })
        .join("");
    format!("{query:?}: {} of {total}\n{lines}", hits.len())
}

#[test]
fn ranking_on_a_synthetic_corpus() {
    let boards = boards();
    let corpus = TextCorpus::of(boards.iter());
    let all = ObjectFilter::default();
    let report = [
        render(&corpus, "план", &all, 0),
        render(&corpus, "релиз план", &all, 0),
        render(&corpus, "ЕЖИК", &all, 0),
        render(&corpus, "проверка", &all, 10),
        render(&corpus, "провекра", &all, 1),
    ]
    .concat();
    insta::assert_snapshot!(report);
}

#[rstest]
#[case::yo_in_text("ежик", "Ёжик", "Ёжик")]
#[case::yo_in_query("ёжик", "ежик", "ежик")]
#[case::decomposed_text("ежик", "Е\u{308}жик", "Е\u{308}жик")]
#[case::decomposed_query("е\u{308}жик", "Ежик", "Ежик")]
#[case::case("ПЛАН", "мой план", "план")]
#[case::punctuation("план-б", "План Б!", "План Б")]
#[case::invisible("план", "пл\u{200b}ан", "пл\u{200b}ан")]
fn normalisation_matches_and_marks_the_original(
    #[case] query: &str,
    #[case] text: &str,
    #[case] marked: &str,
) {
    let boards = [stickers("b", &[text])];
    let corpus = TextCorpus::of(boards.iter());
    let (hits, total) = corpus.search(query, &ObjectFilter::default(), Limit::from(10));
    assert_eq!(total, 1, "{query:?} in {text:?}");
    assert_eq!(hits.first().map(|hit| hit.excerpt.parts().1), Some(marked));
}

#[test]
fn typos_are_forgiven_only_below_the_limit() {
    let texts = ["проверка один", "проверка два", "провекра три"];
    let boards = [stickers("b", &texts)];
    let corpus = TextCorpus::of(boards.iter());
    let fuzzy = |limit| {
        let (hits, total) =
            corpus.search("проверка", &ObjectFilter::default(), Limit::from(limit));
        (hits.iter().filter(|hit| hit.fuzzy).count(), total)
    };
    assert_eq!(fuzzy(2), (0, 2));
    assert_eq!(fuzzy(3), (1, 3));
    assert_eq!(fuzzy(0), (1, 3));
}

#[test]
fn short_words_and_phrases_get_no_typos() {
    let boards = [stickers("b", &["кот", "план релиза"])];
    let corpus = TextCorpus::of(boards.iter());
    assert_eq!(
        corpus
            .search("кто", &ObjectFilter::default(), Limit::from(10))
            .1,
        0
    );
    assert_eq!(
        corpus
            .search("плна релиза", &ObjectFilter::default(), Limit::from(10))
            .1,
        0
    );
}

#[test]
fn filters_keep_one_board_or_one_kind() {
    let boards = boards();
    let corpus = TextCorpus::of(boards.iter());
    let objects = |filter: ObjectFilter| {
        corpus
            .search("план", &filter, Limit::All)
            .0
            .into_iter()
            .map(|hit| format!("{}/{}", hit.board, hit.object))
            .collect::<Vec<_>>()
    };
    let only_mixed = ObjectFilter {
        board: Some("mixed".into()),
        kind: None,
    };
    let only_frames = ObjectFilter {
        board: None,
        kind: Some(ObjectKind::Frame),
    };
    let nowhere = ObjectFilter {
        board: Some("nowhere".into()),
        kind: None,
    };
    assert_eq!(objects(only_mixed), ["mixed/f0"]);
    assert_eq!(objects(only_frames), ["mixed/f0"]);
    assert_eq!(objects(nowhere), Vec::<String>::new());
}

#[test]
fn long_texts_are_cut_around_the_match() {
    let long = format!("{} середина {}", "слово ".repeat(40), "хвост ".repeat(40));
    let boards = [stickers("b", &[&long])];
    let corpus = TextCorpus::of(boards.iter());
    let (hits, _) = corpus.search("середина", &ObjectFilter::default(), Limit::from(1));
    let excerpt = hits.first().map(|hit| hit.excerpt.clone());
    let (before, matched, after) =
        excerpt.as_ref().map(Excerpt::parts).unwrap_or_default();
    assert!(
        before.starts_with('…') && after.ends_with('…'),
        "{excerpt:?}"
    );
    assert_eq!(matched, "середина");
    assert!(
        excerpt
            .is_some_and(|excerpt| excerpt.text.chars().count() <= EXCERPT_CHARS + 2)
    );
}

#[test]
fn empty_queries_and_corpora_find_nothing() {
    let boards = [stickers("b", &["", "текст"])];
    let corpus = TextCorpus::of(boards.iter());
    assert!(!corpus.is_empty());
    assert_eq!(
        corpus
            .search("  !! ", &ObjectFilter::default(), Limit::from(10))
            .1,
        0
    );
    let none: [BoardIndex; 0] = [];
    assert_eq!(
        TextCorpus::of(none.iter())
            .search("текст", &ObjectFilter::default(), Limit::from(10))
            .1,
        0
    );
}

#[rstest]
#[case::same("план", "план", Distance::Near)]
#[case::change("план", "плен", Distance::Near)]
#[case::swap("план", "плна", Distance::Near)]
#[case::insert("план", "пллан", Distance::Near)]
#[case::delete("план", "пан", Distance::Near)]
#[case::two_changes("план", "клен", Distance::Far)]
#[case::too_long("план", "планов", Distance::Far)]
fn one_edit_apart(#[case] a: &str, #[case] b: &str, #[case] expected: Distance) {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    assert_eq!(distance(&a, &b), expected);
    assert_eq!(distance(&b, &a), expected);
}

#[derive(Debug, Default, PartialEq)]
enum Distance {
    Near,
    #[default]
    Far,
}

fn distance(a: &[char], b: &[char]) -> Distance {
    [Distance::Far, Distance::Near]
        .into_iter()
        .nth(usize::from(within_one(a, b)))
        .unwrap_or_default()
}

const WORDS: [&str; 10] = [
    "план",
    "Релиз",
    "ёжик",
    "ежик",
    "клиент",
    "Клиента",
    "api",
    "API-ключ",
    "е\u{308}ж",
    "q4",
];

fn text() -> impl Strategy<Value = String> {
    vec(
        (
            select(&WORDS[..]),
            prop_oneof![" ", "\n", ", ", "-", "", "\u{200b}"],
        ),
        0..8,
    )
    .prop_map(|parts| {
        parts
            .into_iter()
            .fold(String::new(), |mut text, (word, gap)| {
                text.push_str(word);
                text.push_str(&gap);
                text
            })
    })
}

fn normal(text: &str) -> String {
    normalized(text)
}

proptest! {
    #![proptest_config(config())]

    /// Every exact hit holds every query word once normalized; typos only for one word.
    #[test]
    fn every_hit_holds_all_query_words(
        texts in vec(text(), 1..20),
        query in vec(select(&WORDS[..]), 1..3).prop_map(|words| words.join(" ")),
        limit in 0usize..5,
    ) {
        let objects: Vec<&str> = texts.iter().map(String::as_str).collect();
        let boards = [stickers("b", &objects)];
        let corpus = TextCorpus::of(boards.iter());
        let (hits, total) = corpus.search(&query, &ObjectFilter::default(), Limit::from(limit));
        prop_assert!(limit == 0 || hits.len() <= limit);
        prop_assert!(hits.len() <= total);
        let query = normal(&query);
        let words: Vec<&str> = query.split(' ').collect();
        let exact = hits.iter().filter(|hit| !hit.fuzzy).count();
        let checked: Vec<(bool, String)> = hits
            .iter()
            .map(|hit| {
                let at: usize = hit.object.trim_start_matches('s').parse().unwrap_or(usize::MAX);
                (hit.fuzzy, normal(texts.get(at).map_or("", String::as_str)))
            })
            .collect();
        prop_assert!(
            checked
                .iter()
                .filter(|(fuzzy, _)| !fuzzy)
                .all(|(_, text)| words.iter().all(|word| text.contains(word))),
            "{query:?} not all in {checked:?}"
        );
        prop_assert!(
            checked.iter().all(|(fuzzy, _)| !fuzzy || (words.len() == 1 && (limit == 0 || exact < limit))),
            "typo forgiven for {query:?} with {exact} exact of limit {limit}"
        );
    }
}
