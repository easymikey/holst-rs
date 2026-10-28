use std::borrow::Cow;

use proptest::{
    collection::vec,
    option,
    prelude::{
        Just,
        Strategy,
        any,
        prop_assert,
        prop_assert_eq,
        prop_assume,
        prop_oneof,
        proptest,
    },
};

use crate::{
    domain::{
        id,
        id::{BoardId, FolderId, ObjectId, WorkspaceId},
        text::joined,
        url::{ApiPath, BaseUrl, DEFAULT_BASE, encode_segment},
    },
    laws::{config, found, path_text, uuid},
};

const fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn decode(encoded: &str) -> Option<Vec<u8>> {
    let (head, rest) = encoded
        .split_once('%')
        .map_or((encoded, None), |(head, rest)| (head, Some(rest)));
    rest.into_iter().flat_map(|rest| rest.split('%')).try_fold(
        head.as_bytes().to_vec(),
        |mut out, part| {
            let hex = part.get(..2).filter(|hex| {
                hex.bytes()
                    .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
            })?;
            let byte = u8::from_str_radix(hex, 16)
                .ok()
                .filter(|&b| !unreserved(b))?;
            out.push(byte);
            out.extend_from_slice(part.get(2..)?.as_bytes());
            Some(out)
        },
    )
}

fn base() -> Option<BaseUrl> {
    DEFAULT_BASE.parse().ok()
}

/// A board id with a link to it.
#[derive(Debug)]
struct BoardLink {
    id: String,
    url: String,
}

fn board_link() -> impl Strategy<Value = BoardLink> {
    (
        uuid(),
        "[a-z]{1,10}(\\.[a-z]{2,5})?",
        vec(
            "[a-z0-9]{1,8}".prop_filter("not board", |name| name != "board"),
            0..3,
        ),
        option::of("/[a-z0-9/-]{0,12}"),
        option::of(".*"),
        option::of(".*"),
    )
        .prop_map(|(id, host, prefix, tail, query, fragment)| {
            let url = format!(
                "https://{host}/{}board/{id}{}{}{}",
                joined(prefix.iter().map(|segment| format!("{segment}/")), ""),
                tail.unwrap_or_default(),
                query.map(|q| format!("?{q}")).unwrap_or_default(),
                fragment.map(|f| format!("#{f}")).unwrap_or_default(),
            );
            BoardLink { id, url }
        })
}

