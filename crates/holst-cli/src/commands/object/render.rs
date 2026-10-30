//! What the object commands print.

use holst_core::{
    command::{
        board::picture::Delivery,
        object::{get::Downloaded, list::Listing},
    },
    domain::{
        id::short_id,
        text::{counted, size_text},
    },
};

const TEXT_MAX_CHARS: usize = 60;
pub(super) fn listing_text(listing: &Listing) -> String {
    let rectangle = listing.rectangle.map(|rect| format!("rectangle {rect}"));
    let head = format!(
        "{} · {}",
        counted(listing.matched, "object"),
        listing.edit_right.as_str()
    );
    let lines = listing.objects.iter().map(|listed| {
        let name = |id: &str| format!("@{}", short_id(id));
        format!("{}  {}", listed.line(name, TEXT_MAX_CHARS), listed.link)
    });
    let more = listing.next.map(|next| {
        format!(
            "… {}–{next} of {}; next: --offset {next}",
            listing.from.saturating_add(1),
            listing.matched
        )
    });
    rectangle
        .into_iter()
        .chain([head])
        .chain(lines)
        .chain(more)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns a line per original: where it went and how big it is, then a line
/// per original not saved.
pub(super) fn downloaded_text(downloaded: &Downloaded) -> String {
    downloaded
        .objects
        .iter()
        .map(|object| {
            let (place, bytes) = match &object.delivery {
                Delivery::Written { file, bytes } => {
                    (file.display().to_string(), *bytes)
                }
                Delivery::Inline { bytes } => (String::from("stdout"), bytes.len()),
            };
            format!(
                "downloaded @{} to {place} · {}  {}",
                short_id(&object.object),
                size_text(u64::try_from(bytes).unwrap_or(u64::MAX)),
                object.link
            )
        })
        .chain(
            downloaded
                .not_saved
                .iter()
                .map(|line| format!("not saved: {line}")),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use holst_core::{
        command::{
            board::picture::Delivery,
            object::{
                get::{Downloaded, DownloadedObject},
                list::{ListedObject, Listing, TextStyle},
            },
        },
        domain::board::{ArrowEnds, EditRight, Object, ObjectKind},
    };
    use serde_json::Value;

    use crate::commands::object::render::{
        TEXT_MAX_CHARS,
        downloaded_text,
        listing_text,
    };

    fn sticker(id: &str, text: &str) -> Object {
        Object {
            id: id.into(),
            kind: ObjectKind::Sticker,
            parent: None,
            x: 100.0,
            y: 50.4,
            width: Some(192.0),
            height: Some(192.0),
            z_index: 0.0,
            text: text.into(),
            color: Some("yellow4".into()),
            look: None,
            media: None,
            ends: None,
            author: None,
            locked: false,
            rotation: 0.0,
            link_to: None,
            file: None,
            fields: Value::Null,
        }
    }

    fn listed(object: Object) -> ListedObject {
        ListedObject {
            link: format!("L/{}", object.id),
            object,
            drawn: None,
            file: None,
            card: None,
            tags: Vec::new(),
            reactions: std::collections::BTreeMap::new(),
            style: TextStyle::default(),
            detail: Vec::new(),
        }
    }

    fn listing(objects: Vec<ListedObject>, next: Option<usize>) -> Listing {
        Listing {
            board: "b1".into(),
            link: "L".into(),
            edit_right: EditRight::Editable,
            rectangle: None,
            matched: 5,
            from: 0,
            objects,
            next,
        }
    }

    #[test]
    fn the_json_names_the_board_and_each_object_by_id() {
        let json =
            serde_json::to_value(listing(vec![listed(sticker("a1", ""))], None)).ok();
        let at = |path| json.as_ref().and_then(|json| json.pointer(path));
        assert_eq!(at("/board"), Some(&Value::from("b1")));
        assert_eq!(at("/id"), None);
        assert_eq!(at("/objects/0/id"), Some(&Value::from("a1")));
    }

    #[test]
    fn one_matched_object_is_not_plural() {
        let one = Listing {
            matched: 1,
            ..listing(Vec::new(), None)
        };
        assert!(listing_text(&one).starts_with("1 object · editable"));
        let none = Listing {
            matched: 0,
            ..listing(Vec::new(), None)
        };
        assert!(listing_text(&none).starts_with("0 objects · editable"));
    }

    #[test]
    fn an_object_shows_name_kind_place_size_colour_text_and_link() {
        let long = format!("first\n{}", "x".repeat(TEXT_MAX_CHARS));
        let shown = listing_text(&listing(
            vec![listed(sticker("a1b2c3d4e5f6", &long))],
            Some(1),
        ));
        let text = format!("first / {}…", "x".repeat(TEXT_MAX_CHARS - 8));
        assert_eq!(
            shown,
            format!(
                "5 objects · editable\n\
                 @a1b2c3d4 sticker 100,50 192×192  yellow4  \"{text}\"  L/a1b2c3d4e5f6\n\
                 … 1–1 of 5; next: --offset 1"
            )
        );
    }

    #[test]
    fn an_arrow_shows_its_ends_and_a_child_its_frame() {
        let arrow = Object {
            kind: ObjectKind::Arrow,
            width: None,
            height: None,
            color: None,
            ends: Some(ArrowEnds {
                start: Some("s1111111222".into()),
                end: None,
                drawn: None,
            }),
            ..sticker("a1", "")
        };
        let child = Object {
            parent: Some("f1111111222".into()),
            color: None,
            ..sticker("s2", "hi")
        };
        let lines: Vec<String> =
            listing_text(&listing(vec![listed(arrow), listed(child)], None))
                .lines()
                .map(str::to_owned)
                .collect();
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some("@a1 arrow 100,50  @s1111111 → ·  L/a1")
        );
        assert_eq!(
            lines.get(2).map(String::as_str),
            Some("@s2 sticker 100,50 192×192 in @f1111111  \"hi\"  L/s2")
        );
    }

    #[test]
    fn a_download_says_where_the_original_went() {
        let downloaded = Downloaded {
            board: "b1".into(),
            objects: vec![DownloadedObject {
                object: "a1b2c3d4e5f6".into(),
                link: "L/a1".into(),
                delivery: Delivery::Written {
                    file: PathBuf::from("/d/cat.png"),
                    bytes: 2048,
                },
            }],
            not_saved: vec!["a2: gone".to_owned()],
        };
        assert_eq!(
            downloaded_text(&downloaded),
            "downloaded @a1b2c3d4 to /d/cat.png · 2.0 KiB  L/a1\nnot saved: a2: gone"
        );
    }
}
