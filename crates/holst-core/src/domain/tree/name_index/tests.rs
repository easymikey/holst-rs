#![expect(
    clippy::panic_in_result_fn,
    reason = "assertions are the point of a test"
)]

use icu_normalizer::ComposingNormalizerBorrowed;
use rstest::rstest;
use serde_json::{Value, json};

use crate::{
    domain::{
        limit::Limit,
        node::NodeKind,
        tree,
        tree::name_index::{
            build::{NameIndex, NodeIndex, is_inert, is_invisible, normalized},
            query::{NodeFilter, best_per_node, overlapping},
        },
    },
    error::Result,
};

type TestResult = Result<()>;

const W: &str = "00000000-0000-4000-8000-000000000000";
const ALL: NodeFilter = NodeFilter {
    kind: None,
    in_trash: false,
};

fn id(serial: u32) -> String {
    format!("00000001-0000-4000-8000-{serial:012x}")
}

fn index(channels: &Value) -> Result<NameIndex> {
    NameIndex::from_tree_json(&json!({ "channels": channels }).to_string())
}

fn boards(names: &[&str]) -> Result<NameIndex> {
    let channels = names.iter().zip(1..).map(|(name, n)| {
            json!({ "id": id(n), "type": "board", "name": name, "parent_id": W })
        });
    index(&channels.collect())
}

#[rstest]
#[case::repeats_in_one_name("пл")]
#[case::overlapping("аа")]
#[case::exact("планы")]
#[case::nothing("нет")]
fn sparse_hits_match_the_dense_table(
    #[case] query: &str,
    #[values(None, Some(NodeKind::Board), Some(NodeKind::Folder))] kind: Option<
        NodeKind,
    >,
) -> TestResult {
    let index = boards(&["аааа", "планы пл пл", "Планы", "ааа планы", "пл"])?;
    let filter = NodeFilter {
        kind,
        in_trash: false,
    };
    let scored = || {
        overlapping(index.normalized_names.as_bytes(), query.as_bytes())
            .map(|position| index.score(position, query.len()))
    };
    let dense = index
        .dense_best(scored())
        .into_iter()
        .zip(0..)
        .map(|(s, n)| (n, s));
    assert_eq!(
        index.hits(best_per_node(scored()), filter),
        index.hits(dense, filter),
        "{query}"
    );
    Ok(())
}

fn sample() -> Result<NameIndex> {
    index(&json!([
        { "id": id(1), "type": "folder", "name": "Стратегия", "parent_id": W },
        { "id": id(2), "type": "board", "name": "Планы Q4", "parent_id": id(1) },
        { "id": id(3), "type": "board", "name": "Планы", "parent_id": W },
        { "id": id(4), "type": "board", "name": "Ретро: планы и итоги", "parent_id": W },
        { "id": id(5), "type": "board", "name": "Ёлка", "parent_id": W, "in_trash": true },
        { "id": id(6), "type": "board", "name": "итоги квартала", "parent_id": id(1) },
    ]))
}

#[test]
fn trash_reply_is_searchable_as_trash() {
    let reply = json!({ "channels": [
        { "id": id(1), "type": "board", "name": "Old plans", "in_trash": "2026-10-21" },
        { "id": id(2), "type": "folder", "name": "Archive", "in_trash": "2026-10-21",
          "children": [{ "id": id(3), "type": "board", "name": "Old notes", "in_trash": null }] },
    ]});
    let index = NameIndex::of_trash(&reply);
    let trash = NodeFilter {
        kind: None,
        in_trash: true,
    };
    let names = |query, filter| {
        let (hits, _) = index.search(query, filter, Limit::from(10));
        hits.iter().map(|h| index.name(h.node)).collect::<Vec<_>>()
    };
    assert_eq!(names("old", trash), ["Old plans", "Old notes"]);
    assert_eq!(names("old", ALL), Vec::<&str>::new());
    assert_eq!(names("archive", trash), ["Archive"]);
}

#[rstest]
#[case::case_yo_and_punctuation("  Ёжик:  План-Б!! ", "ежик план б")]
#[case::other_scripts("Straße ΔΕΛΤΑ 東京", "straße δελτα 東京")]
#[case::dotted_capital_i("İstanbul", "istanbul")]
#[case::only_punctuation(" -- ", "")]
#[case::soft_hyphen_inside_a_word("Plan\u{ad}s", "plans")]
#[case::zero_width_space_inside_a_word("Pl\u{200b}ans\u{feff}", "plans")]
#[case::bidi_marks(
    "\u{200f}\u{5e9}\u{5dc}\u{5d5}\u{5dd}\u{200e} Q4",
    "\u{5e9}\u{5dc}\u{5d5}\u{5dd} q4"
)]
#[case::decomposed_short_i(
    "\u{41c}\u{43e}\u{438}\u{306} \u{43f}\u{43b}\u{430}\u{43d}",
    "мой план"
)]
#[case::decomposed_yo("\u{415}\u{308}\u{43b}\u{43a}\u{430}", "елка")]
#[case::decomposed_latin("Cafe\u{301}", "caf\u{e9}")]
#[case::stray_mark_is_not_a_letter(" \u{301}x", "x")]
#[case::stray_mark_keeps_words_apart("a \u{301} b", "a b")]
fn normalize(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(normalized(raw), expected);
}

