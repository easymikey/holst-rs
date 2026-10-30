//! A code block: its code in a root `Y.Text`, a language and a theme.

use itertools::Itertools as _;
use serde::Deserialize;
use serde_json::{Value, json};
use yrs::{Doc, GetString as _, Transact as _};

use super::Detail;
use crate::{
    domain::{
        board::{Object, ObjectKind},
        change::value::Made,
        geometry::{Point, Size},
    },
    protocol::document::{DOCUMENT_ID, SCALE, WIDTH},
};

/// The languages the web client's code block offers, as it stores them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, strum::VariantNames, strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Language {
    Typescript,
    Javascript,
    Python,
    Cpp,
    Java,
    Go,
    Rust,
    PlainText,
    Swift,
    Ruby,
    Sql,
    Html,
    Css,
    Json,
    Kotlin,
    Bash,
    Php,
}

/// The two themes of a code block.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Deserialize,
    strum::VariantNames,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

/// Wide at scale 1, as the web client places a block.
const WIDTH_OF: f64 = 500.0;
pub(crate) const LANGUAGE: &str = "lang";
pub(crate) const THEME: &str = "theme";

/// Code block of `text` as the web's paste leaves it; corner set by caller.
#[must_use]
pub fn made(text: &str, language: Option<Language>, theme: Theme) -> Made {
    let code = dedent(text);
    let lines = code.split('\n').count();
    Made {
        kind: ObjectKind::Code,
        at: Point::ORIGIN,
        size: Size::new(
            WIDTH_OF,
            f64::from(u32::try_from(lines).unwrap_or(u32::MAX)) * 21.0 + 24.0,
        )
        .ok(),
        parent: None,
        fields: [
            (WIDTH, json!(WIDTH_OF)),
            (SCALE, json!(1)),
            (THEME, json!(theme.as_ref())),
        ]
        .into_iter()
        .chain(language.map(|language| (LANGUAGE, json!(language.as_ref()))))
        .collect(),
        texts: Vec::new(),
        code: Some(code),
        parts: Vec::new(),
        entries: Vec::new(),
    }
}

/// `text` with Unix breaks, blank edge lines and leading tabs or spaces
/// removed.
fn dedent(text: &str) -> String {
    let unix = text.replace("\r\n", "\n");
    let lines: Vec<&str> = unix.split('\n').collect();
    let blank = |line: &&str| line.trim().is_empty();
    let body = lines
        .iter()
        .position(|line| !blank(line))
        .zip(lines.iter().rposition(|line| !blank(line)))
        .and_then(|(first, last)| lines.get(first..=last))
        .unwrap_or_default();
    let common = |mark: char| {
        body.iter()
            .filter(|line| !blank(line))
            .map(|line| line.chars().take_while(|found| *found == mark).count())
            .min()
            .unwrap_or(0)
    };
    let margin = format!("{}{}", "\t".repeat(common('\t')), " ".repeat(common(' ')));
    body.iter()
        .map(|line| line.strip_prefix(margin.as_str()).unwrap_or(line))
        .join("\n")
}

/// The code from its root text, the language and the theme as words.
pub(super) fn detail(document: &Doc, block: &Object) -> Detail {
    let text = block
        .fields
        .get(DOCUMENT_ID)
        .and_then(Value::as_str)
        .map(|id| {
            document
                .get_or_insert_text(format!("code-mirror-{id}"))
                .get_string(&document.transact())
        });
    Detail {
        text,
        words: [("language", LANGUAGE), ("theme", THEME)]
            .into_iter()
            .filter_map(|(word, key)| {
                block.fields.get(key).map(|stored| (word, stored.clone()))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::json;
    use yrs::{Doc, Text as _, Transact as _};

    use super::{Language, Theme, made};
    use crate::{
        Result,
        domain::{
            board::{ObjectKind, object_of},
            geometry::Size,
            kinds::detail,
        },
    };

    #[test]
    fn a_made_code_has_the_keys_of_the_web_client() -> Result<()> {
        let plain = made("fn main() {}", None, Theme::Light);
        let marked = made("fn main() {}", Some(Language::Rust), Theme::Dark);
        assert_eq!(
            (plain.kind, plain.size, plain.fields, plain.texts),
            (
                ObjectKind::Code,
                Some(Size::new(500.0, 45.0)?),
                vec![
                    ("width", json!(500.0)),
                    ("scale", json!(1)),
                    ("theme", json!("light")),
                ],
                Vec::new()
            )
        );
        assert_eq!(
            marked.fields,
            vec![
                ("width", json!(500.0)),
                ("scale", json!(1)),
                ("theme", json!("dark")),
                ("lang", json!("rust")),
            ]
        );
        Ok(())
    }

    #[test]
    fn the_code_is_stored_with_unix_breaks_and_the_common_indent_cut() {
        let pasted = made(
            "\r\n\t\tif a {\r\n\t\t\tb();\r\n\r\n\t\t}\r\n\r\n",
            None,
            Theme::Light,
        );
        let spaced = made("    a\n      b\n", None, Theme::Light);
        assert_eq!(
            (pasted.code, spaced.code),
            (
                Some("if a {\n\tb();\n\n}".to_owned()),
                Some("a\n  b".to_owned())
            )
        );
    }

    #[test]
    fn a_code_lists_its_root_text_language_and_theme() {
        let document = Doc::new();
        document.get_or_insert_text("code-mirror-d1").insert(
            &mut document.transact_mut(),
            0,
            "fn main() {}",
        );
        let block = object_of(json!({
            "id": "k1", "type": "code", "documentId": "d1",
            "lang": "rust", "theme": "light", "position": {"x": 0, "y": 0}
        }));
        let shown = block.iter().map(|object| detail(&document, object));
        assert_eq!(
            shown
                .map(|found| (found.text, found.words))
                .collect::<Vec<_>>(),
            vec![(
                Some("fn main() {}".to_owned()),
                vec![("language", json!("rust")), ("theme", json!("light"))]
            )]
        );
    }
}
