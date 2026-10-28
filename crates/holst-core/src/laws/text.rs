use std::ops::ControlFlow;

use proptest::{
    collection::vec,
    option,
    prelude::{Strategy, any, prop_assert, prop_assert_eq, prop_oneof, proptest},
    sample::select,
};
use serde_json::{Value, json};

use crate::{
    domain::{
        text::joined,
        time::{CivilDate, Timestamp, civil_from_days},
        tree,
        tree::name_index::build::normalize_into,
    },
    laws::{LAST_MS, board_name, config},
};

fn naive_civil(days: u64) -> CivilDate {
    let leap = |year: u64| {
        (year.is_multiple_of(4) && !year.is_multiple_of(100))
            || year.is_multiple_of(400)
    };
    let year_len = |year: u64| 365 + u64::from(leap(year));
    let (year, rest) = std::iter::successors(Some((1970, days)), |&(year, rest)| {
        rest.checked_sub(year_len(year))
            .map(|rest| (year + 1, rest))
    })
    .last()
    .unwrap_or((1970, days));
    let months = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let found = months
        .iter()
        .zip(1..)
        .try_fold(rest, |rest, (&len, month)| {
            rest.checked_sub(len + u64::from(month == 2 && leap(year)))
                .map_or(ControlFlow::Break((month, rest + 1)), ControlFlow::Continue)
        });
    let (month, day) = found
        .break_value()
        .or_else(|| found.continue_value().map(|left| (12, left + 1)))
        .unwrap_or_default();
    CivilDate { year, month, day }
}

proptest! {
    #![proptest_config(config())]

    /// The time strings compare like the numbers.
    #[test]
    fn iso8601_preserves_order(a in 0..=LAST_MS, b in 0..=LAST_MS) {
        let (a_stamp, b_stamp) = (Timestamp::from_millis(a), Timestamp::from_millis(b));
        prop_assert_eq!(a.cmp(&b), a_stamp.iso8601().cmp(&b_stamp.iso8601()));
        prop_assert_eq!(a_stamp.iso8601().len(), 24);
    }

    /// Hinnant's algorithm agrees with the naive calendar up to year 9999.
    #[test]
    fn civil_from_days_matches_naive_calendar(days in 0..2_932_897_u64) {
        prop_assert_eq!(civil_from_days(days), naive_civil(days));
    }

    #[test]
    fn joined_is_join(items in vec(".{0,4}", 0..6), sep in ".{0,3}") {
        prop_assert_eq!(joined(&items, &sep).to_string(), items.join(&sep));
    }
}