#[test]
fn inert_chars_need_no_nfc() {
    use icu_normalizer::properties::{
        CanonicalCombiningClassMap,
        CanonicalDecomposition,
    };
    let decomposition = CanonicalDecomposition::new();
    let seconds: std::collections::HashSet<char> = (char::MIN..=char::MAX)
        .filter_map(|ch| {
            // The decomposition shows as `Expansion('first', 'second')`.
            format!("{:?}", decomposition.decompose(ch))
                .split_once(", ")
                .and_then(|(_, second)| second.strip_suffix(')'))
                .and_then(|quoted| quoted.strip_prefix('\''))
                .and_then(|quoted| quoted.strip_suffix('\''))
                .and_then(|inner| {
                    inner
                        .strip_prefix("\\u{")
                        .and_then(|hex| hex.strip_suffix('}'))
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .and_then(char::from_u32)
                        .or_else(|| inner.chars().next())
                })
        })
        .collect();
    let classes = CanonicalCombiningClassMap::new();
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    let safe = |ch: char| {
        classes.get_u8(ch) == 0
            && !seconds.contains(&ch)
            && nfc.is_normalized(ch.encode_utf8(&mut [0; 4]))
    };
    let broken: Vec<char> = (char::MIN..=char::MAX)
        .filter(|&ch| is_inert(ch))
        .filter(|&ch| {
            !safe(ch)
                || ch
                    .to_lowercase()
                    .filter(|lower| lower.is_alphanumeric() && !is_invisible(*lower))
                    .any(|lower| !safe(lower))
        })
        .collect();
    assert_eq!(broken, Vec::<char>::new());
}

#[rstest]
#[case::exact_prefix_word_substring(
        "планы",
        ALL,
        10,
        (&["Планы", "Планы Q4", "Ретро: планы и итоги"][..], 3)
    )]
#[case::limit_keeps_best("планы", ALL, 1, (&["Планы"][..], 3))]
#[case::zero_limit_means_all(
        "планы",
        ALL,
        0,
        (&["Планы", "Планы Q4", "Ретро: планы и итоги"][..], 3)
    )]
#[case::words_in_any_order("квартала итоги", ALL, 10, (&["итоги квартала"][..], 1))]
#[case::folders_only(
        "страт",
        NodeFilter { kind: Some(NodeKind::Folder), in_trash: false },
        10,
        (&["Стратегия"][..], 1)
    )]
#[case::trash_is_hidden("елка", ALL, 10, (&[][..], 0))]
#[case::trash_on_request("елка", NodeFilter { kind: None, in_trash: true }, 10, (&["Ёлка"][..], 1))]
#[case::empty_query("", ALL, 10, (&[][..], 0))]
fn search_sample(
    #[case] query: &str,
    #[case] filter: NodeFilter,
    #[case] limit: usize,
    #[case] expected: (&[&str], usize),
) -> TestResult {
    let index = sample()?;
    let (hits, total) = index.search(query, filter, Limit::from(limit));
    let names: Vec<_> = hits.iter().map(|h| index.name(h.node)).collect();
    assert_eq!((names.as_slice(), total), expected);
    Ok(())
}

// Scores: `EXACT`, `MatchStart::score`, +`WHOLE_WORD`, `ALL_WORDS`, +1 a board.
#[rstest]
#[case::overlapping_matches(&["х1 1 1"], "1 1", &[("х1 1 1", 651)])]
#[case::names_without_letters_keep_offsets(&["🚀", "", "Plan"], "plan", &[("Plan", 1001)])]
#[case::never_spans_two_names(&["аб", "вг"], "бв", &[])]
#[case::second_name_whole(&["аб", "вг"], "вг", &[("вг", 1001)])]
#[case::punctuation_prefers_whole_words(
        &["Доска 1-15 планирование", "Доска 1-150 планирование", "Доска 0-1 итоги 15"],
        "1-15",
        &[
            ("Доска 1-15 планирование", 651),
            ("Доска 1-150 планирование", 601),
            ("Доска 0-1 итоги 15", 201),
        ]
    )]
fn scores(
    #[case] names: &[&str],
    #[case] query: &str,
    #[case] expected: &[(&str, u32)],
) -> TestResult {
    let index = boards(names)?;
    let (hits, total) = index.search(query, ALL, Limit::from(10));
    let found: Vec<_> = hits.iter().map(|h| (index.name(h.node), h.score)).collect();
    assert_eq!((found.as_slice(), total), (expected, expected.len()));
    Ok(())
}