proptest! {
    #![proptest_config(config())]

    /// Only unreserved bytes and `%XX`, decodes back.
    #[test]
    fn segment_is_safe_and_reversible(raw in path_text()) {
        prop_assume!(!matches!(raw.as_str(), "" | "." | ".."));
        let encoded = encode_segment(&raw)?;
        prop_assert!(encoded.bytes().all(|b| unreserved(b) || b == b'%'));
        prop_assert_eq!(decode(&encoded), Some(raw.as_bytes().to_vec()));
        prop_assert_eq!(
            matches!(encoded, Cow::Borrowed(_)),
            raw.bytes().all(unreserved)
        );
    }

    /// `.`, `..` and "" are refused.
    #[test]
    fn dot_segments_and_empty_are_refused(raw in prop_oneof![Just(String::new()), Just(".".to_owned()), Just("..".to_owned())]) {
        prop_assert!(encode_segment(&raw).is_err());
    }

    /// Found in lowercase in any `…/board/{id}` link with path, query, fragment.
    #[test]
    fn board_id_is_found_in_any_board_link(
        board in board_link(),
    ) {
        let lower = board.id.to_ascii_lowercase();
        prop_assert_eq!(found(&board.url), Some(lower.clone()));
        prop_assert_eq!(found(&format!("  {}\n", board.id)), Some(lower));
    }

    /// A UUID outside the `board` segment or inside a name is not taken.
    #[test]
    fn board_id_is_not_taken_from_other_places(
        id in uuid(),
        name in "[a-zA-Zа-яА-Я][a-zA-Zа-яА-Я ]{0,10}",
    ) {
        prop_assert_eq!(found(&format!("https://h/workspace/{id}/boards")), None);
        prop_assert_eq!(found(&format!("{name} {id}")), None);
        prop_assert_eq!(found(&format!("{id}{name}")), None);
    }

    /// `is_uuid` agrees with the `uuid` crate.
    #[test]
    fn is_uuid_agrees_with_uuid_crate(
        id in uuid(),
        noise in "[0-9a-fA-F-]{36}",
    ) {
        prop_assert!(id::is_uuid(&id));
        prop_assert!(id.parse::<BoardId>().is_ok());
        prop_assert_eq!(id::is_uuid(&noise), uuid::Uuid::try_parse(&noise).is_ok());
    }

    /// `ApiPath` accepts exactly the paths from the root that are not URLs.
    #[test]
    fn api_path_accepts_exactly_rooted_paths(
        path in prop_oneof![
            path_text(),
            "/[a-z/_]{0,12}",
            "//[a-z.]{1,8}/x",
            "https://[a-z]{1,6}/[a-z]{0,4}",
            "/[a-z_]{1,8}\\?url=https://[a-z]{1,6}",
        ],
    ) {
        // A `://` after the first `/` is inside the path or the query.
        let rooted = path.starts_with('/') && !path.starts_with("//");
        let parsed = path.parse::<ApiPath>();
        prop_assert_eq!(parsed.is_ok(), rooted);
        prop_assert!(parsed.iter().all(|item| item.as_str() == path));
    }

    /// A request URL is always on the base's origin.
    #[test]
    fn request_urls_stay_on_the_base_origin(
        path in prop_oneof![
            "/[a-z/_]{0,12}",
            "[a-z/_]{0,12}",
            "https://[a-z]{1,6}(\\.[a-z]{2,3})?(:[0-9]{2,4})?/[a-z]{0,4}",
            "https://app\\.holst\\.so/[a-z]{0,6}",
            "http://app\\.holst\\.so/[a-z]{0,6}",
        ],
    ) {
        let base = base();
        let url = base.as_ref().map(|base| base.url_for(&path));
        prop_assert!(url.is_some());
        let own = base.as_ref().map(|base| base.url_for(DEFAULT_BASE).map(|u| u.origin()));
        let same = url.and_then(Result::ok).map(|u| u.origin());
        prop_assert!(same.is_none() || same == own.and_then(Result::ok));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Previous {
    Hex,
    Other,
}

impl Previous {
    fn of(ch: char) -> Self {
        [Self::Other, Self::Hex]
            .get(usize::from(ch.is_ascii_hexdigit()))
            .copied()
            .unwrap_or(Self::Other)
    }
}

type Piece = (Option<(usize, usize)>, usize, Previous);

pub(crate) fn spans_char_by_char(text: &str) -> Vec<(usize, usize)> {
    /// The piece at `at`, after a hex digit or not.
    fn piece(text: &str, at: usize, previous: Previous) -> Option<Piece> {
        let rest = text.get(at..)?;
        let is_hex = |c: char| c.is_ascii_hexdigit();
        let uuid = (previous == Previous::Other)
            .then_some(rest)
            .and_then(|rest| rest.split_at_checked(id::UUID_LEN))
            .filter(|&(found, tail)| id::is_uuid(found) && !tail.starts_with(is_hex));
        uuid.map_or_else(
            || {
                rest.chars()
                    .next()
                    .map(|ch| (None, at + ch.len_utf8(), Previous::of(ch)))
            },
            |_| {
                let end = at + id::UUID_LEN;
                Some((Some((at, end)), end, Previous::Hex))
            },
        )
    }
    std::iter::successors(piece(text, 0, Previous::Other), |&(_, next, previous)| {
        piece(text, next, previous)
    })
    .filter_map(|(span, ..)| span)
    .collect()
}

fn uuid_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        uuid(),
        "[0-9a-fA-F-]{0,12}",
        "[ а-яё:,()😀]{0,3}",
        any::<String>(),
    ];
    vec(piece, 0..8).prop_map(|pieces| pieces.concat())
}