fn odd_lowercase_words() -> impl Strategy<Value = String> {
    let odd: Vec<char> = (0..=u32::from(char::MAX))
        .filter_map(char::from_u32)
        .filter(|c| {
            c.is_alphanumeric()
                && c.to_lowercase().any(|lower| !lower.is_alphanumeric())
        })
        .collect();
    vec(
        prop_oneof![select(odd), select(&['a', 'я', ' ', '.'][..])],
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn appended(mut out: String, raw: &str) -> String {
    normalize_into(raw, &mut out);
    out
}

fn normalize(raw: &str) -> String {
    appended(String::new(), raw)
}

proptest! {
    #![proptest_config(config())]

    /// Idempotent; words separated by one space, without `ё`.
    #[test]
    fn normalize_is_idempotent_and_tidy(
        raw in prop_oneof![any::<String>(), board_name(), odd_lowercase_words()],
    ) {
        let once = normalize(&raw);
        prop_assert!(!once.starts_with(' ') && !once.ends_with(' '));
        prop_assert!(!once.contains("  ") && !once.contains('ё'));
        prop_assert_eq!(normalize(&once), once);
    }

    /// A name typed decomposed (NFD, as macOS file names) is the same name.
    #[test]
    fn normalize_ignores_decomposition(
        raw in prop_oneof![any::<String>(), board_name(), "[a-zа-яё йЙЁéÉçñ]{0,12}"],
    ) {
        let decomposed = icu_normalizer::DecomposingNormalizerBorrowed::new_nfd().normalize(&raw);
        prop_assert_eq!(normalize(&decomposed), normalize(&raw));
    }

    /// Zero-width spaces and soft hyphens between letters change nothing.
    #[test]
    fn normalize_ignores_invisible_characters(
        raw in board_name(),
        invisible in select(&['\u{200b}', '\u{ad}', '\u{feff}', '\u{200d}', '\u{2060}'][..]),
    ) {
        let spread: String = raw
            .chars()
            .flat_map(|ch| [ch, invisible])
            .collect();
        prop_assert_eq!(normalize(&spread), normalize(&raw));
    }

    /// Appends to the buffer without touching what is there.
    #[test]
    fn normalize_appends(prefix in ".{0,8}", raw in board_name()) {
        let out = appended(prefix.clone(), &raw);
        prop_assert_eq!(out, format!("{prefix}{}", normalize(&raw)));
    }
}

#[derive(Debug, Clone)]
struct Node {
    id: Option<String>,
    key: &'static str,
    children: Vec<Node>,
}

const NESTING: &[&str] = &["children", "channels", "folders", "items", "tree"];

fn node() -> impl Strategy<Value = Node> {
    let leaf =
        (option::of("[a-z0-9]{1,4}"), select(NESTING)).prop_map(|(id, key)| Node {
            id,
            key,
            children: Vec::new(),
        });
    leaf.prop_recursive(4, 32, 4, |inner| {
        (
            option::of("[a-z0-9]{1,4}"),
            select(NESTING),
            vec(inner, 0..4),
        )
            .prop_map(|(id, key, children)| Node { id, key, children })
    })
}

fn node_json(node: &Node) -> Value {
    let children: Vec<_> = node.children.iter().map(node_json).collect();
    let fields = [
        ("type", json!("board")),
        ("name", json!("x")),
        (node.key, Value::Array(children)),
    ];
    let id = node.id.iter().map(|id| ("id", json!(id)));
    Value::Object(
        fields
            .into_iter()
            .chain(id)
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn expected_nodes(
    nodes: &[Node],
    parent: Option<&str>,
) -> Vec<(String, Option<String>)> {
    nodes
        .iter()
        .flat_map(|node| {
            let own = node
                .id
                .as_deref()
                .map(|id| (id.to_owned(), parent.map(str::to_owned)));
            let next = node.id.as_deref().or(parent);
            own.into_iter().chain(expected_nodes(&node.children, next))
        })
        .collect()
}

proptest! {
    #![proptest_config(config())]

    /// No node with an id is lost; the parent is the nearest ancestor with an id.
    #[test]
    fn flatten_keeps_every_node_and_its_nearest_parent(roots in vec(node(), 0..4)) {
        let value = Value::Array(roots.iter().map(node_json).collect());
        let got: Vec<_> = tree::flatten(&value)
            .into_iter()
            .map(|node| (node.id, node.parent_id))
            .collect();
        prop_assert_eq!(got, expected_nodes(&roots, None));
    }

    /// A spec reply parses verbatim, without the heuristic.
    #[test]
    fn documented_tree_parses_verbatim(
        channels in vec(("[a-z0-9]{1,6}", select(&["board", "folder"][..]), board_name(), option::of("[a-z0-9]{1,6}"), any::<bool>()), 0..12),
    ) {
        let text = json!({ "channels": channels.iter().map(|(id, kind, name, parent, trash)| json!({
            "id": id, "type": kind, "name": name, "parent_id": parent, "in_trash": trash,
        })).collect::<Vec<_>>() }).to_string();
        let parsed = tree::parse(&text)?;
        prop_assert!(parsed.fallback.is_none());
        let got: Vec<_> = parsed.nodes.into_iter()
            .map(|node| (node.id, node.kind.as_str().to_owned(), node.name, node.parent_id, node.in_trash))
            .collect();
        let expected: Vec<_> = channels.into_iter()
            .map(|(id, kind, name, parent, trash)| (id, kind.to_owned(), name, parent, trash))
            .collect();
        prop_assert_eq!(got, expected);
    }
}

#[test]
fn normalize_is_idempotent_on_multi_char_lowercase() {
    let tricky = ["İ", "İstanbul", "ǅ", "ẞ", "Ⅷ", "ΣΑΣ", "ﬁ"];
    let unstable: Vec<_> = tricky
        .iter()
        .filter(|raw| normalize(&normalize(raw)) != normalize(raw))
        .collect();
    assert!(unstable.is_empty(), "{unstable:?}");
}