#[test]
fn non_uuid_ids_are_kept() -> TestResult {
    let index = index(&json!([
        { "id": "f", "type": "folder", "name": "Folder" },
        { "id": "b1", "type": "board", "name": "Board", "parent_id": "f" },
    ]))?;
    assert_eq!(index.id(NodeIndex(1)), "b1");
    assert_eq!(index.find_id("b1"), Some(NodeIndex(1)));
    assert_eq!(index.folder_path(NodeIndex(1)), "Folder");
    Ok(())
}

#[test]
fn folder_path_has_three_levels_and_survives_cycles() -> TestResult {
    let index = index(&json!([
        { "id": id(1), "type": "folder", "name": "A" },
        { "id": id(2), "type": "folder", "name": "B", "parent_id": id(1) },
        { "id": id(3), "type": "folder", "name": "C", "parent_id": id(2) },
        { "id": id(4), "type": "board", "name": "D", "parent_id": id(3) },
        { "id": id(5), "type": "folder", "name": "X", "parent_id": id(6) },
        { "id": id(6), "type": "folder", "name": "Y", "parent_id": id(5) },
    ]))?;
    assert_eq!(index.folder_path(NodeIndex(3)), "A/B/C");
    assert_eq!(index.folder_path(NodeIndex(0)), "");
    // The X↔Y cycle is cut at 32 levels.
    let path = index.folder_path(NodeIndex(4));
    assert_eq!(path.split('/').count(), 32);
    assert!(path.ends_with('Y'));
    Ok(())
}

#[test]
fn nested_children_are_not_lost() -> TestResult {
    let index = index(&json!([
        { "id": id(1), "type": "folder", "name": "Folder", "children": [
            { "id": id(2), "type": "board", "name": "Nested" },
        ]},
    ]))?;
    assert_eq!(index.len(), 2);
    assert_eq!(index.folder_path(NodeIndex(1)), "Folder");
    Ok(())
}

#[test]
fn non_canonical_and_nil_uuids_are_kept_verbatim() -> TestResult {
    let upper = "AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B";
    let nil = "00000000-0000-0000-0000-000000000000";
    let index = index(&json!([
        { "id": "x", "type": "board", "name": "Not a UUID" },
        { "id": upper, "type": "board", "name": "Upper case" },
        { "id": nil, "type": "board", "name": "Nil" },
    ]))?;
    assert_eq!(index.id(NodeIndex(1)), upper, "the id comes back as sent");
    assert_eq!(index.find_id(upper), Some(NodeIndex(1)));
    assert_eq!(
        index.find_id(nil),
        Some(NodeIndex(2)),
        "nil is not the marker"
    );
    Ok(())
}

#[test]
fn order_index_is_kept() -> TestResult {
    let index = index(&json!([
        { "id": id(1), "type": "board", "name": "B", "order_index": 2.5 },
        { "id": id(2), "type": "board", "name": "A" },
    ]))?;
    assert!(index.order(NodeIndex(0)).total_cmp(&2.5).is_eq());
    assert!(index.order(NodeIndex(1)).is_infinite());
    Ok(())
}

#[test]
fn ids_parents_and_paths_survive_compaction() -> TestResult {
    let index = sample()?;
    let q4 = index
        .find_id(&id(2))
        .map(|q4| (index.id(q4), index.kind(q4), index.folder_path(q4)));
    assert_eq!(q4, Some((id(2), NodeKind::Board, "Стратегия".to_owned())));
    let top = index.find_id(&id(3)).map(|top| index.parent(top));
    assert_eq!(top, Some(None));
    Ok(())
}

#[test]
fn numbers_from_another_index_are_harmless() -> TestResult {
    let big = sample()?;
    let small = boards(&["one"])?;
    let foreign = big.nodes().last().unwrap_or(NodeIndex(0));
    assert_eq!(
        (small.id(foreign), small.name(foreign), small.kind(foreign)),
        (String::new(), "", NodeKind::Other)
    );
    assert_eq!(small.parent(foreign), None);
    assert!(!small.in_trash(foreign));
    Ok(())
}

#[test]
fn trash_marker_null_is_not_trash() -> TestResult {
    let index = index(&json!([
        { "id": id(1), "type": "board", "name": "A", "in_trash": null },
        { "id": id(2), "type": "board", "name": "B", "in_trash": false },
        { "id": id(3), "type": "board", "name": "C", "in_trash": "2026-10-21" },
    ]))?;
    let trash: Vec<_> = index.nodes().map(|n| index.in_trash(n)).collect();
    assert_eq!(trash, [false, false, true]);
    Ok(())
}

#[test]
fn same_result_from_nodes_and_from_json() -> TestResult {
    let text = json!({ "channels": [
        { "id": id(1), "type": "board", "name": "A\"b\\c", "parent_id": W },
    ]})
    .to_string();
    let a = NameIndex::from_tree_json(&text)?;
    let b = NameIndex::from_nodes(&tree::parse(&text)?.nodes);
    assert_eq!(a.name(NodeIndex(0)), "A\"b\\c");
    assert_eq!(a.name(NodeIndex(0)), b.name(NodeIndex(0)));
    assert_eq!(a.id(NodeIndex(0)), b.id(NodeIndex(0)));
    Ok(())
}