proptest! {
    #![proptest_config(config())]

    /// The hyphen-to-hyphen scan finds exactly what the char-by-char scan finds.
    #[test]
    fn uuid_spans_match_the_char_by_char_scan(text in uuid_text()) {
        let spans: Vec<_> = id::uuid_spans(&text).collect();
        prop_assert_eq!(&spans, &spans_char_by_char(&text));
        let whole = spans
            .iter()
            .all(|&(start, end)| text.get(start..end).is_some_and(id::is_uuid));
        prop_assert!(whole);
    }

    /// A UUID between non-hex characters is always found, exactly there.
    #[test]
    fn a_separated_uuid_is_found(
        value in uuid(),
        before in "[ g-zа-я:(]{0,4}",
        after in "[ g-zа-я:)]{0,4}",
    ) {
        let text = format!("{before}{value}{after}");
        let spans: Vec<_> = id::uuid_spans(&text).collect();
        prop_assert_eq!(spans, vec![(before.len(), before.len() + id::UUID_LEN)]);
    }
}

proptest! {
    #![proptest_config(config())]

    /// A UUID in either case parses to its lowercase, and the JSON string
    /// brings back the same id, for every UUID-named kind.
    #[test]
    fn uuid_ids_round_trip_through_json(value in uuid()) {
        let lower = value.to_ascii_lowercase();
        let board = value.parse::<BoardId>().ok();
        let workspace = value.parse::<WorkspaceId>().ok();
        let folder = value.parse::<FolderId>().ok();
        prop_assert_eq!(board.as_ref().map(BoardId::as_str), Some(lower.as_str()));
        prop_assert_eq!(workspace.as_ref().map(WorkspaceId::as_str), Some(lower.as_str()));
        prop_assert_eq!(folder.as_ref().map(FolderId::as_str), Some(lower.as_str()));
        let json = format!("\"{lower}\"");
        prop_assert_eq!(board.and_then(|id| serde_json::to_string(&id).ok()), Some(json.clone()));
        prop_assert_eq!(workspace.and_then(|id| serde_json::to_string(&id).ok()), Some(json.clone()));
        prop_assert_eq!(folder.and_then(|id| serde_json::to_string(&id).ok()), Some(json.clone()));
        prop_assert_eq!(
            serde_json::from_str::<BoardId>(&json).ok().map(String::from),
            Some(lower.clone())
        );
        prop_assert_eq!(
            serde_json::from_str::<WorkspaceId>(&json).ok().map(String::from),
            Some(lower.clone())
        );
        prop_assert_eq!(
            serde_json::from_str::<FolderId>(&json).ok().map(String::from),
            Some(lower)
        );
    }

    /// An object id is kept as given, and its JSON string brings it back.
    #[test]
    fn object_id_round_trips_through_json(raw in "[^\\s\\p{Cc}]{1,40}") {
        let id = raw.parse::<ObjectId>().ok();
        prop_assert_eq!(id.as_ref().map(ObjectId::as_str), Some(raw.as_str()));
        let json = id.and_then(|id| serde_json::to_string(&id).ok());
        let back = json.and_then(|json| serde_json::from_str::<ObjectId>(&json).ok());
        prop_assert_eq!(back.map(String::from), Some(raw));
    }

    /// Text that is not a UUID is no workspace, folder or board id.
    #[test]
    fn non_uuid_text_is_no_uuid_id(raw in any::<String>()) {
        prop_assume!(!id::is_uuid(&raw));
        prop_assert!(raw.parse::<WorkspaceId>().is_err());
        prop_assert!(raw.parse::<FolderId>().is_err());
        prop_assert!(raw.parse::<BoardId>().is_err());
    }
}
