//! What the object commands print.

use holst_core::{
    command::{
        board::picture::Delivery,
        object::{
            get::Downloaded,
            list::{ListedObject, Listing},
        },
    },
    domain::text::{counted, one_line},
};

use crate::output::view::size_text;

const TEXT_MAX_CHARS: usize = 60;
/// How many characters of an id name an object.
const SHORT_ID: usize = 8;

/// Returns the first characters of an id, as `@` names an object.
fn short(id: &str) -> &str {
    id.get(..SHORT_ID).unwrap_or(id)
}

pub(super) fn listing_text(listing: &Listing) -> String {
    let rectangle = listing.rectangle.map(|rect| format!("rectangle {rect}"));
    let head = format!(
        "{} · {}",
        counted(listing.matched, "object"),
        listing.edit_right.as_str()
    );
    let lines = listing.objects.iter().map(object_line);
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

fn object_line(listed: &ListedObject) -> String {
    let object = &listed.object;
    let corner = format!("({:.0}, {:.0})", object.x, object.y);
    let size = object
        .width
        .zip(object.height)
        .map(|(width, height)| format!(" {width:.0}×{height:.0}"))
        .unwrap_or_default();
    let inside = object
        .parent
        .as_deref()
        .map(|parent| format!(" in @{}", short(parent)))
        .unwrap_or_default();
    let color = object
        .color
        .as_deref()
        .map(|color| format!("  {color}"))
        .unwrap_or_default();
    let look = object
        .look
        .as_ref()
        .map(|look| format!("  {}", look.words()))
        .unwrap_or_default();
    let style = listed.style.line();
    let ends = object
        .ends
        .as_ref()
        .map(|ends| {
            let end =
                |end: &Option<String>| end.as_deref().map_or("·", short).to_owned();
            format!("  {} → {}", end(&ends.start), end(&ends.end))
        })
        .unwrap_or_default();
    let tags = if listed.tags.is_empty() {
        String::new()
    } else {
        format!("  #{}", listed.tags.join(" #"))
    };
    let padlock = if object.locked { "  locked" } else { "" };
    let turn = if object.rotation == 0.0 {
        String::new()
    } else {
        format!("  {}°", (object.rotation * 100.0).round() / 100.0)
    };
    let text = shorten(&one_line(&object.text));
    let quoted = if text.is_empty() {
        String::new()
    } else {
        format!("  \"{text}\"")
    };
    format!(
        "@{} {:<7} {corner}{size}{inside}{color}{look}{style}{ends}{padlock}{turn}{tags}{quoted}  {}",
        short(&object.id),
        object.kind.short_name(),
        listed.link
    )
}

fn shorten(text: &str) -> String {
    text.char_indices().nth(TEXT_MAX_CHARS).map_or_else(
        || text.to_owned(),
        |(cut, _)| format!("{}…", text.get(..cut).unwrap_or(text)),
    )
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
                short(&object.object),
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
            ends: None,
            author: None,
            locked: false,
            rotation: 0.0,
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
            style: TextStyle::default(),
        }
    }

    fn listing(objects: Vec<ListedObject>, next: Option<usize>) -> Listing {
        Listing {
            id: "b1".into(),
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
                 @a1b2c3d4 sticker (100, 50) 192×192  yellow4  \"{text}\"  L/a1b2c3d4e5f6\n\
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
            Some("@a1 arrow   (100, 50)  s1111111 → ·  L/a1")
        );
        assert_eq!(
            lines.get(2).map(String::as_str),
            Some("@s2 sticker (100, 50) 192×192 in @f1111111  \"hi\"  L/s2")
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
