//! A code block: its language and theme rewritten.

use super::Written;
use crate::{
    domain::{
        board::ObjectKind,
        kinds::code::{LANGUAGE, Language, THEME, Theme},
    },
    error::{Error, Result, board_change::BoardChangeError},
};

pub(super) const CODE: &str = "code";

pub(super) fn check(kind: &ObjectKind) -> Result<()> {
    (*kind == ObjectKind::Code).then_some(()).ok_or_else(|| {
        Error::BoardChange(BoardChangeError::NotChangeable {
            kind: kind.as_str().to_owned(),
            field: CODE,
        })
    })
}

/// The language and the theme given, as the add writes them.
pub(super) fn write(language: Option<Language>, theme: Option<Theme>) -> Written {
    Written {
        fields: language
            .map(|language| (LANGUAGE, language.as_ref().into()))
            .into_iter()
            .chain(theme.map(|theme| (THEME, theme.as_ref().into())))
            .collect(),
        touched: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert is the point of a test")]

    use serde_json::{Value, json};
    use yrs::{Doc, Map as _, ReadTxn as _, Transact as _, types::ToJson as _};

    use crate::{
        domain::{
            board,
            change::{
                kind_edit::KindEdit,
                tests::built_with_ids,
                value::{BoardChange, NewEntry, NewObject, ObjectEdit, ObjectPatch},
            },
            geometry::Point,
            id_source::SequentialIds,
            kinds::code::{self, Language, Theme},
            rich_text::TextFormat,
        },
        error::{Error, Result, board_change::BoardChangeError},
    };

    fn board_with(new: NewObject) -> Result<Doc> {
        let document = Doc::new();
        let add = BoardChange::AddAll(vec![NewEntry {
            new,
            frame: None,
            format: TextFormat::Plain,
        }]);
        built_with_ids(&document, &SequentialIds::new("s"), &add)?;
        Ok(document)
    }

    fn object_id(document: &Doc) -> String {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.keys(&transaction).next().map(str::to_owned))
            .unwrap_or_default()
    }

    fn edit(document: &Doc, kind_edit: KindEdit) -> Result<()> {
        let change = BoardChange::EditAll(vec![ObjectEdit {
            object: object_id(document),
            patch: ObjectPatch {
                kind_edits: vec![kind_edit],
                ..ObjectPatch::default()
            },
            stacking: None,
            tags: None,
        }]);
        built_with_ids(document, &SequentialIds::new("r"), &change).map(drop)
    }

    fn stored(document: &Doc) -> Value {
        let transaction = document.transact();
        transaction
            .get_map("objects")
            .and_then(|objects| objects.get(&transaction, &object_id(document)))
            .and_then(|value| board::json(&value.to_json(&transaction)))
            .unwrap_or_default()
    }

    fn a_block() -> Result<Doc> {
        let mut made = code::made("let a = 1;", None, Theme::Light);
        made.at = Point::new(10.0, 20.0)?;
        board_with(NewObject::Made(made))
    }

    fn words(document: &Doc) -> Vec<Value> {
        let fields = stored(document);
        ["lang", "theme"]
            .into_iter()
            .map(|name| fields.get(name).cloned().unwrap_or_default())
            .collect()
    }

    #[test]
    fn the_language_and_the_theme_are_written() -> Result<()> {
        let document = a_block()?;
        edit(
            &document,
            KindEdit::Code {
                language: Some(Language::Python),
                theme: Some(Theme::Dark),
            },
        )?;
        assert_eq!(words(&document), vec![json!("python"), json!("dark")]);
        Ok(())
    }

    #[test]
    fn a_word_left_out_stays() -> Result<()> {
        let document = a_block()?;
        edit(
            &document,
            KindEdit::Code {
                language: None,
                theme: Some(Theme::Dark),
            },
        )?;
        assert_eq!(words(&document), vec![Value::Null, json!("dark")]);
        Ok(())
    }

    #[test]
    fn a_sticker_takes_neither() -> Result<()> {
        let document = board_with(NewObject::Sticker {
            text: "note".into(),
            at: Point::new(1.0, 1.0)?,
            size: None,
            color: None,
        })?;
        let refused = edit(
            &document,
            KindEdit::Code {
                language: Some(Language::Go),
                theme: None,
            },
        )
        .err();
        assert!(matches!(
            refused,
            Some(Error::BoardChange(BoardChangeError::NotChangeable {
                field: "code",
                ..
            }))
        ));
        Ok(())
    }
}
